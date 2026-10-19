//! Port of `@tus/file-store` 2.1.0 (`FileStore`) with `FileKvStore` of
//! `@tus/utils` 0.7.0 as its config store: the payload of an upload is the
//! file `<directory>/<id>`, its record the JSON file `<directory>/<id>.json`
//! (`JSON.stringify(upload)` at creation, so the TypeScript server and this
//! one read each other's records).

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use super::store::{
    DataStore, EXT_CREATION, EXT_CREATION_DEFER_LENGTH, EXT_CREATION_WITH_UPLOAD, EXT_EXPIRATION, EXT_TERMINATION,
    StoreError, Upload, UploadStorage, errors,
};
use crate::storage::ByteStream;

const EXTENSIONS: &[&str] =
    &[EXT_CREATION, EXT_CREATION_WITH_UPLOAD, EXT_CREATION_DEFER_LENGTH, EXT_TERMINATION, EXT_EXPIRATION];

pub struct FileStore {
    /// The directory as configured (`TUS_TMP_PATH`), joined the way Node does.
    directory: String,
    expiration_ms: u64,
}

impl FileStore {
    /// `new FileStore({ directory, expirationPeriodInMilliseconds })`; the
    /// directory is created (mode 0777 before the umask) like the constructor
    /// does.
    pub fn new(directory: &str, expiration_ms: u64) -> FileStore {
        let _ = std::fs::create_dir_all(directory);
        FileStore { directory: directory.to_string(), expiration_ms }
    }

    /// `path.join(directory, id)` (normalised, as Node joins).
    fn joined(&self, id: &str) -> PathBuf {
        PathBuf::from(node_path_join(&self.directory, id))
    }

    /// `${directory}/${id}` (plain concatenation, as getUpload/remove use).
    fn concat(&self, id: &str) -> String {
        format!("{}/{id}", self.directory)
    }

    /// `FileKvStore.resolve(key)`: `path.resolve(directory, key + ".json")`.
    fn record_path(&self, id: &str) -> PathBuf {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        crate::storage::local::resolve_lexically(&cwd, &Path::new(&self.directory).join(format!("{id}.json")))
    }

    async fn get_record(&self, id: &str) -> Option<Upload> {
        let text = tokio::fs::read_to_string(self.record_path(id)).await.ok()?;
        serde_json::from_str(&text).ok()
    }

    async fn set_record(&self, id: &str, upload: &Upload) -> Result<(), StoreError> {
        let json = serde_json::to_string(upload).map_err(|e| StoreError::Other(e.into()))?;
        tokio::fs::write(self.record_path(id), json).await?;
        Ok(())
    }

    async fn delete_record_file(&self, id: &str) -> Result<(), StoreError> {
        tokio::fs::remove_file(self.record_path(id)).await?;
        Ok(())
    }

    /// `FileKvStore.list()`: payload names that have a `.json` sibling, by
    /// sorting the directory and pairing each entry with the next.
    async fn list(&self) -> Result<Vec<String>, StoreError> {
        let mut dir = tokio::fs::read_dir(&self.directory).await?;
        let mut files = Vec::new();
        while let Some(entry) = dir.next_entry().await? {
            files.push(entry.file_name().to_string_lossy().into_owned());
        }
        files.sort();
        let name = |f: &str| f.strip_suffix(".json").unwrap_or(f).to_string();
        Ok(files
            .iter()
            .enumerate()
            .filter(|(i, f)| *i + 1 < files.len() && name(f) == name(&files[i + 1]))
            .map(|(_, f)| f.clone())
            .collect())
    }
}

/// Node's `path.join(a, b)` for the two-segment case it is used with here.
pub fn node_path_join(a: &str, b: &str) -> String {
    let joined = if a.is_empty() {
        b.to_string()
    } else if b.is_empty() {
        a.to_string()
    } else {
        format!("{a}/{b}")
    };
    let absolute = joined.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let body = parts.join("/");
    match (absolute, body.is_empty()) {
        (true, _) => format!("/{body}"),
        (false, true) => ".".to_string(),
        (false, false) => body,
    }
}

impl DataStore for FileStore {
    fn extensions(&self) -> &[&'static str] {
        EXTENSIONS
    }

    fn create(&self, mut upload: Upload) -> BoxFuture<'_, Result<Upload, StoreError>> {
        Box::pin(async move {
            let path = self.joined(&upload.id);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&path, b"").await?;
            // The record is written BEFORE storage is set, as upstream.
            self.set_record(&upload.id.clone(), &upload).await?;
            upload.storage =
                Some(UploadStorage { kind: "file".into(), path: path.to_string_lossy().into_owned(), bucket: None });
            Ok(upload)
        })
    }

    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            if tokio::fs::remove_file(self.concat(id)).await.is_err() {
                return Err(errors::FILE_NOT_FOUND.into());
            }
            self.delete_record_file(id).await
        })
    }

    fn write<'a>(&'a self, mut body: ByteStream, id: &'a str, offset: u64) -> BoxFuture<'a, Result<u64, StoreError>> {
        Box::pin(async move {
            // createWriteStream(path, { flags: "r+", start: offset })
            let file = tokio::fs::OpenOptions::new().read(true).write(true).open(self.joined(id)).await;
            let Ok(mut file) = file else {
                return Err(errors::FILE_WRITE_ERROR.into());
            };
            if file.seek(std::io::SeekFrom::Start(offset)).await.is_err() {
                return Err(errors::FILE_WRITE_ERROR.into());
            }
            let mut received: u64 = 0;
            while let Some(chunk) = body.next().await {
                let Ok(chunk) = chunk else {
                    let _ = file.flush().await;
                    return Err(errors::FILE_WRITE_ERROR.into());
                };
                if file.write_all(&chunk).await.is_err() {
                    return Err(errors::FILE_WRITE_ERROR.into());
                }
                received += chunk.len() as u64;
            }
            if file.flush().await.is_err() {
                return Err(errors::FILE_WRITE_ERROR.into());
            }
            Ok(offset + received)
        })
    }

    fn get_upload<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Upload, StoreError>> {
        Box::pin(async move {
            let Some(record) = self.get_record(id).await else {
                return Err(errors::FILE_NOT_FOUND.into());
            };
            let file_path = self.concat(id);
            match tokio::fs::metadata(&file_path).await {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(errors::FILE_NO_LONGER_EXISTS.into()),
                Err(e) => Err(e.into()),
                Ok(m) if m.is_dir() => Err(errors::FILE_NOT_FOUND.into()),
                Ok(m) => Ok(Upload {
                    id: id.to_string(),
                    size: record.size,
                    offset: m.len(),
                    metadata: record.metadata,
                    creation_date: record
                        .creation_date
                        .or_else(|| Some(crate::db::types::JsDate::now().to_iso_string())),
                    storage: Some(UploadStorage { kind: "file".into(), path: file_path, bucket: None }),
                }),
            }
        })
    }

    fn declare_upload_length<'a>(&'a self, id: &'a str, upload_length: u64) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let Some(mut record) = self.get_record(id).await else {
                return Err(errors::FILE_NOT_FOUND.into());
            };
            record.size = Some(upload_length);
            self.set_record(id, &record).await
        })
    }

    fn delete_expired(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
            let now = crate::db::types::JsDate::now().millis();
            let mut removed = 0u64;
            for id in self.list().await? {
                let Some(info) = self.get_record(&id).await else {
                    continue;
                };
                // `info.size !== info.offset`, on the record as written at creation.
                if self.expiration_ms == 0 || info.size == Some(info.offset) {
                    continue;
                }
                let Some(created) = info.creation_millis() else {
                    continue;
                };
                if now > created + self.expiration_ms as i64 {
                    match self.remove(&id).await {
                        Ok(()) => removed += 1,
                        Err(StoreError::Tus(e)) if e == errors::FILE_NO_LONGER_EXISTS => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            Ok(removed)
        })
    }

    fn get_expiration(&self) -> u64 {
        self.expiration_ms
    }

    fn directory(&self) -> Option<&Path> {
        Some(Path::new(&self.directory))
    }

    fn delete_record<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move { self.delete_record_file(id).await })
    }

    fn supports_read(&self) -> bool {
        true
    }

    fn read<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<ByteStream, StoreError>> {
        Box::pin(async move {
            let file = tokio::fs::File::open(self.joined(id)).await?;
            let stream: ByteStream = Box::pin(tokio_util::io::ReaderStream::new(file));
            Ok(stream)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_like_node() {
        assert_eq!(node_path_join("./data/tus", "abc"), "data/tus/abc");
        assert_eq!(node_path_join("/app/data/tus", "abc"), "/app/data/tus/abc");
        assert_eq!(node_path_join("data/tus/", "a/b"), "data/tus/a/b");
        assert_eq!(node_path_join("/x", "../y"), "/y");
    }

    #[tokio::test]
    async fn create_write_get_remove_and_expire() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_str().unwrap().to_string();
        let store = FileStore::new(&d, 24 * 3600 * 1000);
        let upload = Upload::new("abc".into(), Some(5), 0, None);
        let created = store.create(upload).await.unwrap();
        assert_eq!(created.storage.unwrap().kind, "file");
        let body: ByteStream = Box::pin(futures_util::stream::once(async { Ok(bytes::Bytes::from_static(b"hel")) }));
        assert_eq!(store.write(body, "abc", 0).await.unwrap(), 3);
        let got = store.get_upload("abc").await.unwrap();
        assert_eq!((got.offset, got.size), (3, Some(5)));
        assert_eq!(store.list().await.unwrap(), vec!["abc".to_string()]);
        assert_eq!(store.delete_expired().await.unwrap(), 0);
        // An old record expires.
        let mut old = Upload::new("old".into(), Some(5), 0, None);
        old.creation_date = Some("2020-01-01T00:00:00.000Z".into());
        store.create(old).await.unwrap();
        assert_eq!(store.delete_expired().await.unwrap(), 1);
        store.remove("abc").await.unwrap();
        assert!(matches!(store.remove("abc").await, Err(StoreError::Tus(e)) if e == errors::FILE_NOT_FOUND));
        assert!(matches!(store.get_upload("abc").await, Err(StoreError::Tus(e)) if e == errors::FILE_NOT_FOUND));
    }
}
