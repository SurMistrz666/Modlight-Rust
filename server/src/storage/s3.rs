//! Port of `lib/storage/s3.ts`: S3-compatible storage (AWS S3, MinIO,
//! Cloudflare R2, Backblaze B2, DigitalOcean Spaces). Uploads stream via
//! multipart (lib-storage `Upload`, ported in [`lib_storage`]), so a multi-GB
//! file is never held in memory; large downloads can be offloaded to the
//! bucket via presigned URLs ([`presigner`]).
//!
//! The client is the official AWS SDK (`aws-sdk-s3`) configured like
//! upstream's `new S3Client({region, endpoint, forcePathStyle, credentials})`:
//! static credentials, everything else at the SDK defaults, which match the
//! JavaScript SDK's (standard retries with 3 attempts, CRC32 request checksums
//! and response checksum validation "when supported", no proxy). HTTPS uses
//! rustls with ring, like the rest of the crate. Unlike Node, the Rust SDK
//! also fails a transfer the server stalls (no bytes for 5 s while being
//! read) and gives up connecting after 3.1 s; a reader that pauses (a paused
//! video) is not a stall.

pub mod lib_storage;
pub mod presigner;

use std::path::Path;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use bytes::Bytes;
use futures_util::future::BoxFuture;

use super::{ByteRange, ByteStream, StatResult, StorageDriver};

/// lib/storage/s3.ts `write()`: "64 MB parts, up to 4 concurrent - good for
/// large files without huge memory."
pub const WRITE_PART_SIZE: u64 = 64 * 1024 * 1024;
pub const WRITE_QUEUE_SIZE: usize = 4;

/// `presignGet(key, expiresInSeconds = 3600)`.
pub const PRESIGN_DEFAULT_EXPIRES_IN: u64 = 3600;

#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    /// Optional key prefix inside the bucket.
    pub prefix: Option<String>,
}

impl S3Config {
    /// Read the S3_* variables as lib/storage/index.ts does, with its
    /// "Missing required env var X for S3 storage driver" refusal.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<S3Config> {
        let non_empty = |k: &str| var(k).filter(|v| !v.is_empty());
        let required =
            |k: &str| non_empty(k).ok_or_else(|| anyhow::anyhow!("Missing required env var {k} for S3 storage driver"));
        Ok(S3Config {
            endpoint: non_empty("S3_ENDPOINT"),
            region: non_empty("S3_REGION").unwrap_or_else(|| "auto".to_string()),
            bucket: required("S3_BUCKET")?,
            access_key_id: required("S3_ACCESS_KEY_ID")?,
            secret_access_key: required("S3_SECRET_ACCESS_KEY")?,
            force_path_style: var("S3_FORCE_PATH_STYLE").as_deref() == Some("true"),
            prefix: non_empty("S3_PREFIX"),
        })
    }
}

/// `new S3Client({ region, endpoint: endpoint || undefined, forcePathStyle,
/// credentials })`, also what `@tus/s3-store` builds from its `s3ClientConfig`.
pub fn s3_client(config: &S3Config) -> Client {
    let http_client = aws_smithy_http_client::Builder::new()
        .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
            aws_smithy_http_client::tls::rustls_provider::CryptoMode::Ring,
        ))
        .build_https();
    let mut builder = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(config.region.clone()))
        .credentials_provider(Credentials::new(
            config.access_key_id.clone(),
            config.secret_access_key.clone(),
            None,
            None,
            "modlight-env",
        ))
        .force_path_style(config.force_path_style)
        .http_client(http_client);
    if let Some(endpoint) = config.endpoint.as_deref().filter(|e| !e.is_empty()) {
        builder = builder.endpoint_url(endpoint);
    }
    Client::from_conf(builder.build())
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// A failed S3 request, reduced to what upstream's callers read off the
/// JavaScript SDK's exceptions: the service error code (`name` / `Code`:
/// `NoSuchKey`, `NotFound`, `NoSuchUpload`, ...), the HTTP status and
/// `message` (also what `@tus/server` puts into a 500 body).
#[derive(Debug)]
pub struct S3Error {
    /// The S3 error code (`<Code>` of the error document; `NotFound` for a
    /// HEAD answered 404, which has no body).
    pub code: Option<String>,
    pub status: Option<u16>,
    /// The service's `<Message>`, `"UnknownError"` when it sent none (what the
    /// JavaScript SDK puts in `error.message`), or the transport failure.
    pub message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl S3Error {
    pub fn from_sdk<E>(err: SdkError<E, aws_sdk_s3::config::http::HttpResponse>) -> S3Error
    where
        E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
    {
        let status = err.raw_response().map(|r| r.status().as_u16());
        let code = err.code().map(str::to_string);
        let message = match &err {
            SdkError::ServiceError(_) => err.message().unwrap_or("UnknownError").to_string(),
            other => display_chain(other),
        };
        S3Error { code, status, message, source: Some(Box::new(err)) }
    }

    /// An error with only a message (failures the ported code raises itself).
    pub fn msg(message: impl Into<String>) -> S3Error {
        S3Error { code: None, status: None, message: message.into(), source: None }
    }

    pub fn has_code(&self, code: &str) -> bool {
        self.code.as_deref() == Some(code)
    }

    /// The SDK's own error, for diagnostics.
    pub fn sdk_error(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        self.source.as_deref()
    }
}

impl std::fmt::Display for S3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The message already spells out the SDK error's chain, so the SDK error is
/// kept for `Debug` only and not exposed as `source()` (which would repeat
/// it in `{:#}` output).
impl std::error::Error for S3Error {}

/// `a: b: c` from an error and its sources, skipping a source whose text its
/// parent already ends with.
fn display_chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut next = err.source();
    while let Some(e) = next {
        let text = e.to_string();
        if !out.ends_with(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        next = e.source();
    }
    out
}

/// `SdkError` -> `anyhow::Error` carrying an [`S3Error`].
pub(crate) fn sdk_err<E>(err: SdkError<E, aws_sdk_s3::config::http::HttpResponse>) -> anyhow::Error
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    anyhow::Error::new(S3Error::from_sdk(err))
}

/// The [`S3Error`] somewhere in `err`'s chain (also inside an `io::Error`, as
/// stream errors carry it).
pub fn s3_error(err: &anyhow::Error) -> Option<&S3Error> {
    err.chain().find_map(|e| {
        e.downcast_ref::<S3Error>().or_else(|| {
            e.downcast_ref::<std::io::Error>().and_then(|io| io.get_ref()).and_then(|i| i.downcast_ref::<S3Error>())
        })
    })
}

/// `isMissingObjectError` of lib/media/actions.ts and lib/maintenance/trash.ts:
/// "Local ENOENT / S3 NoSuchKey: the object is already gone, which is
/// success" (`code === "ENOENT" || name === "NoSuchKey" || name === "NotFound"
/// || Code === "NoSuchKey"`). Answers for both drivers.
pub fn is_not_found(err: &anyhow::Error) -> bool {
    let gone = |s3: &S3Error| s3.has_code("NoSuchKey") || s3.has_code("NotFound");
    err.chain().any(|e| {
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            return io.kind() == std::io::ErrorKind::NotFound
                || io.get_ref().and_then(|i| i.downcast_ref::<S3Error>()).is_some_and(gone);
        }
        e.downcast_ref::<S3Error>().is_some_and(gone)
    })
}

// ── The driver ──────────────────────────────────────────────────────────────

pub struct S3Driver {
    pub config: S3Config,
    client: Client,
    bucket: String,
    prefix: String,
}

/// `(cfg.prefix ?? "").replace(/^\/+|\/+$/g, "")`
fn trim_prefix(prefix: Option<&str>) -> String {
    prefix.unwrap_or("").trim_matches('/').to_string()
}

/// `this.prefix ? `${this.prefix}/${k}` : k`
fn prefixed_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() { key.to_string() } else { format!("{prefix}/{key}") }
}

/// The GetObject `Range` header for an inclusive byte range.
pub fn range_header(range: ByteRange) -> String {
    format!("bytes={}-{}", range.start, range.end)
}

/// An SDK body as the crate's [`ByteStream`].
pub(crate) fn into_byte_stream(body: aws_sdk_s3::primitives::ByteStream) -> ByteStream {
    Box::pin(futures_util::stream::unfold(Some(body), |state| async move {
        let mut body = state?;
        match body.next().await {
            Some(Ok(bytes)) => Some((Ok(bytes), Some(body))),
            Some(Err(e)) => Some((Err(std::io::Error::other(e)), None)),
            None => None,
        }
    }))
}

impl S3Driver {
    pub fn new(config: S3Config) -> S3Driver {
        let client = s3_client(&config);
        S3Driver { bucket: config.bucket.clone(), prefix: trim_prefix(config.prefix.as_deref()), client, config }
    }

    /// Read the S3_* variables as lib/storage/index.ts does, with its
    /// "Missing required env var X for S3 storage driver" refusal.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<S3Driver> {
        Ok(S3Driver::new(S3Config::from_env(var)?))
    }

    /// The SDK client (tests and tools).
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The bucket key a storage key maps to (`key()` upstream).
    pub fn key(&self, k: &str) -> String {
        prefixed_key(&self.prefix, k)
    }

    async fn upload(&self, key: String, body: lib_storage::Body) -> anyhow::Result<()> {
        let upload = lib_storage::Upload::new(lib_storage::UploadOptions {
            client: self.client.clone(),
            bucket: self.bucket.clone(),
            key,
            body,
            queue_size: WRITE_QUEUE_SIZE,
            part_size: WRITE_PART_SIZE,
        })?;
        upload.done().await
    }

    /// `presignGet(key, expiresInSeconds)` signed at an explicit time.
    pub async fn presign_get_at(
        &self,
        key: &str,
        expires_in_seconds: u64,
        signing_date: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<String> {
        presigner::get_signed_url_get_object(&self.config, &self.key(key), expires_in_seconds, signing_date).await
    }
}

impl StorageDriver for S3Driver {
    fn name(&self) -> &'static str {
        "s3"
    }

    fn write<'a>(&'a self, path: &'a str, body: ByteStream) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { self.upload(self.key(path), lib_storage::Body::Stream(body)).await })
    }

    fn ingest_local_file<'a>(&'a self, local_tmp_path: &'a Path, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            // `this.write(key, createReadStream(localTmpPath))`: lib-storage
            // sizes a file stream with lstat, so the parts are the file's
            // 64 MiB slices; they are read straight from disk here.
            let size = tokio::fs::metadata(local_tmp_path).await?.len();
            let body = lib_storage::Body::File { path: local_tmp_path.to_path_buf(), size };
            self.upload(self.key(path), body).await?;
            // `rm(localTmpPath, { force: true })`
            match tokio::fs::remove_file(local_tmp_path).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            }
        })
    }

    fn read_stream<'a>(&'a self, path: &'a str, range: Option<ByteRange>) -> BoxFuture<'a, anyhow::Result<ByteStream>> {
        Box::pin(async move {
            let res = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(self.key(path))
                .set_range(range.map(range_header))
                .send()
                .await
                .map_err(sdk_err)?;
            Ok(into_byte_stream(res.body))
        })
    }

    fn read_buffer<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Bytes>> {
        Box::pin(async move {
            let res =
                self.client.get_object().bucket(&self.bucket).key(self.key(path)).send().await.map_err(sdk_err)?;
            let bytes = res.body.collect().await.map_err(std::io::Error::other)?;
            Ok(bytes.into_bytes())
        })
    }

    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Option<StatResult>>> {
        // Upstream catches every error (`catch { return null; }`): a missing
        // object, but also an unreachable endpoint, reads as "no such object".
        Box::pin(async move {
            let key = self.key(path);
            match self.client.head_object().bucket(&self.bucket).key(&key).send().await {
                Ok(res) => Ok(Some(StatResult { size: res.content_length().unwrap_or(0).max(0) as u64 })),
                Err(err) => {
                    tracing::debug!(key = %key, error = %DisplayErrorContext(&err), "S3 HeadObject failed");
                    Ok(None)
                }
            }
        })
    }

    fn delete<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.client.delete_object().bucket(&self.bucket).key(self.key(path)).send().await.map_err(sdk_err)?;
            Ok(())
        })
    }

    fn delete_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let normalized = if prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
            if normalized == "/" {
                anyhow::bail!("deletePrefix: refusing to empty the bucket");
            }
            let mut token: Option<String> = None;
            loop {
                let listed = self
                    .client
                    .list_objects_v2()
                    .bucket(&self.bucket)
                    .prefix(self.key(&normalized))
                    .set_continuation_token(token.take())
                    .send()
                    .await
                    .map_err(sdk_err)?;
                let keys: Vec<String> = listed.contents().iter().filter_map(|o| o.key().map(str::to_string)).collect();
                if !keys.is_empty() {
                    // DeleteObjects takes 1000 keys per call, which is also the page size
                    // ListObjectsV2 returns, so one round trip per page.
                    let objects = keys
                        .into_iter()
                        .map(|key| ObjectIdentifier::builder().key(key).build())
                        .collect::<Result<Vec<_>, _>>()?;
                    let delete = Delete::builder().set_objects(Some(objects)).quiet(true).build()?;
                    self.client.delete_objects().bucket(&self.bucket).delete(delete).send().await.map_err(sdk_err)?;
                }
                token = match listed.is_truncated() {
                    Some(true) => listed.next_continuation_token().map(str::to_string),
                    _ => None,
                };
                if token.is_none() {
                    return Ok(());
                }
            }
        })
    }

    fn presign_get<'a>(
        &'a self,
        path: &'a str,
        expires_in_seconds: u64,
    ) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        Box::pin(async move { Ok(Some(self.presign_get_at(path, expires_in_seconds, chrono::Utc::now()).await?)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: std::collections::HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn missing_variables_are_refused_like_upstream() {
        let err = S3Driver::from_env(|k| (k == "S3_BUCKET").then(|| "b".to_string())).err().unwrap();
        assert_eq!(err.to_string(), "Missing required env var S3_ACCESS_KEY_ID for S3 storage driver");
        let err = S3Driver::from_env(env(&[("S3_BUCKET", ""), ("S3_ACCESS_KEY_ID", "a")])).err().unwrap();
        assert_eq!(err.to_string(), "Missing required env var S3_BUCKET for S3 storage driver");
    }

    #[test]
    fn configuration_defaults_like_lib_storage_index() {
        let c = S3Config::from_env(env(&[
            ("S3_BUCKET", "media"),
            ("S3_ACCESS_KEY_ID", "a"),
            ("S3_SECRET_ACCESS_KEY", "s"),
            ("S3_REGION", ""),
            ("S3_ENDPOINT", ""),
            ("S3_FORCE_PATH_STYLE", "TRUE"),
        ]))
        .unwrap();
        assert_eq!(c.region, "auto");
        assert_eq!(c.endpoint, None);
        // `=== "true"`, case-sensitive.
        assert!(!c.force_path_style);
        assert_eq!(c.prefix, None);
        let c = S3Config::from_env(env(&[
            ("S3_BUCKET", "media"),
            ("S3_ACCESS_KEY_ID", "a"),
            ("S3_SECRET_ACCESS_KEY", "s"),
            ("S3_REGION", "eu-central-1"),
            ("S3_ENDPOINT", "http://127.0.0.1:9000"),
            ("S3_FORCE_PATH_STYLE", "true"),
            ("S3_PREFIX", "/modlight/"),
        ]))
        .unwrap();
        assert_eq!(c.region, "eu-central-1");
        assert_eq!(c.endpoint.as_deref(), Some("http://127.0.0.1:9000"));
        assert!(c.force_path_style);
        assert_eq!(c.prefix.as_deref(), Some("/modlight/"));
    }

    #[test]
    fn keys_are_prefixed_like_upstream() {
        assert_eq!(trim_prefix(None), "");
        assert_eq!(trim_prefix(Some("")), "");
        assert_eq!(trim_prefix(Some("/")), "");
        assert_eq!(trim_prefix(Some("//media//")), "media");
        assert_eq!(trim_prefix(Some("a/b/")), "a/b");
        assert_eq!(prefixed_key("", "u1/x.png"), "u1/x.png");
        assert_eq!(prefixed_key("media", "u1/x.png"), "media/u1/x.png");
        assert_eq!(prefixed_key("a/b", "derived/1/"), "a/b/derived/1/");

        let d = S3Driver::new(S3Config {
            endpoint: Some("http://127.0.0.1:1".into()),
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "a".into(),
            secret_access_key: "s".into(),
            force_path_style: true,
            prefix: Some("/tenant/".into()),
        });
        assert_eq!(d.key("u1/a.webp"), "tenant/u1/a.webp");
        assert_eq!(d.name(), "s3");
        assert_eq!(d.local_path("u1/a.webp").unwrap(), None);
    }

    #[test]
    fn range_header_is_inclusive() {
        assert_eq!(range_header(ByteRange { start: 0, end: 4095 }), "bytes=0-4095");
        assert_eq!(range_header(ByteRange { start: 100, end: 299 }), "bytes=100-299");
    }

    #[test]
    fn not_found_detection() {
        let io = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(is_not_found(&io));
        assert!(is_not_found(&io.context("reading u1/x")));
        let denied = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(!is_not_found(&denied));
        let err =
            |code: &str| S3Error { code: Some(code.into()), status: Some(404), message: "m".into(), source: None };
        for (code, expected) in
            [("NoSuchKey", true), ("NotFound", true), ("NoSuchBucket", false), ("AccessDenied", false)]
        {
            assert_eq!(is_not_found(&anyhow::Error::new(err(code))), expected, "{code}");
        }
        // Carried inside a stream error.
        assert!(is_not_found(&anyhow::Error::from(std::io::Error::other(err("NoSuchKey")))));
        assert!(!is_not_found(&anyhow::anyhow!("boom")));
        assert_eq!(s3_error(&anyhow::Error::new(err("NoSuchUpload"))).unwrap().code.as_deref(), Some("NoSuchUpload"));
    }

    #[tokio::test]
    async fn transport_failures_read_as_one_line() {
        let d = S3Driver::new(S3Config {
            endpoint: Some("http://127.0.0.1:1".into()),
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "a".into(),
            secret_access_key: "s".into(),
            force_path_style: true,
            prefix: None,
        });
        let err = d.read_buffer("k").await.unwrap_err();
        let text = err.to_string();
        assert!(text.starts_with("dispatch failure: "), "{text}");
        assert!(text.contains("Connection refused"), "{text}");
        assert!(!text.contains("DispatchFailure {"), "no Debug dump: {text}");
        assert_eq!(format!("{err:#}"), text);
        assert!(!is_not_found(&err));
        let s3 = s3_error(&err).unwrap();
        assert_eq!(s3.code, None);
        assert!(s3.sdk_error().is_some());
    }

    #[tokio::test]
    async fn stat_swallows_every_error_like_upstream() {
        // Nothing listens on port 1: upstream's `catch { return null; }`.
        let d = S3Driver::new(S3Config {
            endpoint: Some("http://127.0.0.1:1".into()),
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "a".into(),
            secret_access_key: "s".into(),
            force_path_style: true,
            prefix: None,
        });
        assert_eq!(d.stat("__healthcheck__").await.unwrap(), None);
        assert!(!d.exists("__healthcheck__").await.unwrap());
        assert!(d.delete_prefix("/").await.unwrap_err().to_string().contains("refusing to empty the bucket"));
        assert!(d.delete_prefix("").await.unwrap_err().to_string().contains("refusing to empty the bucket"));
    }
}
