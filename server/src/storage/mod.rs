//! Port of `lib/storage/index.ts` and `lib/storage/types.ts`: the storage
//! abstraction every upload, conversion and download goes through.
//!
//! Design goals (driven by the large-file reliability requirement):
//! - everything streams; nothing buffers a whole file in memory;
//! - `read_stream` supports byte ranges for HTTP Range / 206 media serving;
//! - `ingest_local_file` finalizes a completed upload (e.g. a tus temp file)
//!   into place without an extra full re-read where the backend can avoid it.
//!
//! Both drivers are complete: [`local`] (lib/storage/local.ts) and [`s3`]
//! (lib/storage/s3.ts, with the lib-storage multipart upload and the
//! s3-request-presigner it uses).

pub mod local;
pub mod s3;

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::Stream;
use futures_util::future::BoxFuture;

use crate::config::Config;
pub use local::LocalDriver;
pub use s3::{S3Driver, is_not_found};

/// A stream of file bytes.
pub type ByteStream = Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>;

/// A buffer as a one-chunk stream (`storage.write(key, buffer)` upstream).
pub fn bytes_stream(bytes: Bytes) -> ByteStream {
    Box::pin(futures_util::stream::once(async move { Ok(bytes) }))
}

/// Inclusive byte range, as upstream's `ByteRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    /// Inclusive end offset.
    pub end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatResult {
    pub size: u64,
}

/// The storage interface (`StorageDriver` upstream).
pub trait StorageDriver: Send + Sync {
    /// `"local"` or `"s3"`.
    fn name(&self) -> &'static str;

    /// Stream-write `body` to `path` (creates parent dirs as needed).
    fn write<'a>(&'a self, path: &'a str, body: ByteStream) -> BoxFuture<'a, anyhow::Result<()>>;

    /// Move an already-complete local temp file into storage at `path`
    /// (rename on the local driver, upload on S3).
    fn ingest_local_file<'a>(&'a self, local_tmp_path: &'a Path, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;

    /// Open a read stream, optionally for a byte range.
    fn read_stream<'a>(&'a self, path: &'a str, range: Option<ByteRange>) -> BoxFuture<'a, anyhow::Result<ByteStream>>;

    /// Read the whole object (small files only: text preview, thumb sources).
    fn read_buffer<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Bytes>>;

    /// `None` for a missing object.
    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Option<StatResult>>>;

    fn exists<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<bool>> {
        Box::pin(async move { Ok(self.stat(path).await?.is_some()) })
    }

    fn delete<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;

    /// Delete every object under `prefix` (which must end in "/"). Missing
    /// prefixes are success.
    fn delete_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;

    /// A presigned URL to read the object directly (S3 only).
    fn presign_get<'a>(
        &'a self,
        _path: &'a str,
        _expires_in_seconds: u64,
    ) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }

    /// A filesystem path an external process (ffmpeg) may open, or `None`.
    fn local_path(&self, _path: &str) -> anyhow::Result<Option<PathBuf>> {
        Ok(None)
    }
}

/// `getStorage()`: the configured driver, or the error building it raised
/// (upstream threw on every call, which is what /api/health reports).
#[derive(Clone)]
pub struct Storage(Result<Arc<dyn StorageDriver>, Arc<String>>);

impl Storage {
    /// Built like upstream: from `process.env` directly, not from lib/config.
    /// (`STORAGE_LOCAL_PATH ?? "./data/storage"`: an empty value means the
    /// working directory here, where lib/config's `||` falls back.)
    pub fn from_env(_config: &Config) -> Storage {
        let var = |k: &str| std::env::var(k).ok();
        let driver = var("STORAGE_DRIVER").unwrap_or_else(|| "local".to_string()).to_lowercase();
        if driver == "s3" {
            return match S3Driver::from_env(var) {
                Ok(d) => Storage(Ok(Arc::new(d))),
                Err(e) => Storage(Err(Arc::new(e.to_string()))),
            };
        }
        let root = var("STORAGE_LOCAL_PATH").unwrap_or_else(|| "./data/storage".to_string());
        Storage(Ok(Arc::new(LocalDriver::new(&root))))
    }

    /// A specific driver (tests, tools).
    pub fn with_driver(driver: Arc<dyn StorageDriver>) -> Storage {
        Storage(Ok(driver))
    }

    pub fn get(&self) -> anyhow::Result<Arc<dyn StorageDriver>> {
        match &self.0 {
            Ok(d) => Ok(d.clone()),
            Err(e) => Err(anyhow::anyhow!("{e}")),
        }
    }
}

/// `readHead(key, bytes)`: the first bytes of an object for content sniffing,
/// empty when it cannot be read.
pub async fn read_head(storage: &dyn StorageDriver, key: &str, bytes: u64) -> Bytes {
    use futures_util::StreamExt;
    let Ok(mut stream) = storage.read_stream(key, Some(ByteRange { start: 0, end: bytes.saturating_sub(1) })).await
    else {
        return Bytes::new();
    };
    let mut buf = Vec::new();
    while let Some(Ok(chunk)) = stream.next().await {
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 >= bytes {
            break;
        }
    }
    buf.truncate(bytes as usize);
    Bytes::from(buf)
}
