//! Port of `lib/storage/local.ts`: local filesystem storage. All paths are
//! relative keys like "{userCode}/{code}.{ext}", resolved under one root.

use std::io::SeekFrom;
use std::path::{Component, Path, PathBuf};

use anyhow::bail;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;

use super::{ByteRange, ByteStream, StatResult, StorageDriver};

pub struct LocalDriver {
    root: PathBuf,
}

/// `path.resolve(...)`: absolute and lexically normalised (no symlinks).
pub fn resolve_lexically(base: &Path, rest: &Path) -> PathBuf {
    let joined = if rest.is_absolute() { rest.to_path_buf() } else { base.join(rest) };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl LocalDriver {
    pub fn new(root: &str) -> LocalDriver {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        LocalDriver { root: resolve_lexically(&cwd, Path::new(root)) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a storage key to an absolute path, guarding against traversal.
    pub fn full(&self, key: &str) -> anyhow::Result<PathBuf> {
        let p = resolve_lexically(&self.root, Path::new(key));
        if p != self.root && !p.starts_with(&self.root) {
            bail!("Path traversal blocked: {key}");
        }
        Ok(p)
    }

    /// Convenience for callers that need an absolute path.
    pub fn absolute_path(&self, key: &str) -> anyhow::Result<PathBuf> {
        self.full(key)
    }

    /// `LocalDriver.joinKey(...parts)`
    pub fn join_key(parts: &[&str]) -> String {
        let joined: PathBuf = parts.iter().collect();
        resolve_lexically(Path::new("/"), &joined).to_string_lossy().trim_start_matches('/').to_string()
    }
}

fn stream_file(file: tokio::fs::File, limit: Option<u64>) -> ByteStream {
    match limit {
        Some(n) => Box::pin(ReaderStream::new(file.take(n))),
        None => Box::pin(ReaderStream::new(file)),
    }
}

impl StorageDriver for LocalDriver {
    fn name(&self) -> &'static str {
        "local"
    }

    fn write<'a>(&'a self, path: &'a str, mut body: ByteStream) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let target = self.full(path)?;
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut out = tokio::fs::File::create(&target).await?;
            while let Some(chunk) = body.next().await {
                out.write_all(&chunk?).await?;
            }
            out.flush().await?;
            Ok(())
        })
    }

    fn ingest_local_file<'a>(&'a self, local_tmp_path: &'a Path, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let target = self.full(path)?;
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            // Fast path: atomic rename when on the same filesystem.
            match tokio::fs::rename(local_tmp_path, &target).await {
                Ok(()) => Ok(()),
                Err(err) => {
                    // Cross-device (EXDEV) or EPERM: fall back to copy + unlink.
                    let cross = err.raw_os_error() == Some(18) || err.kind() == std::io::ErrorKind::PermissionDenied;
                    if !cross {
                        return Err(err.into());
                    }
                    tokio::fs::copy(local_tmp_path, &target).await?;
                    let _ = tokio::fs::remove_file(local_tmp_path).await;
                    Ok(())
                }
            }
        })
    }

    fn read_stream<'a>(&'a self, path: &'a str, range: Option<ByteRange>) -> BoxFuture<'a, anyhow::Result<ByteStream>> {
        Box::pin(async move {
            let target = self.full(path)?;
            let mut file = tokio::fs::File::open(&target).await?;
            match range {
                Some(r) => {
                    file.seek(SeekFrom::Start(r.start)).await?;
                    let len = r.end.saturating_sub(r.start).saturating_add(1);
                    Ok(stream_file(file, Some(len)))
                }
                None => Ok(stream_file(file, None)),
            }
        })
    }

    fn read_buffer<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Bytes>> {
        Box::pin(async move { Ok(Bytes::from(tokio::fs::read(self.full(path)?).await?)) })
    }

    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Option<StatResult>>> {
        // Any failure (missing file, traversal) is "no such object", as upstream.
        Box::pin(async move {
            let Ok(target) = self.full(path) else {
                return Ok(None);
            };
            match tokio::fs::metadata(&target).await {
                Ok(m) => Ok(Some(StatResult { size: m.len() })),
                Err(_) => Ok(None),
            }
        })
    }

    fn delete<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let target = self.full(path)?;
            match tokio::fs::remove_file(&target).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::IsADirectory || target.is_dir() => {
                    match tokio::fs::remove_dir_all(&target).await {
                        Ok(()) => Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(e.into()),
                    }
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    fn delete_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            // A prefix is a directory on this driver, so one recursive remove does it.
            let dir = prefix.trim_end_matches('/');
            if dir.is_empty() {
                bail!("deletePrefix: refusing to delete the storage root");
            }
            let target = self.full(dir)?;
            match tokio::fs::remove_dir_all(&target).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) if target.is_file() => {
                    let _ = e;
                    tokio::fs::remove_file(&target).await?;
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    fn local_path(&self, path: &str) -> anyhow::Result<Option<PathBuf>> {
        Ok(Some(self.full(path)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_stream(data: &'static [u8]) -> ByteStream {
        Box::pin(futures_util::stream::once(async move { Ok(Bytes::from_static(data)) }))
    }

    #[tokio::test]
    async fn write_read_range_stat_delete() {
        let dir = tempfile::tempdir().unwrap();
        let d = LocalDriver::new(dir.path().to_str().unwrap());
        d.write("tag/code.txt", bytes_stream(b"hello world")).await.unwrap();
        assert_eq!(d.stat("tag/code.txt").await.unwrap(), Some(StatResult { size: 11 }));
        assert_eq!(d.read_buffer("tag/code.txt").await.unwrap(), Bytes::from_static(b"hello world"));
        let mut s = d.read_stream("tag/code.txt", Some(ByteRange { start: 6, end: 10 })).await.unwrap();
        let mut got = Vec::new();
        while let Some(c) = s.next().await {
            got.extend_from_slice(&c.unwrap());
        }
        assert_eq!(got, b"world");
        d.delete("tag/code.txt").await.unwrap();
        d.delete("tag/code.txt").await.unwrap();
        assert_eq!(d.stat("tag/code.txt").await.unwrap(), None);
        assert!(d.stat("__healthcheck__").await.unwrap().is_none());
    }

    #[test]
    fn traversal_is_blocked() {
        let d = LocalDriver::new("/srv/storage");
        assert!(d.full("../etc/passwd").is_err());
        assert!(d.full("/etc/passwd").is_err());
        assert_eq!(d.full("a/../b").unwrap(), PathBuf::from("/srv/storage/b"));
        assert_eq!(d.full("").unwrap(), PathBuf::from("/srv/storage"));
    }

    #[tokio::test]
    async fn delete_prefix_refuses_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let d = LocalDriver::new(dir.path().to_str().unwrap());
        assert!(d.delete_prefix("/").await.is_err());
        d.write("derived/u1/a.webp", bytes_stream(b"x")).await.unwrap();
        d.delete_prefix("derived/u1/").await.unwrap();
        assert!(!d.exists("derived/u1/a.webp").await.unwrap());
        d.delete_prefix("derived/missing/").await.unwrap();
    }
}
