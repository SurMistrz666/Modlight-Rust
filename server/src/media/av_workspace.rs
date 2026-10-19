//! Port of `lib/media/av-workspace.ts`: a real path for ffmpeg to read, and
//! a private directory for it to write into.
//!
//! ffmpeg wants files, not buffers - which is a feature here, not a limitation.
//! The image path reads whole uploads into memory and is capped at 32 MB because
//! of it; ffmpeg streams from disk, so a 400 MB video costs the same resident
//! memory as a 4 MB one (measured: 57 MB peak for a 37 MB remux).
//!
//! Two jobs:
//!
//!  1. INPUT. The local driver already has the file on a filesystem, so its path
//!     is handed over as-is - no copy, no temp space, no second read of 400 MB.
//!     Any other driver has to stage a copy, and says so by paying for it.
//!
//!  2. OUTPUT. Every job gets its own directory, and it is removed whatever
//!     happens. This is also a security boundary: ffmpeg writes only inside it,
//!     so a target name can never turn into a path, and a killed process cannot
//!     leave a half-written file where a later request might serve it.
//!
//! "Whatever happens" includes a dropped future here: the directory also goes
//! when the workspace is dropped without [`AvWorkspace::dispose`].

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rand::Rng;
use tokio::io::AsyncWriteExt;

use crate::config::Config;
use crate::storage::{ByteStream, StorageDriver};

#[derive(Debug)]
pub struct AvWorkspace {
    /// Absolute path ffmpeg reads from.
    pub input: PathBuf,
    dir: Option<PathBuf>,
}

impl AvWorkspace {
    /// Absolute path inside the private directory for ffmpeg to write to.
    pub fn output(&self, name: &str) -> anyhow::Result<PathBuf> {
        let dir = self.dir.as_ref().ok_or_else(|| anyhow::anyhow!("workspace already disposed"))?;
        Ok(dir.join(sanitize_name(name)?))
    }

    /// Removes the scratch directory, and the staged input if there was one.
    pub async fn dispose(mut self) {
        if let Some(dir) = self.dir.take() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    }

    /// The private directory (tests, diagnostics).
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }
}

impl Drop for AvWorkspace {
    fn drop(&mut self) {
        if let Some(dir) = self.dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// What `openAvWorkspace` answers: a workspace, or why there is none.
#[derive(Debug)]
pub enum OpenedWorkspace {
    Ready(AvWorkspace),
    Missing,
    TooLarge { size: u64 },
}

/// Where scratch directories go. The uploads temp dir already exists and is ours.
fn scratch_root(config: &Config) -> PathBuf {
    let configured = &config.uploads.tmp_path;
    if configured.is_empty() {
        return std::env::temp_dir();
    }
    let p = Path::new(configured);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(p)
    }
}

/// `mkdtemp(prefix)`: six random alphanumerics, created exclusively.
async fn mkdtemp(root: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    loop {
        let suffix: String =
            rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(6).map(char::from).collect();
        let dir = root.join(format!("{prefix}{suffix}"));
        match tokio::fs::create_dir(&dir).await {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Open a workspace for one job.
///
/// `max_bytes` is checked against the object's real size before anything is
/// staged, so an oversized file costs a stat rather than a copy.
pub async fn open_av_workspace(
    storage: &dyn StorageDriver,
    config: &Config,
    storage_key: &str,
    max_bytes: u64,
) -> anyhow::Result<OpenedWorkspace> {
    let Some(info) = storage.stat(storage_key).await? else {
        return Ok(OpenedWorkspace::Missing);
    };
    if info.size > max_bytes {
        return Ok(OpenedWorkspace::TooLarge { size: info.size });
    }

    // The parent must exist for mkdtemp. The image creates data/tmp in the
    // Dockerfile, but a fresh clone has no data/ at all, and every ffmpeg path
    // (posters, motion previews, timeline sheets) failed there with ENOENT.
    let root = scratch_root(config);
    tokio::fs::create_dir_all(&root).await?;
    let dir = mkdtemp(&root, "av-").await?;
    // Owns the directory from here: an error below removes it on drop.
    let mut ws = AvWorkspace { input: PathBuf::new(), dir: Some(dir.clone()) };

    // The bytes may already be on a filesystem ffmpeg can read, in which case
    // copying them would double the I/O of every conversion for no gain. The
    // driver answers that; asking for a concrete type would put an
    // implementation back in the middle of an abstraction built to avoid it.
    //
    // No existence check here: `storage.stat()` above already returned this
    // object's size, so a second one would re-ask a question we have the
    // answer to. A row whose bytes vanish between the two would fail at
    // ffprobe, which reports it as an unreadable file.
    match storage.local_path(storage_key)? {
        Some(direct) => ws.input = direct,
        None => {
            let staged = dir.join("input");
            let mut file = tokio::fs::File::create(&staged).await?;
            let mut stream = storage.read_stream(storage_key, None).await?;
            while let Some(chunk) = stream.next().await {
                file.write_all(&chunk?).await?;
            }
            file.flush().await?;
            ws.input = staged;
        }
    }
    Ok(OpenedWorkspace::Ready(ws))
}

/// Output names are built from our own target table, never from a request - but
/// the check is cheap and this is the one place a path could be formed, so it is
/// enforced rather than assumed.
fn sanitize_name(name: &str) -> anyhow::Result<&str> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if !first_ok || !rest_ok {
        anyhow::bail!("Refusing unsafe output name: {name}");
    }
    Ok(name)
}

/// A file stream that owns the workspace its file lives in.
struct DisposingStream {
    inner: ByteStream,
    _workspace: AvWorkspace,
}

impl Stream for DisposingStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Stream a finished output file, then drop the workspace it lived in (when
/// the stream ends or is dropped).
pub async fn read_and_dispose(path: &Path, ws: AvWorkspace) -> std::io::Result<ByteStream> {
    let file = tokio::fs::File::open(path).await?;
    let inner: ByteStream = Box::pin(tokio_util::io::ReaderStream::new(file));
    Ok(Box::pin(DisposingStream { inner, _workspace: ws }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::storage::{ByteRange, LocalDriver, StatResult};
    use futures_util::future::BoxFuture;

    fn config_with_tmp(tmp: &Path) -> Config {
        let vars = [("UPLOAD_TMP_PATH".to_string(), tmp.to_string_lossy().into_owned())].into_iter().collect();
        Config::from_env_map(&vars).expect("config")
    }

    /// A driver with no local paths, standing in for S3.
    struct RemoteOnly(Arc<LocalDriver>);

    impl StorageDriver for RemoteOnly {
        fn name(&self) -> &'static str {
            "s3"
        }
        fn write<'a>(&'a self, path: &'a str, body: ByteStream) -> BoxFuture<'a, anyhow::Result<()>> {
            self.0.write(path, body)
        }
        fn ingest_local_file<'a>(&'a self, p: &'a Path, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
            self.0.ingest_local_file(p, path)
        }
        fn read_stream<'a>(
            &'a self,
            path: &'a str,
            range: Option<ByteRange>,
        ) -> BoxFuture<'a, anyhow::Result<ByteStream>> {
            self.0.read_stream(path, range)
        }
        fn read_buffer<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Bytes>> {
            self.0.read_buffer(path)
        }
        fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Option<StatResult>>> {
            self.0.stat(path)
        }
        fn delete<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
            self.0.delete(path)
        }
        fn delete_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
            self.0.delete_prefix(prefix)
        }
    }

    #[tokio::test]
    async fn hands_a_local_file_over_as_is_and_stages_a_remote_one() {
        let root = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("u")).unwrap();
        std::fs::write(root.path().join("u/a.mp4"), b"0123456789").unwrap();
        let local = Arc::new(LocalDriver::new(root.path().to_str().unwrap()));
        let config = config_with_tmp(tmp.path());

        let OpenedWorkspace::Ready(ws) = open_av_workspace(&*local, &config, "u/a.mp4", 100).await.unwrap() else {
            panic!("expected a workspace");
        };
        assert_eq!(ws.input, local.full("u/a.mp4").unwrap());
        let dir = ws.dir().unwrap().to_path_buf();
        assert!(dir.file_name().unwrap().to_string_lossy().starts_with("av-"));
        assert_eq!(ws.output("out.webp").unwrap(), dir.join("out.webp"));
        ws.dispose().await;
        assert!(!dir.exists());

        let remote = RemoteOnly(local.clone());
        let OpenedWorkspace::Ready(ws) = open_av_workspace(&remote, &config, "u/a.mp4", 100).await.unwrap() else {
            panic!("expected a workspace");
        };
        assert_eq!(std::fs::read(&ws.input).unwrap(), b"0123456789");
        let dir = ws.dir().unwrap().to_path_buf();
        // Dropped without dispose: still removed.
        drop(ws);
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn answers_missing_and_too_large_without_staging_anything() {
        let root = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("big"), vec![0u8; 50]).unwrap();
        let local = LocalDriver::new(root.path().to_str().unwrap());
        let config = config_with_tmp(tmp.path());
        assert!(matches!(open_av_workspace(&local, &config, "nope", 100).await.unwrap(), OpenedWorkspace::Missing));
        assert!(matches!(
            open_av_workspace(&local, &config, "big", 10).await.unwrap(),
            OpenedWorkspace::TooLarge { size: 50 }
        ));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn refuses_an_output_name_that_could_become_a_path() {
        assert!(sanitize_name("out.mp4").is_ok());
        assert!(sanitize_name("Poster_1-a.webp").is_ok());
        for bad in ["", ".hidden", "../x", "a/b", "-x", "a b", "ä.mp4"] {
            assert!(sanitize_name(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn streams_the_output_and_then_removes_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("in"), b"x").unwrap();
        let local = LocalDriver::new(root.path().to_str().unwrap());
        let config = config_with_tmp(tmp.path());
        let OpenedWorkspace::Ready(ws) = open_av_workspace(&local, &config, "in", 10).await.unwrap() else { panic!() };
        let out = ws.output("out.bin").unwrap();
        std::fs::write(&out, b"result").unwrap();
        let dir = ws.dir().unwrap().to_path_buf();
        let mut stream = read_and_dispose(&out, ws).await.unwrap();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(body, b"result");
        drop(stream);
        assert!(!dir.exists());
    }
}
