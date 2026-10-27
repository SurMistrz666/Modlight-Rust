//! A storage driver that decrypts exactly the objects one request was
//! allowed to read, and passes everything else through.
//!
//! Everything above the storage layer (Range, thumbnails, posters, motion
//! tiles, timelines, conversions, document previews, album ZIPs, ffmpeg
//! workspaces) reads plaintext from it without knowing. A request that was not
//! given a view reads the stored ciphertext, so a gate forgotten somewhere
//! fails closed: garbage, never a file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::future::BoxFuture;

use super::cipher::{CipherHeader, ObjectCipher, collect, decrypt_range_stream, decrypt_stream};
use super::keys::{AccountSecret, open_file_key};
use crate::db::models::Upload;
use crate::state::AppState;
use crate::storage::{ByteRange, ByteStream, StatResult, Storage, StorageDriver};

/// The cipher of one stored object, from its row and the account's key.
pub fn object_cipher(secret: &AccountSecret, storage_key: &str, header: &str) -> anyhow::Result<ObjectCipher> {
    let header = CipherHeader::parse(header)?;
    let key = open_file_key(secret, &header.sealed_key, storage_key)?;
    Ok(ObjectCipher::from_parts(key, header.prefix, header.chunk, header.plain_size))
}

pub struct VaultView {
    inner: Arc<dyn StorageDriver>,
    objects: HashMap<String, Arc<ObjectCipher>>,
}

impl VaultView {
    pub fn new(inner: Arc<dyn StorageDriver>) -> VaultView {
        VaultView { inner, objects: HashMap::new() }
    }

    pub fn add(&mut self, storage_key: &str, cipher: ObjectCipher) {
        self.objects.insert(storage_key.to_string(), Arc::new(cipher));
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

impl StorageDriver for VaultView {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn write<'a>(&'a self, path: &'a str, body: ByteStream) -> BoxFuture<'a, anyhow::Result<()>> {
        self.inner.write(path, body)
    }

    fn ingest_local_file<'a>(&'a self, local_tmp_path: &'a Path, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.inner.ingest_local_file(local_tmp_path, path)
    }

    fn read_stream<'a>(&'a self, path: &'a str, range: Option<ByteRange>) -> BoxFuture<'a, anyhow::Result<ByteStream>> {
        Box::pin(async move {
            let Some(cipher) = self.objects.get(path).cloned() else {
                return self.inner.read_stream(path, range).await;
            };
            let Some(range) = range else {
                let sealed = self.inner.read_stream(path, None).await?;
                return Ok(decrypt_stream(cipher, sealed));
            };
            if cipher.plain_size == 0 || range.start > range.end || range.start >= cipher.plain_size {
                anyhow::bail!("range outside the object");
            }
            let range = ByteRange { start: range.start, end: range.end.min(cipher.plain_size - 1) };
            let (sealed_range, _, _) = cipher.cipher_range(range);
            let sealed = self.inner.read_stream(path, Some(sealed_range)).await?;
            Ok(decrypt_range_stream(cipher, range, sealed))
        })
    }

    fn read_buffer<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Bytes>> {
        Box::pin(async move {
            if !self.objects.contains_key(path) {
                return self.inner.read_buffer(path).await;
            }
            Ok(collect(self.read_stream(path, None).await?).await?)
        })
    }

    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<Option<StatResult>>> {
        Box::pin(async move {
            let stat = self.inner.stat(path).await?;
            Ok(match (stat, self.objects.get(path)) {
                // The size every reader above works in is the plaintext's.
                (Some(_), Some(cipher)) => Some(StatResult { size: cipher.plain_size }),
                (stat, _) => stat,
            })
        })
    }

    fn delete<'a>(&'a self, path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.inner.delete(path)
    }

    fn delete_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.inner.delete_prefix(prefix)
    }

    fn presign_get<'a>(&'a self, path: &'a str, expires: u64) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        if self.objects.contains_key(path) {
            // A presigned URL would hand out the ciphertext.
            return Box::pin(async { Ok(None) });
        }
        self.inner.presign_get(path, expires)
    }

    fn local_path(&self, path: &str) -> anyhow::Result<Option<PathBuf>> {
        if self.objects.contains_key(path) {
            // ffmpeg must read plaintext: the workspace stages it through
            // `read_stream` into its own temporary directory.
            return Ok(None);
        }
        self.inner.local_path(path)
    }
}

/// A state whose storage decrypts the encrypted ones among `uploads` with
/// `secret`. Unchanged when none of them is encrypted. An encrypted upload
/// without a key, or whose key does not open, is an error: the caller's gate
/// should have answered first.
pub fn readable(state: &AppState, secret: Option<&AccountSecret>, uploads: &[&Upload]) -> anyhow::Result<AppState> {
    let encrypted: Vec<&&Upload> = uploads.iter().filter(|u| u.is_encrypted()).collect();
    if encrypted.is_empty() {
        return Ok(state.clone());
    }
    let Some(secret) = secret else {
        anyhow::bail!("encrypted file without its key");
    };
    let inner = state.storage.get()?;
    let mut view = VaultView::new(inner);
    for u in encrypted {
        let header = u.encryption.as_deref().unwrap_or_default();
        view.add(&u.storage_key, object_cipher(secret, &u.storage_key, header)?);
    }
    let mut out = state.clone();
    out.storage = Storage::with_driver(Arc::new(view));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalDriver;
    use crate::vault::cipher::{CHUNK_SIZE, encrypt_stream};
    use crate::vault::keys::seal_file_key;

    async fn stored(driver: &LocalDriver, key: &str, plain: &[u8], secret: &AccountSecret) -> String {
        let cipher = Arc::new(ObjectCipher::generate(plain.len() as u64));
        let sealed_key = seal_file_key(&secret.public_key(), cipher.key(), key).unwrap();
        let body = crate::storage::bytes_stream(Bytes::copy_from_slice(plain));
        driver.write(key, encrypt_stream(cipher.clone(), body)).await.unwrap();
        CipherHeader { chunk: CHUNK_SIZE, plain_size: plain.len() as u64, prefix: cipher.prefix, sealed_key }.encode()
    }

    #[tokio::test]
    async fn reads_decrypt_and_everything_else_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        let driver = Arc::new(LocalDriver::new(dir.path().to_str().unwrap()));
        let secret = AccountSecret::generate();
        let plain: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let header = stored(&driver, "t/a.bin", &plain, &secret).await;
        driver.write("t/plain.txt", crate::storage::bytes_stream(Bytes::from_static(b"hello"))).await.unwrap();

        let mut view = VaultView::new(driver.clone());
        view.add("t/a.bin", object_cipher(&secret, "t/a.bin", &header).unwrap());
        // Stat is the plaintext's size; the stored object is larger.
        assert_eq!(view.stat("t/a.bin").await.unwrap().unwrap().size, plain.len() as u64);
        assert!(driver.stat("t/a.bin").await.unwrap().unwrap().size > plain.len() as u64);
        assert_eq!(view.read_buffer("t/a.bin").await.unwrap().to_vec(), plain);
        let r = ByteRange { start: 65_530, end: 131_080 };
        let part = collect(view.read_stream("t/a.bin", Some(r)).await.unwrap()).await.unwrap();
        assert_eq!(part.to_vec(), plain[65_530..=131_080].to_vec());
        // A range running past the end is cut at the end.
        let tail =
            collect(view.read_stream("t/a.bin", Some(ByteRange { start: 199_990, end: 999_999 })).await.unwrap())
                .await
                .unwrap();
        assert_eq!(tail.to_vec(), plain[199_990..].to_vec());
        assert!(view.local_path("t/a.bin").unwrap().is_none());
        assert!(view.local_path("t/plain.txt").unwrap().is_some());
        assert_eq!(view.read_buffer("t/plain.txt").await.unwrap().as_ref(), b"hello");
        // Without the view the bytes are ciphertext.
        assert_ne!(driver.read_buffer("t/a.bin").await.unwrap().to_vec(), plain);
        // Another account's key does not open the record.
        assert!(object_cipher(&AccountSecret::generate(), "t/a.bin", &header).is_err());
        // Nor does the record of another object.
        assert!(object_cipher(&secret, "t/b.bin", &header).is_err());
    }
}
