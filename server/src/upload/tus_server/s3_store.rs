//! Port of `@tus/s3-store` 2.0.4 (`S3Store`), the datastore lib/upload/tus.ts
//! `makeDatastore()` builds when `STORAGE_DRIVER=s3`.
//!
//! Implementation (based on tusd's s3store, as upstream says): creating an
//! upload stores an `<id>.info` object whose body is the JSON of the `Upload`
//! (and whose user metadata carries `upload-id` and `tus-version`) and starts
//! a multipart upload of the object `<id>`. Every PATCH body is cut into
//! chunks of `calcOptimalPartSize` bytes on local disk; each chunk becomes the
//! next part, except a chunk under the 5 MiB S3 minimum that does not finish
//! the upload, which is parked as the object `<id>.part` and prepended to the
//! next PATCH. When the offset reaches the size, the multipart upload is
//! completed. The info object is not deleted.
//!
//! Keys, bodies, user metadata and tags are the TypeScript store's, so an
//! upload started on the reference server resumes here against the same
//! bucket and the other way round. Like upstream, the store has no key prefix
//! (`S3_PREFIX` does not apply to it) and keeps a per-process metadata cache.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::bail;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream as SdkStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart, Delete, ObjectIdentifier, Part};
use base64::Engine;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use rand::RngCore;
use tokio::io::AsyncWriteExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::store::{
    DataStore, EXT_CREATION, EXT_CREATION_DEFER_LENGTH, EXT_CREATION_WITH_UPLOAD, EXT_EXPIRATION, EXT_TERMINATION,
    Metadata, StoreError, TusError, Upload, UploadStorage, errors,
};
use crate::db::types::JsDate;
use crate::storage::ByteStream;
use crate::storage::s3::{S3Config, s3_client, s3_error, sdk_err};

/// `TUS_RESUMABLE` of `@tus/utils`.
pub const TUS_RESUMABLE: &str = "1.0.0";

/// lib/upload/tus.ts `makeDatastore()`: `partSize: 64 * 1024 * 1024 // >=5MiB, <=5GiB`.
pub const MODLIGHT_PART_SIZE: u64 = 64 * 1024 * 1024;

const EXTENSIONS: &[&str] =
    &[EXT_CREATION, EXT_CREATION_WITH_UPLOAD, EXT_CREATION_DEFER_LENGTH, EXT_TERMINATION, EXT_EXPIRATION];

/// `Options` of `@tus/s3-store`.
#[derive(Debug, Clone)]
pub struct S3StoreOptions {
    /// The preferred part size; the store may raise it to stay under
    /// `max_multipart_parts`. Default 8 MiB.
    pub part_size: Option<u64>,
    /// Default 5 MiB.
    pub min_part_size: Option<u64>,
    /// Default 10,000.
    pub max_multipart_parts: Option<u64>,
    /// Tag objects `Tus-Completed=true|false` when uploads expire. Default true.
    pub use_tags: Option<bool>,
    /// Default 60.
    pub max_concurrent_part_uploads: Option<usize>,
    /// Default 0 (never expire).
    pub expiration_period_in_milliseconds: Option<u64>,
    /// `s3ClientConfig`: the bucket and the client settings (`prefix` is not
    /// used: the store writes at the bucket root).
    pub s3_client_config: S3Config,
    /// Where chunks are staged (`os.tmpdir()`); not an upstream option.
    pub tmp_dir: Option<PathBuf>,
}

impl S3StoreOptions {
    pub fn new(s3_client_config: S3Config) -> S3StoreOptions {
        S3StoreOptions {
            part_size: None,
            min_part_size: None,
            max_multipart_parts: None,
            use_tags: None,
            max_concurrent_part_uploads: None,
            expiration_period_in_milliseconds: None,
            s3_client_config,
            tmp_dir: None,
        }
    }
}

/// `MetadataValue`: what the info object holds.
#[derive(Debug, Clone)]
pub struct MetadataValue {
    pub file: Upload,
    /// The multipart upload id (`'upload-id'`).
    pub upload_id: Option<String>,
    pub tus_version: Option<String>,
}

pub struct S3Store {
    bucket: String,
    /// `MemoryKvStore<MetadataValue>`
    cache: Mutex<HashMap<String, MetadataValue>>,
    client: Client,
    preferred_part_size: u64,
    expiration_period_in_milliseconds: u64,
    use_tags: bool,
    part_upload_semaphore: Arc<Semaphore>,
    pub max_multipart_parts: u64,
    pub min_part_size: u64,
    /// 5 TiB
    pub max_upload_size: u64,
    tmp_dir: PathBuf,
}

fn calc_offset_from_parts(parts: &[Part]) -> u64 {
    parts.iter().map(|p| p.size().unwrap_or(0).max(0) as u64).sum()
}

/// `crypto.randomBytes(size).toString('base64url').slice(0, size)`
fn random_string(size: usize) -> String {
    let mut bytes = vec![0u8; size];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut s = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    s.truncate(size);
    s
}

/// `Number.parseInt(String(v), 10)` for the numbers of an info object.
fn json_int(v: Option<&serde_json::Value>) -> Option<u64> {
    match v? {
        serde_json::Value::Number(n) => {
            n.as_u64().or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64))
        }
        serde_json::Value::String(s) => {
            let digits: String = s.trim_start().chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        }
        _ => None,
    }
}

/// `new Upload({ id, size, offset, metadata, creation_date, storage })` from
/// the parsed info object, as `getMetadata` builds it.
fn upload_from_info(id: &str, file: &serde_json::Value) -> Upload {
    // `Number.isFinite(file.size) ? Number.parseInt(file.size, 10) : undefined`
    let size = match file.get("size") {
        Some(v @ serde_json::Value::Number(_)) => json_int(Some(v)),
        _ => None,
    };
    let metadata = file.get("metadata").and_then(|m| m.as_object()).map(|m| {
        m.iter()
            .map(|(k, v)| {
                let value = match v {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => Some(s.clone()),
                    other => Some(other.to_string()),
                };
                (k.clone(), value)
            })
            .collect::<Metadata>()
    });
    Upload {
        id: id.to_string(),
        metadata,
        size,
        offset: json_int(file.get("offset")).unwrap_or(0),
        // `upload.creation_date ?? new Date().toISOString()`
        creation_date: Some(match file.get("creation_date") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Null) | None => JsDate::now().to_iso_string(),
            Some(other) => other.to_string(),
        }),
        storage: file.get("storage").and_then(|s| serde_json::from_value::<UploadStorage>(s.clone()).ok()),
    }
}

/// A local temp file removed when dropped.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `downloadIncompletePart()`: the parked tail of the previous PATCH, on disk.
struct IncompletePart {
    size: u64,
    file: TempFile,
}

impl IncompletePart {
    /// `createReader({ cleanUpOnEnd: true })`: the file is removed once the
    /// stream is done with.
    async fn into_reader(self) -> std::io::Result<ByteStream> {
        let reader = tokio::fs::File::open(&self.file.0).await?;
        let guard = self.file;
        Ok(Box::pin(tokio_util::io::ReaderStream::new(reader).map(move |r| {
            let _keep = &guard;
            r
        })))
    }
}

/// The S3 calls a part upload needs, cloned into each spawned upload.
#[derive(Clone)]
struct PartUploader {
    client: Client,
    bucket: String,
    id: String,
    upload_id: Option<String>,
    incomplete_tagging: Option<String>,
}

impl PartUploader {
    /// `uploadPart(metadata, readStream, partNumber)`: the ETag.
    async fn upload_part(&self, body: SdkStream, part_number: i32) -> anyhow::Result<String> {
        let data = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(&self.id)
            .set_upload_id(self.upload_id.clone())
            .part_number(part_number)
            .body(body)
            .send()
            .await
            .map_err(sdk_err)?;
        tracing::debug!("[{}] finished uploading part #{part_number}", self.id);
        Ok(data.e_tag().unwrap_or_default().to_string())
    }

    /// `uploadIncompletePart(id, readStream)`
    async fn upload_incomplete_part(&self, body: SdkStream) -> anyhow::Result<String> {
        let data = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(format!("{}.part", self.id))
            .body(body)
            .set_tagging(self.incomplete_tagging.clone())
            .send()
            .await
            .map_err(sdk_err)?;
        tracing::debug!("[{}] finished uploading incomplete part", self.id);
        Ok(data.e_tag().unwrap_or_default().to_string())
    }
}

/// A chunk being written by the splitter (`fileHandle`, `currentChunkPath`).
/// The file is removed when the chunk is dropped, also when `write` itself
/// is dropped midway.
struct Chunk {
    temp: TempFile,
    file: tokio::fs::File,
    size: u64,
    permit: OwnedSemaphorePermit,
}

/// What the spawned part uploads report back.
struct PartsState {
    bytes_uploaded: AtomicU64,
    /// `splitterStream.destroy(error)`: a part failed, stop splitting.
    failed: AtomicBool,
}

/// `StreamSplitter` of `@tus/utils` with `uploadParts`' event handlers: cuts
/// the body into `chunk_size` files and uploads each finished one.
struct Splitter<'a> {
    store: &'a S3Store,
    uploader: PartUploader,
    chunk_size: u64,
    filename_template: String,
    part: u64,
    current: Option<Chunk>,
    size: Option<u64>,
    current_part_number: i32,
    offset: u64,
    tasks: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
    state: Arc<PartsState>,
}

impl Splitter<'_> {
    /// `_newChunk()`: a permit first (`beforeChunkStarted`), then the file.
    async fn new_chunk(&mut self) -> anyhow::Result<()> {
        let permit = self.store.part_upload_semaphore.clone().acquire_owned().await?;
        let path = self.store.tmp_dir.join(format!("{}-{}", self.filename_template, self.part));
        let file = tokio::fs::File::create(&path).await?;
        self.current = Some(Chunk { temp: TempFile(path), file, size: 0, permit });
        Ok(())
    }

    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()> {
        let chunk = self.current.as_mut().expect("a chunk is open");
        chunk.file.write_all(data).await?;
        chunk.size += data.len() as u64;
        Ok(())
    }

    /// `_write(chunk)`
    async fn write(&mut self, mut data: Bytes) -> anyhow::Result<()> {
        if self.current.is_none() {
            self.new_chunk().await?;
        }
        loop {
            let current_size = self.current.as_ref().map_or(0, |c| c.size);
            let overflow = (current_size + data.len() as u64).saturating_sub(self.chunk_size);
            if overflow == 0 {
                break;
            }
            let take = data.len() - overflow as usize;
            self.write_chunk(&data[..take]).await?;
            self.finish_chunk().await?;
            self.new_chunk().await?;
            data = data.slice(take..);
        }
        self.write_chunk(&data).await
    }

    /// `_final()`
    async fn finish(&mut self) -> anyhow::Result<()> {
        if self.current.is_some() { self.finish_chunk().await } else { Ok(()) }
    }

    /// `_handleError()` plus `uploadParts`' cleanup of the pending chunk
    /// (the file goes with the chunk).
    fn abandon(&mut self) {
        self.current = None;
    }

    /// `_finishChunk()` and the `chunkFinished` handler of `uploadParts`.
    async fn finish_chunk(&mut self) -> anyhow::Result<()> {
        let Some(mut chunk) = self.current.take() else { return Ok(()) };
        chunk.file.flush().await?;
        drop(chunk.file);
        self.part += 1;

        let part_size = chunk.size;
        let part_number = self.current_part_number;
        self.current_part_number += 1;
        self.offset += part_size;
        let is_final_part = self.size == Some(self.offset);
        let as_part = part_size >= self.store.min_part_size || is_final_part;

        let uploader = self.uploader.clone();
        let state = self.state.clone();
        let (temp, permit) = (chunk.temp, chunk.permit);
        self.tasks.push(tokio::spawn(async move {
            let result = async {
                let body = SdkStream::from_path(&temp.0).await?;
                if as_part {
                    uploader.upload_part(body, part_number).await?;
                } else {
                    uploader.upload_incomplete_part(body).await?;
                }
                anyhow::Ok(())
            }
            .await;
            drop(temp);
            drop(permit);
            match result {
                Ok(()) => {
                    state.bytes_uploaded.fetch_add(part_size, Ordering::SeqCst);
                    Ok(())
                }
                Err(err) => {
                    state.failed.store(true, Ordering::SeqCst);
                    Err(err)
                }
            }
        }));
        Ok(())
    }
}

/// A body error as `@tus/server` reports it: a tus refusal raised by the
/// server's own stream wrapper (the size limiter) keeps its status.
fn stream_error(err: std::io::Error) -> StoreError {
    if let Some(tus) = err.get_ref().and_then(|e| e.downcast_ref::<TusError>()) {
        return StoreError::Tus(tus.clone());
    }
    StoreError::Other(err.into())
}

impl S3Store {
    pub fn new(options: S3StoreOptions) -> S3Store {
        S3Store {
            bucket: options.s3_client_config.bucket.clone(),
            cache: Mutex::new(HashMap::new()),
            client: s3_client(&options.s3_client_config),
            preferred_part_size: options.part_size.filter(|v| *v != 0).unwrap_or(8 * 1024 * 1024),
            expiration_period_in_milliseconds: options.expiration_period_in_milliseconds.unwrap_or(0),
            use_tags: options.use_tags.unwrap_or(true),
            part_upload_semaphore: Arc::new(Semaphore::new(options.max_concurrent_part_uploads.unwrap_or(60))),
            max_multipart_parts: options.max_multipart_parts.filter(|v| *v != 0).unwrap_or(10_000),
            min_part_size: options.min_part_size.filter(|v| *v != 0).unwrap_or(5_242_880),
            max_upload_size: 5_497_558_138_880,
            tmp_dir: options.tmp_dir.unwrap_or_else(std::env::temp_dir),
        }
    }

    /// lib/upload/tus.ts `makeDatastore()` for `STORAGE_DRIVER=s3`: 64 MiB
    /// parts, unfinished uploads expire after `TUS_EXPIRATION_MS` (24 h), the
    /// client from the S3_* variables (as lib/storage/index.ts reads them,
    /// with its refusal when one is missing).
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<S3Store> {
        let config = S3Config::from_env(var)?;
        let mut options = S3StoreOptions::new(config);
        options.part_size = Some(MODLIGHT_PART_SIZE);
        options.expiration_period_in_milliseconds = Some(crate::upload::tus::TUS_EXPIRATION.as_millis() as u64);
        Ok(S3Store::new(options))
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    fn should_use_expiration_tags(&self) -> bool {
        self.expiration_period_in_milliseconds != 0 && self.use_tags
    }

    fn use_complete_tag(&self, value: &str) -> Option<String> {
        self.should_use_expiration_tags().then(|| format!("Tus-Completed={value}"))
    }

    /// `infoKey(id)`
    pub fn info_key(&self, id: &str) -> String {
        format!("{id}.info")
    }

    /// `partKey(id, isIncomplete)`
    pub fn part_key(&self, id: &str, is_incomplete: bool) -> String {
        if is_incomplete { format!("{id}.part") } else { id.to_string() }
    }

    fn cache_get(&self, id: &str) -> Option<MetadataValue> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    fn cache_set(&self, id: &str, value: MetadataValue) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), value);
    }

    /// `clearCache(id)`
    fn clear_cache(&self, id: &str) {
        tracing::debug!("[{id}] removing cached data");
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    fn uploader(&self, metadata: &MetadataValue) -> PartUploader {
        PartUploader {
            client: self.client.clone(),
            bucket: self.bucket.clone(),
            id: metadata.file.id.clone(),
            upload_id: metadata.upload_id.clone(),
            incomplete_tagging: self.use_complete_tag("false"),
        }
    }

    /// `saveMetadata(upload, uploadId)`: the info object, its body the
    /// upload's JSON.
    async fn save_metadata(&self, upload: &Upload, upload_id: Option<&str>) -> anyhow::Result<()> {
        tracing::debug!("[{}] saving metadata", upload.id);
        self.put_info(upload, upload_id, "false").await?;
        tracing::debug!("[{}] metadata file saved", upload.id);
        Ok(())
    }

    async fn put_info(&self, upload: &Upload, upload_id: Option<&str>, completed: &str) -> anyhow::Result<()> {
        let mut req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(self.info_key(&upload.id))
            .body(SdkStream::from(serde_json::to_vec(upload)?))
            .set_tagging(self.use_complete_tag(completed));
        if let Some(upload_id) = upload_id {
            req = req.metadata("upload-id", upload_id);
        }
        req.metadata("tus-version", TUS_RESUMABLE).send().await.map_err(sdk_err)?;
        Ok(())
    }

    /// `completeMetadata(upload)`
    async fn complete_metadata(&self, upload: &Upload) -> anyhow::Result<()> {
        if !self.should_use_expiration_tags() {
            return Ok(());
        }
        let MetadataValue { upload_id, .. } = self.get_metadata(&upload.id).await?;
        self.put_info(upload, upload_id.as_deref(), "true").await
    }

    /// `getMetadata(id)`: the info object, cached for the process.
    pub async fn get_metadata(&self, id: &str) -> anyhow::Result<MetadataValue> {
        if let Some(cached) = self.cache_get(id) {
            return Ok(cached);
        }
        let res = self.client.get_object().bucket(&self.bucket).key(self.info_key(id)).send().await.map_err(sdk_err)?;
        let user_metadata = res.metadata().cloned().unwrap_or_default();
        let body = res.body.collect().await.map_err(std::io::Error::other)?.into_bytes();
        let file: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&body))?;
        let value = MetadataValue {
            tus_version: user_metadata.get("tus-version").cloned(),
            upload_id: user_metadata.get("upload-id").cloned(),
            file: upload_from_info(id, &file),
        };
        self.cache_set(id, value.clone());
        Ok(value)
    }

    /// `getIncompletePart(id)`: `None` when there is none (NoSuchKey).
    async fn get_incomplete_part(&self, id: &str) -> anyhow::Result<Option<SdkStream>> {
        match self.client.get_object().bucket(&self.bucket).key(self.part_key(id, true)).send().await {
            Ok(data) => Ok(Some(data.body)),
            Err(err) => {
                let err = sdk_err(err);
                if s3_error(&err).is_some_and(|e| e.has_code("NoSuchKey")) { Ok(None) } else { Err(err) }
            }
        }
    }

    /// `getIncompletePartSize(id)`: `None` when there is none (NotFound).
    async fn get_incomplete_part_size(&self, id: &str) -> anyhow::Result<Option<u64>> {
        match self.client.head_object().bucket(&self.bucket).key(self.part_key(id, true)).send().await {
            Ok(data) => Ok(data.content_length().map(|n| n.max(0) as u64)),
            Err(err) => {
                let err = sdk_err(err);
                if s3_error(&err).is_some_and(|e| e.has_code("NotFound")) { Ok(None) } else { Err(err) }
            }
        }
    }

    /// `deleteIncompletePart(id)`
    async fn delete_incomplete_part(&self, id: &str) -> anyhow::Result<()> {
        self.client.delete_object().bucket(&self.bucket).key(self.part_key(id, true)).send().await.map_err(sdk_err)?;
        Ok(())
    }

    /// `downloadIncompletePart(id)`: the parked part copied to a temp file.
    async fn download_incomplete_part(&self, id: &str) -> anyhow::Result<Option<IncompletePart>> {
        let Some(mut body) = self.get_incomplete_part(id).await? else {
            return Ok(None);
        };
        let file = TempFile(self.unique_tmp_file_name("tus-s3-incomplete-part-").await?);
        let mut out = tokio::fs::File::create(&file.0).await?;
        let mut size = 0u64;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(std::io::Error::other)?;
            size += chunk.len() as u64;
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(Some(IncompletePart { size, file }))
    }

    /// `uploadParts(metadata, readStream, currentPartNumber, offset)`: the
    /// bytes uploaded (parts and parked tail).
    async fn upload_parts(
        &self,
        metadata: &MetadataValue,
        mut src: ByteStream,
        current_part_number: i32,
        offset: u64,
    ) -> Result<u64, StoreError> {
        let state = Arc::new(PartsState { bytes_uploaded: AtomicU64::new(0), failed: AtomicBool::new(false) });
        let mut splitter = Splitter {
            store: self,
            uploader: self.uploader(metadata),
            chunk_size: self.calc_optimal_part_size(metadata.file.size),
            filename_template: random_string(10),
            part: 0,
            current: None,
            size: metadata.file.size,
            current_part_number,
            offset,
            tasks: Vec::new(),
            state: state.clone(),
        };

        // `pipeline(readStream, splitterStream)`
        let mut pipeline_error: Option<StoreError> = None;
        loop {
            if state.failed.load(Ordering::SeqCst) {
                break;
            }
            match src.next().await {
                None => {
                    if let Err(err) = splitter.finish().await {
                        pipeline_error = Some(StoreError::Other(err));
                    }
                    break;
                }
                Some(Err(err)) => {
                    pipeline_error = Some(stream_error(err));
                    break;
                }
                Some(Ok(data)) if data.is_empty() => {}
                Some(Ok(data)) => {
                    if let Err(err) = splitter.write(data).await {
                        pipeline_error = Some(StoreError::Other(err));
                        break;
                    }
                }
            }
        }
        // A destroyed splitter (failed part, body error) drops its open chunk.
        splitter.abandon();
        drop(src);

        // Wait for all part uploads, then fail with the first failure, in
        // part order, or the body's error.
        let mut first_failure: Option<StoreError> = None;
        for task in std::mem::take(&mut splitter.tasks) {
            let outcome = match task.await {
                Ok(outcome) => outcome,
                Err(join) => Err(anyhow::anyhow!("part upload task failed: {join}")),
            };
            if let Err(err) = outcome
                && first_failure.is_none()
            {
                first_failure = Some(StoreError::Other(err));
            }
        }
        if let Some(err) = first_failure.or(pipeline_error) {
            return Err(err);
        }
        Ok(state.bytes_uploaded.load(Ordering::SeqCst))
    }

    /// `finishMultipartUpload(metadata, parts)`: the object's `Location`.
    async fn finish_multipart_upload(
        &self,
        metadata: &MetadataValue,
        parts: Vec<Part>,
    ) -> anyhow::Result<Option<String>> {
        let mut completed: Vec<CompletedPart> = parts
            .iter()
            .map(|p| {
                CompletedPart::builder()
                    .set_e_tag(p.e_tag().map(str::to_string))
                    .set_part_number(p.part_number())
                    .build()
            })
            .collect();
        // Handle zero-byte uploads - S3 requires at least one part to complete a multipart upload
        // S3 allows the last part to be 0 bytes, so we upload a single empty part
        if completed.is_empty() {
            let etag = self.uploader(metadata).upload_part(SdkStream::from_static(b""), 1).await?;
            completed.push(CompletedPart::builder().e_tag(etag).part_number(1).build());
        }
        let response = self
            .client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&metadata.file.id)
            .set_upload_id(metadata.upload_id.clone())
            .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(completed)).build())
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(response.location().map(str::to_string))
    }

    /// `retrieveParts(id)`: every uploaded part, sorted by number.
    async fn retrieve_parts(&self, id: &str) -> anyhow::Result<Vec<Part>> {
        let metadata = self.get_metadata(id).await?;
        let mut parts = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let data = self
                .client
                .list_parts()
                .bucket(&self.bucket)
                .key(id)
                .set_upload_id(metadata.upload_id.clone())
                .set_part_number_marker(marker.take())
                .send()
                .await
                .map_err(sdk_err)?;
            parts.extend(data.parts().iter().cloned());
            if data.is_truncated() != Some(true) {
                break;
            }
            marker = data.next_part_number_marker().map(str::to_string);
            if marker.is_none() {
                break;
            }
        }
        parts.sort_by_key(|p| p.part_number().unwrap_or(0));
        Ok(parts)
    }

    /// `calcOptimalPartSize(size)`
    pub fn calc_optimal_part_size(&self, size: Option<u64>) -> u64 {
        // When upload size is not know we assume largest possible value (`maxUploadSize`)
        let size = size.unwrap_or(self.max_upload_size);
        let optimal_part_size = if size <= self.preferred_part_size {
            // When upload is smaller or equal to PreferredPartSize, we upload in just one part.
            size
        } else if size <= self.preferred_part_size * self.max_multipart_parts {
            // Does the upload fit in MaxMultipartParts parts or less with PreferredPartSize.
            self.preferred_part_size
        } else {
            // The upload is too big for the preferred size.
            // We devide the size with the max amount of parts and round it up.
            size.div_ceil(self.max_multipart_parts)
        };
        // Always ensure the part size is at least minPartSize
        optimal_part_size.max(self.min_part_size)
    }

    /// `read(id)`: the finished object (what `@tus/server`'s GET handler
    /// streams). Not part of [`DataStore`].
    pub async fn read(&self, id: &str) -> anyhow::Result<ByteStream> {
        let data = self.client.get_object().bucket(&self.bucket).key(id).send().await.map_err(sdk_err)?;
        Ok(crate::storage::s3::into_byte_stream(data.body))
    }

    /// `uniqueTmpFileName(template)`
    async fn unique_tmp_file_name(&self, template: &str) -> anyhow::Result<PathBuf> {
        const MAX_TRIES: u32 = 10;
        for _ in 0..MAX_TRIES {
            let path = self.tmp_dir.join(format!("{template}{}", random_string(10)));
            match tokio::fs::symlink_metadata(&path).await {
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(path),
                Err(e) => return Err(e.into()),
            }
        }
        bail!("Could not find a unique file name after {MAX_TRIES} tries")
    }

    /// `getExpirationDate(created_at)` as epoch milliseconds.
    fn expiration_millis(&self, created_at_millis: i64) -> i64 {
        created_at_millis.saturating_add(self.expiration_period_in_milliseconds as i64)
    }
}

impl DataStore for S3Store {
    fn extensions(&self) -> &[&'static str] {
        EXTENSIONS
    }

    /// `create(upload)`: start the multipart upload and write the info object.
    fn create(&self, mut upload: Upload) -> BoxFuture<'_, Result<Upload, StoreError>> {
        Box::pin(async move {
            tracing::debug!("[{}] initializing multipart upload", upload.id);
            let mut request = self
                .client
                .create_multipart_upload()
                .bucket(&self.bucket)
                .key(&upload.id)
                .metadata("tus-version", TUS_RESUMABLE);
            if let Some(content_type) = upload.meta("contentType").filter(|v| !v.is_empty()) {
                request = request.content_type(content_type);
            }
            if let Some(cache_control) = upload.meta("cacheControl").filter(|v| !v.is_empty()) {
                request = request.cache_control(cache_control);
            }
            upload.creation_date = Some(JsDate::now().to_iso_string());
            let res = request.send().await.map_err(sdk_err)?;
            upload.storage = Some(UploadStorage {
                kind: "s3".to_string(),
                path: res.key().unwrap_or(&upload.id).to_string(),
                bucket: Some(self.bucket.clone()),
            });
            self.save_metadata(&upload, res.upload_id()).await?;
            tracing::debug!("[{}] multipart upload created ({})", upload.id, res.upload_id().unwrap_or_default());
            Ok(upload)
        })
    }

    /// `remove(id)`: abort the multipart upload, delete the object and the
    /// info object. (Upstream's "not found" branch tests `error.code`, which
    /// S3 errors never have, so every failure surfaces as it is.)
    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let MetadataValue { upload_id, .. } = self.get_metadata(id).await?;
            if let Some(upload_id) = upload_id.filter(|u| !u.is_empty()) {
                self.client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(id)
                    .upload_id(upload_id)
                    .send()
                    .await
                    .map_err(sdk_err)?;
            }
            let objects = [id.to_string(), self.info_key(id)]
                .into_iter()
                .map(|key| ObjectIdentifier::builder().key(key).build())
                .collect::<Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)?;
            let delete = Delete::builder().set_objects(Some(objects)).build().map_err(anyhow::Error::from)?;
            self.client.delete_objects().bucket(&self.bucket).delete(delete).send().await.map_err(sdk_err)?;
            self.clear_cache(id);
            Ok(())
        })
    }

    /// `write(src, id, offset)`: the new offset.
    fn write<'a>(&'a self, body: ByteStream, id: &'a str, offset: u64) -> BoxFuture<'a, Result<u64, StoreError>> {
        Box::pin(async move {
            // Metadata request needs to happen first
            let metadata = self.get_metadata(id).await?;
            let parts = self.retrieve_parts(id).await?;
            let part_number = parts.last().and_then(|p| p.part_number()).unwrap_or(0);
            let next_part_number = part_number + 1;

            let incomplete_part = self.download_incomplete_part(id).await?;
            let requested_offset = offset;
            let incomplete_size = incomplete_part.as_ref().map_or(0, |p| p.size);
            let (src, offset) = match incomplete_part {
                Some(part) => {
                    // once the file is on disk, we delete the incomplete part
                    self.delete_incomplete_part(id).await?;
                    let head = part.into_reader().await?;
                    let src: ByteStream = Box::pin(head.chain(body));
                    (src, requested_offset.saturating_sub(incomplete_size))
                }
                None => (body, offset),
            };

            let bytes_uploaded = self.upload_parts(&metadata, src, next_part_number, offset).await?;

            // The size of the incomplete part should not be counted, because the
            // process of the incomplete part should be fully transparent to the user.
            let new_offset = (requested_offset + bytes_uploaded).saturating_sub(incomplete_size);

            if metadata.file.size == Some(new_offset) {
                let finished: anyhow::Result<()> = async {
                    let parts = self.retrieve_parts(id).await?;
                    self.finish_multipart_upload(&metadata, parts).await?;
                    self.complete_metadata(&metadata.file).await?;
                    self.clear_cache(id);
                    Ok(())
                }
                .await;
                if let Err(err) = finished {
                    tracing::debug!("[{id}] failed to finish upload: {err:#}");
                    return Err(err.into());
                }
            }
            Ok(new_offset)
        })
    }

    /// `getUpload(id)`: the offset is the sum of the parts plus the parked
    /// tail; a multipart upload that no longer exists was completed.
    fn get_upload<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Upload, StoreError>> {
        Box::pin(async move {
            let metadata = match self.get_metadata(id).await {
                Ok(metadata) => metadata,
                Err(err) => {
                    tracing::debug!("getUpload: No file found. {err:#}");
                    return Err(errors::FILE_NOT_FOUND.into());
                }
            };
            let offset = match self.retrieve_parts(id).await {
                Ok(parts) => calc_offset_from_parts(&parts),
                Err(err) => {
                    // Check if the error is caused by the upload not being found. This happens
                    // when the multipart upload has already been completed or aborted. Since
                    // we already found the info object, we know that the upload has been
                    // completed and therefore can ensure the the offset is the size.
                    // AWS S3 returns NoSuchUpload, but other implementations, such as DigitalOcean
                    // Spaces, can also return NoSuchKey.
                    if s3_error(&err).is_some_and(|e| e.has_code("NoSuchUpload") || e.has_code("NoSuchKey")) {
                        let mut upload = metadata.file.clone();
                        upload.offset = metadata.file.size.unwrap_or(0);
                        return Ok(upload);
                    }
                    return Err(err.into());
                }
            };
            let incomplete_part_size = self.get_incomplete_part_size(id).await?;
            let mut upload = metadata.file;
            upload.offset = offset + incomplete_part_size.unwrap_or(0);
            Ok(upload)
        })
    }

    /// `declareUploadLength(file_id, upload_length)`
    fn declare_upload_length<'a>(&'a self, id: &'a str, upload_length: u64) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut value = self.get_metadata(id).await?;
            value.file.size = Some(upload_length);
            // Upstream mutates the cached object in place.
            self.cache_set(id, value.clone());
            self.save_metadata(&value.file, value.upload_id.as_deref()).await?;
            Ok(())
        })
    }

    /// `deleteExpired()`: abort expired multipart uploads and delete their
    /// info and parked-part objects; the number of objects S3 reports deleted.
    fn delete_expired(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
            if self.get_expiration() == 0 {
                return Ok(0);
            }
            let mut key_marker: Option<String> = None;
            let mut upload_id_marker: Option<String> = None;
            let mut deleted = 0u64;
            loop {
                let list = self
                    .client
                    .list_multipart_uploads()
                    .bucket(&self.bucket)
                    .set_key_marker(key_marker.take())
                    .set_upload_id_marker(upload_id_marker.take())
                    .send()
                    .await
                    .map_err(sdk_err)?;
                let now = chrono::Utc::now().timestamp_millis();
                let expired: Vec<_> = list
                    .uploads()
                    .iter()
                    .filter(|u| {
                        u.initiated()
                            .and_then(|d| d.to_millis().ok())
                            .is_some_and(|initiated| now > self.expiration_millis(initiated))
                    })
                    .collect();
                let objects_to_delete: Vec<String> = expired
                    .iter()
                    .flat_map(|u| {
                        let key = u.key().unwrap_or("undefined");
                        [self.info_key(key), self.part_key(key, true)]
                    })
                    .collect();

                // Batch delete 1000 items at a time
                let deletions = objects_to_delete.chunks(1000).map(|batch| {
                    let objects = batch
                        .iter()
                        .map(|key| ObjectIdentifier::builder().key(key).build())
                        .collect::<Result<Vec<_>, _>>();
                    async move {
                        let delete = Delete::builder().set_objects(Some(objects?)).build()?;
                        let out = self
                            .client
                            .delete_objects()
                            .bucket(&self.bucket)
                            .delete(delete)
                            .send()
                            .await
                            .map_err(sdk_err)?;
                        anyhow::Ok(out.deleted().len() as u64)
                    }
                });
                let aborts = expired.iter().map(|u| async move {
                    self.client
                        .abort_multipart_upload()
                        .bucket(&self.bucket)
                        .set_key(u.key().map(str::to_string))
                        .set_upload_id(u.upload_id().map(str::to_string))
                        .send()
                        .await
                        .map_err(sdk_err)?;
                    anyhow::Ok(())
                });
                let (objects_deleted, aborted) = futures_util::future::join(
                    futures_util::future::join_all(deletions),
                    futures_util::future::join_all(aborts),
                )
                .await;
                deleted += objects_deleted.iter().flatten().sum::<u64>();
                if let Some(err) = objects_deleted.into_iter().filter_map(Result::err).next() {
                    return Err(err.into());
                }
                if let Some(err) = aborted.into_iter().filter_map(Result::err).next() {
                    return Err(err.into());
                }

                if list.is_truncated() != Some(true) {
                    break;
                }
                key_marker = list.next_key_marker().map(str::to_string);
                upload_id_marker = list.next_upload_id_marker().map(str::to_string);
            }
            Ok(deleted)
        })
    }

    fn get_expiration(&self) -> u64 {
        self.expiration_period_in_milliseconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(part_size: Option<u64>) -> S3Store {
        let mut options = S3StoreOptions::new(S3Config {
            endpoint: Some("http://127.0.0.1:1".into()),
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "a".into(),
            secret_access_key: "s".into(),
            force_path_style: true,
            prefix: None,
        });
        options.part_size = part_size;
        S3Store::new(options)
    }

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn optimal_part_size_like_the_typescript_store() {
        let s = store(Some(64 * MIB));
        assert_eq!(s.calc_optimal_part_size(Some(0)), 5 * MIB);
        assert_eq!(s.calc_optimal_part_size(Some(MIB)), 5 * MIB);
        assert_eq!(s.calc_optimal_part_size(Some(10 * MIB)), 10 * MIB);
        assert_eq!(s.calc_optimal_part_size(Some(64 * MIB)), 64 * MIB);
        assert_eq!(s.calc_optimal_part_size(Some(2 * 1024 * MIB)), 64 * MIB);
        assert_eq!(s.calc_optimal_part_size(Some(64 * MIB * 10_000)), 64 * MIB);
        assert_eq!(s.calc_optimal_part_size(Some(64 * MIB * 10_000 + 1)), (64 * MIB * 10_000 + 1).div_ceil(10_000));
        // Deferred length: sized for the 5 TiB maximum.
        assert_eq!(s.calc_optimal_part_size(None), 5_497_558_138_880u64.div_ceil(10_000));
        // The default preferred size is 8 MiB.
        assert_eq!(store(None).calc_optimal_part_size(Some(100 * MIB)), 8 * MIB);
    }

    #[test]
    fn keys_like_the_typescript_store() {
        let s = store(None);
        assert_eq!(s.info_key("abc"), "abc.info");
        assert_eq!(s.part_key("abc", true), "abc.part");
        assert_eq!(s.part_key("abc", false), "abc");
        assert_eq!(
            s.extensions(),
            ["creation", "creation-with-upload", "creation-defer-length", "termination", "expiration"]
        );
        assert_eq!(s.get_expiration(), 0);
        assert_eq!(s.use_complete_tag("false"), None);
    }

    #[test]
    fn modlight_configuration() {
        let s = S3Store::from_env(|k| match k {
            "S3_BUCKET" => Some("media".into()),
            "S3_ACCESS_KEY_ID" => Some("a".into()),
            "S3_SECRET_ACCESS_KEY" => Some("s".into()),
            "S3_PREFIX" => Some("ignored".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(s.bucket(), "media");
        assert_eq!(s.get_expiration(), 24 * 60 * 60 * 1000);
        assert_eq!(s.calc_optimal_part_size(Some(200 * MIB)), 64 * MIB);
        assert_eq!(s.use_complete_tag("true").as_deref(), Some("Tus-Completed=true"));
        assert_eq!(s.info_key("x"), "x.info");
    }

    #[test]
    fn info_object_body_is_the_upload_json() {
        let mut metadata = Metadata::new();
        metadata.insert("filename".into(), Some("a b.png".into()));
        metadata.insert("filetype".into(), Some("image/png".into()));
        metadata.insert("folderId".into(), None);
        metadata.insert("userId".into(), Some("u1".into()));
        let upload = Upload {
            id: "0123456789abcdef0123456789abcdef".into(),
            metadata: Some(metadata),
            size: Some(10_485_760),
            offset: 0,
            creation_date: Some("2026-10-19T12:00:00.000Z".into()),
            storage: Some(UploadStorage {
                kind: "s3".into(),
                path: "0123456789abcdef0123456789abcdef".into(),
                bucket: Some("media".into()),
            }),
        };
        let json = serde_json::to_string(&upload).unwrap();
        assert_eq!(
            json,
            r#"{"id":"0123456789abcdef0123456789abcdef","metadata":{"filename":"a b.png","filetype":"image/png","folderId":null,"userId":"u1"},"size":10485760,"offset":0,"creation_date":"2026-10-19T12:00:00.000Z","storage":{"type":"s3","path":"0123456789abcdef0123456789abcdef","bucket":"media"}}"#
        );
        // getMetadata reads it back into the same upload.
        let parsed = upload_from_info(&upload.id, &serde_json::from_str(&json).unwrap());
        assert_eq!(parsed, upload);
        // A deferred length has no "size".
        let mut deferred = upload.clone();
        deferred.size = None;
        let json = serde_json::to_string(&deferred).unwrap();
        assert!(!json.contains("\"size\""));
        assert_eq!(upload_from_info(&upload.id, &serde_json::from_str(&json).unwrap()).size, None);
    }

    #[test]
    fn info_objects_are_read_like_get_metadata() {
        let v: serde_json::Value =
            serde_json::json!({"id": "other", "size": "12", "offset": "3", "metadata": {"n": 5}});
        let u = upload_from_info("abc", &v);
        assert_eq!(u.id, "abc");
        // `Number.isFinite("12")` is false: a string size is no size.
        assert_eq!(u.size, None);
        assert_eq!(u.offset, 3);
        assert_eq!(u.metadata.unwrap().get("n").unwrap().as_deref(), Some("5"));
        assert!(u.creation_date.is_some());
        assert_eq!(u.storage, None);
    }

    #[test]
    fn random_strings_are_base64url() {
        let s = random_string(10);
        assert_eq!(s.len(), 10);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn body_errors_keep_tus_refusals() {
        let err = stream_error(std::io::Error::other(errors::ERR_MAX_SIZE_EXCEEDED));
        assert_eq!(err.status_and_body(), (413, "Maximum size exceeded\n".to_string()));
        let err = stream_error(std::io::Error::other("reset"));
        assert_eq!(err.status_and_body(), (500, "Something went wrong with that request\nreset\n".to_string()));
    }
}
