//! Port of `@aws-sdk/lib-storage` 3.1058.0 `Upload`, the class every
//! lib/storage/s3.ts write goes through (`new Upload({client, params: {Bucket,
//! Key, Body}, queueSize: 4, partSize: 64 MiB}).done()`).
//!
//! Same requests in the same order: a body of at most one part is one
//! `PutObject`; anything longer is `CreateMultipartUpload` (with
//! `ChecksumAlgorithm: CRC32`, as lib-storage adds it whenever the client
//! computes checksums "when supported", the default), `queueSize` concurrent
//! `UploadPart`s fed from one shared chunker, then `CompleteMultipartUpload`
//! with the parts sorted and their CRC32s. When any part fails the other
//! uploaders stop taking parts, the multipart upload is aborted and the first
//! failure is returned.
//!
//! A stream is cut into in-memory parts like `getChunkStream` (a full part is
//! emitted only once more bytes follow it, so a body of exactly one part is a
//! single PUT). A file (`ingestLocalFile`) has the same part boundaries, but
//! each part is read straight from the file instead of through memory.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::bail;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::{ByteStream as SdkStream, Length};
use aws_sdk_s3::types::{ChecksumAlgorithm, CompletedMultipartUpload, CompletedPart};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;

use super::{S3Error, sdk_err};
use crate::storage::ByteStream;

/// `Upload.MIN_PART_SIZE`
pub const MIN_PART_SIZE: u64 = 1024 * 1024 * 5;
/// `Upload.MAX_PARTS`
pub const MAX_PARTS: u64 = 10_000;
/// `Upload.queueSize` default.
pub const DEFAULT_QUEUE_SIZE: usize = 4;

/// `params.Body`
pub enum Body {
    /// A stream of unknown length (`Readable`).
    Stream(ByteStream),
    /// A file of known size (`fs.createReadStream(path)`, sized by lstat).
    File { path: PathBuf, size: u64 },
}

pub struct UploadOptions {
    pub client: Client,
    pub bucket: String,
    pub key: String,
    pub body: Body,
    /// 0 means the default (4).
    pub queue_size: usize,
    /// 0 means `max(MIN_PART_SIZE, ceil(totalBytes / MAX_PARTS))`.
    pub part_size: u64,
}

pub struct Upload {
    client: Client,
    bucket: String,
    key: String,
    queue_size: usize,
    part_size: u64,
    total_bytes: Option<u64>,
    expected_parts_count: Option<u64>,
    body: Body,
}

/// One chunk handed to an uploader (`dataPart`).
#[derive(Debug)]
struct DataPart {
    part_number: i32,
    data: PartData,
    last_part: bool,
}

#[derive(Debug)]
enum PartData {
    Memory(Bytes),
    File { path: Arc<PathBuf>, offset: u64, len: u64 },
}

impl PartData {
    async fn into_sdk_stream(self) -> anyhow::Result<SdkStream> {
        match self {
            PartData::Memory(bytes) => Ok(SdkStream::from(bytes)),
            PartData::File { len: 0, .. } => Ok(SdkStream::from_static(b"")),
            PartData::File { path, offset, len } => Ok(SdkStream::read_from()
                .path(path.as_path())
                .offset(offset)
                .length(Length::Exact(len))
                .build()
                .await?),
        }
    }
}

/// `getChunk(data, partSize)`: the async generator all uploaders share.
enum Feeder {
    Stream { body: ByteStream, buffer: BytesMut, part_number: i32, ended: bool, done: bool },
    File { path: Arc<PathBuf>, size: u64, offset: u64, part_number: i32, done: bool },
}

impl Feeder {
    fn new(body: Body) -> Feeder {
        match body {
            Body::Stream(body) => {
                Feeder::Stream { body, buffer: BytesMut::new(), part_number: 1, ended: false, done: false }
            }
            Body::File { path, size } => {
                Feeder::File { path: Arc::new(path), size, offset: 0, part_number: 1, done: false }
            }
        }
    }

    /// The generator's `return()`: later calls see the end.
    fn close(&mut self) {
        match self {
            Feeder::Stream { done, .. } | Feeder::File { done, .. } => *done = true,
        }
    }

    async fn next(&mut self, part_size: u64) -> anyhow::Result<Option<DataPart>> {
        match self {
            Feeder::Stream { body, buffer, part_number, ended, done } => {
                if *done {
                    return Ok(None);
                }
                loop {
                    // `while (currentBuffer.length > partSize) yield ...`
                    if buffer.len() as u64 > part_size {
                        let data = buffer.split_to(part_size as usize).freeze();
                        let n = *part_number;
                        *part_number += 1;
                        return Ok(Some(DataPart { part_number: n, data: PartData::Memory(data), last_part: false }));
                    }
                    if *ended {
                        *done = true;
                        let data = std::mem::take(buffer).freeze();
                        return Ok(Some(DataPart {
                            part_number: *part_number,
                            data: PartData::Memory(data),
                            last_part: true,
                        }));
                    }
                    match body.next().await {
                        Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                        Some(Err(err)) => {
                            *done = true;
                            return Err(err.into());
                        }
                        None => *ended = true,
                    }
                }
            }
            Feeder::File { path, size, offset, part_number, done } => {
                if *done {
                    return Ok(None);
                }
                let remaining = *size - *offset;
                if remaining > part_size {
                    let part = DataPart {
                        part_number: *part_number,
                        data: PartData::File { path: path.clone(), offset: *offset, len: part_size },
                        last_part: false,
                    };
                    *offset += part_size;
                    *part_number += 1;
                    return Ok(Some(part));
                }
                *done = true;
                Ok(Some(DataPart {
                    part_number: *part_number,
                    data: PartData::File { path: path.clone(), offset: *offset, len: remaining },
                    last_part: true,
                }))
            }
        }
    }
}

/// The mutable state of one `done()` run, shared by the uploaders.
struct Shared {
    feeder: tokio::sync::Mutex<Feeder>,
    /// Set when an uploader failed: the generator was closed by its `for await`.
    stop: AtomicBool,
    /// `createMultiPartPromise`: the outcome every uploader awaits.
    create: tokio::sync::Mutex<Option<Result<String, String>>>,
    /// `abortMultipartUploadCommand` (the upload id while it may be aborted).
    abortable: StdMutex<Option<String>>,
    uploaded_parts: StdMutex<Vec<CompletedPart>>,
    upload_enqueued_parts_count: AtomicU64,
    is_multi_part: AtomicBool,
    failures: StdMutex<Vec<anyhow::Error>>,
}

impl Upload {
    pub fn new(options: UploadOptions) -> anyhow::Result<Upload> {
        let queue_size = if options.queue_size == 0 { DEFAULT_QUEUE_SIZE } else { options.queue_size };
        let total_bytes = match &options.body {
            Body::File { size, .. } => Some(*size),
            Body::Stream(_) => None,
        };
        let part_size = if options.part_size == 0 {
            MIN_PART_SIZE.max(total_bytes.unwrap_or(0).div_ceil(MAX_PARTS))
        } else {
            options.part_size
        };
        // `__validateInput`
        if part_size < MIN_PART_SIZE {
            bail!(
                "EntityTooSmall: Your proposed upload part size [{part_size}] is smaller than the minimum allowed size [{MIN_PART_SIZE}] (5MB)"
            );
        }
        Ok(Upload {
            client: options.client,
            bucket: options.bucket,
            key: options.key,
            queue_size,
            part_size,
            total_bytes,
            expected_parts_count: total_bytes.map(|t| t.div_ceil(part_size)),
            body: options.body,
        })
    }

    /// `done()` (`__doMultipartUpload`).
    pub async fn done(self) -> anyhow::Result<()> {
        let Upload { client, bucket, key, queue_size, part_size, total_bytes, expected_parts_count, body } = self;
        let shared = Shared {
            feeder: tokio::sync::Mutex::new(Feeder::new(body)),
            stop: AtomicBool::new(false),
            create: tokio::sync::Mutex::new(None),
            abortable: StdMutex::new(None),
            uploaded_parts: StdMutex::new(Vec::new()),
            upload_enqueued_parts_count: AtomicU64::new(0),
            is_multi_part: AtomicBool::new(true),
            failures: StdMutex::new(Vec::new()),
        };
        let ctx = Ctx { client: &client, bucket: &bucket, key: &key, part_size, shared: &shared };

        let uploaders = (0..queue_size).map(|_| async {
            if let Err(err) = ctx.do_concurrent_upload().await {
                ctx.shared.stop.store(true, Ordering::SeqCst);
                ctx.shared.failures.lock().unwrap_or_else(|e| e.into_inner()).push(err);
            }
        });
        futures_util::future::join_all(uploaders).await;

        let mut failures = std::mem::take(&mut *shared.failures.lock().unwrap_or_else(|e| e.into_inner()));
        if !failures.is_empty() {
            ctx.mark_upload_as_aborted().await?;
            return Err(failures.swap_remove(0));
        }

        if shared.is_multi_part.load(Ordering::SeqCst) {
            let mut parts = std::mem::take(&mut *shared.uploaded_parts.lock().unwrap_or_else(|e| e.into_inner()));
            if let (Some(total), Some(expected)) = (total_bytes, expected_parts_count)
                && parts.len() as u64 != expected
            {
                bail!(
                    "Expected {expected} part(s) but uploaded {} part(s).\n\
                     The expected part count is based on the byte-count of the input.params.Body,\n\
                     which was read from the size of the file given by Body.path on disk as reported by lstatSync and is {total}.\n\
                     If this is not correct, provide an override value by setting a number\n\
                     to input.params.ContentLength in bytes.\n",
                    parts.len()
                );
            }
            parts.sort_by_key(|p| p.part_number().unwrap_or(0));
            let upload_id = match shared.create.lock().await.clone() {
                Some(Ok(upload_id)) => upload_id,
                _ => String::new(),
            };
            client
                .complete_multipart_upload()
                .bucket(&bucket)
                .key(&key)
                .upload_id(upload_id)
                .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(parts)).build())
                .send()
                .await
                .map_err(sdk_err)?;
        }
        // `this.abortMultipartUploadCommand = null`
        *shared.abortable.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

struct Ctx<'a> {
    client: &'a Client,
    bucket: &'a str,
    key: &'a str,
    part_size: u64,
    shared: &'a Shared,
}

impl Ctx<'_> {
    /// `__doConcurrentUpload(dataFeeder)`: one of the `queueSize` loops.
    async fn do_concurrent_upload(&self) -> anyhow::Result<()> {
        let shared = self.shared;
        loop {
            let part = {
                let mut feeder = shared.feeder.lock().await;
                if shared.stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                match feeder.next(self.part_size).await {
                    Ok(Some(part)) => part,
                    Ok(None) => return Ok(()),
                    Err(err) => {
                        feeder.close();
                        return Err(err);
                    }
                }
            };
            if shared.upload_enqueued_parts_count.load(Ordering::SeqCst) > MAX_PARTS {
                bail!("Exceeded {MAX_PARTS} parts in multipart upload to Bucket: {} Key: {}.", self.bucket, self.key);
            }
            if part.part_number == 1 && part.last_part {
                return self.upload_using_put(part).await;
            }
            let upload_id = self.create_multipart_upload().await?;
            shared.upload_enqueued_parts_count.fetch_add(1, Ordering::SeqCst);
            let part_number = part.part_number;
            let res = self
                .client
                .upload_part()
                .bucket(self.bucket)
                .key(self.key)
                .upload_id(upload_id)
                .body(part.data.into_sdk_stream().await?)
                .part_number(part_number)
                .send()
                .await
                .map_err(sdk_err)?;
            let Some(etag) = res.e_tag() else {
                bail!(
                    "Part {part_number} is missing ETag in UploadPart response. Missing Bucket CORS configuration for ETag header?"
                );
            };
            let completed = CompletedPart::builder()
                .part_number(part_number)
                .e_tag(etag)
                .set_checksum_crc32(res.checksum_crc32().map(str::to_string))
                .set_checksum_crc32_c(res.checksum_crc32_c().map(str::to_string))
                .set_checksum_sha1(res.checksum_sha1().map(str::to_string))
                .set_checksum_sha256(res.checksum_sha256().map(str::to_string))
                .build();
            shared.uploaded_parts.lock().unwrap_or_else(|e| e.into_inner()).push(completed);
        }
    }

    /// `__uploadUsingPut(dataPart)`
    async fn upload_using_put(&self, part: DataPart) -> anyhow::Result<()> {
        self.shared.is_multi_part.store(false, Ordering::SeqCst);
        self.client
            .put_object()
            .bucket(self.bucket)
            .key(self.key)
            .body(part.data.into_sdk_stream().await?)
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    /// `__createMultipartUpload()`: created once, awaited by every uploader.
    async fn create_multipart_upload(&self) -> anyhow::Result<String> {
        let mut created = self.shared.create.lock().await;
        if let Some(outcome) = created.as_ref() {
            return outcome.clone().map_err(|message| S3Error::msg(message).into());
        }
        let res = self
            .client
            .create_multipart_upload()
            .bucket(self.bucket)
            .key(self.key)
            .checksum_algorithm(ChecksumAlgorithm::Crc32)
            .send()
            .await;
        match res {
            Ok(out) => {
                let upload_id = out.upload_id().unwrap_or_default().to_string();
                *self.shared.abortable.lock().unwrap_or_else(|e| e.into_inner()) = Some(upload_id.clone());
                *created = Some(Ok(upload_id.clone()));
                Ok(upload_id)
            }
            Err(err) => {
                let err = S3Error::from_sdk(err);
                *created = Some(Err(err.message.clone()));
                Err(err.into())
            }
        }
    }

    /// `markUploadAsAborted()`
    async fn mark_upload_as_aborted(&self) -> anyhow::Result<()> {
        let upload_id = self.shared.abortable.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(upload_id) = upload_id {
            self.client
                .abort_multipart_upload()
                .bucket(self.bucket)
                .key(self.key)
                .upload_id(upload_id)
                .send()
                .await
                .map_err(sdk_err)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_of(chunks: Vec<&'static [u8]>) -> ByteStream {
        Box::pin(futures_util::stream::iter(chunks.into_iter().map(|c| Ok(Bytes::from_static(c)))))
    }

    async fn drain(mut feeder: Feeder, part_size: u64) -> Vec<(i32, u64, bool)> {
        let mut out = Vec::new();
        while let Some(p) = feeder.next(part_size).await.unwrap() {
            let len = match &p.data {
                PartData::Memory(b) => b.len() as u64,
                PartData::File { len, .. } => *len,
            };
            out.push((p.part_number, len, p.last_part));
        }
        out
    }

    #[tokio::test]
    async fn stream_parts_follow_get_chunk_stream() {
        // Exactly one part: a single PUT, never an empty second part.
        assert_eq!(drain(Feeder::new(Body::Stream(stream_of(vec![b"abcd"]))), 4).await, vec![(1, 4, true)]);
        assert_eq!(drain(Feeder::new(Body::Stream(stream_of(vec![b"ab", b"cd"]))), 4).await, vec![(1, 4, true)]);
        // Empty body: one empty last part.
        assert_eq!(drain(Feeder::new(Body::Stream(stream_of(vec![]))), 4).await, vec![(1, 0, true)]);
        // One byte over: a full part and a one-byte last part.
        assert_eq!(
            drain(Feeder::new(Body::Stream(stream_of(vec![b"abcde"]))), 4).await,
            vec![(1, 4, false), (2, 1, true)]
        );
        // Two exact parts.
        assert_eq!(
            drain(Feeder::new(Body::Stream(stream_of(vec![b"abc", b"defgh"]))), 4).await,
            vec![(1, 4, false), (2, 4, true)]
        );
        // A chunk spanning several parts.
        assert_eq!(
            drain(Feeder::new(Body::Stream(stream_of(vec![b"abcdefghij", b"k"]))), 4).await,
            vec![(1, 4, false), (2, 4, false), (3, 3, true)]
        );
    }

    #[tokio::test]
    async fn file_parts_have_the_same_boundaries() {
        let file = |size| Feeder::new(Body::File { path: PathBuf::from("/nonexistent"), size });
        assert_eq!(drain(file(0), 4).await, vec![(1, 0, true)]);
        assert_eq!(drain(file(4), 4).await, vec![(1, 4, true)]);
        assert_eq!(drain(file(5), 4).await, vec![(1, 4, false), (2, 1, true)]);
        assert_eq!(drain(file(8), 4).await, vec![(1, 4, false), (2, 4, true)]);
        assert_eq!(drain(file(11), 4).await, vec![(1, 4, false), (2, 4, false), (3, 3, true)]);
    }

    #[tokio::test]
    async fn a_failing_stream_closes_the_feeder() {
        let body: ByteStream = Box::pin(futures_util::stream::iter(vec![
            Ok(Bytes::from_static(b"abcdef")),
            Err(std::io::Error::other("client went away")),
        ]));
        let mut feeder = Feeder::new(Body::Stream(body));
        assert_eq!(feeder.next(4).await.unwrap().unwrap().part_number, 1);
        assert_eq!(feeder.next(4).await.unwrap_err().to_string(), "client went away");
        assert!(feeder.next(4).await.unwrap().is_none());
    }

    #[test]
    fn options_are_validated_like_lib_storage() {
        let client = super::super::s3_client(&super::super::S3Config {
            endpoint: Some("http://127.0.0.1:1".into()),
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "a".into(),
            secret_access_key: "s".into(),
            force_path_style: true,
            prefix: None,
        });
        let opts = |part_size, body| UploadOptions {
            client: client.clone(),
            bucket: "b".into(),
            key: "k".into(),
            body,
            queue_size: 0,
            part_size,
        };
        let err = Upload::new(opts(1024, Body::Stream(stream_of(vec![])))).err().unwrap();
        assert_eq!(
            err.to_string(),
            "EntityTooSmall: Your proposed upload part size [1024] is smaller than the minimum allowed size [5242880] (5MB)"
        );
        let u = Upload::new(opts(0, Body::File { path: "/x".into(), size: 100 * 1024 * 1024 * 1024 })).unwrap();
        assert_eq!(u.queue_size, 4);
        assert_eq!(u.part_size, MIN_PART_SIZE.max((100u64 << 30).div_ceil(MAX_PARTS)));
        assert_eq!(u.expected_parts_count, Some((100u64 << 30).div_ceil(u.part_size)));
    }
}
