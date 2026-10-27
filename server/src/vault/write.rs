//! Encrypting new objects for an account. Only the public key is needed, so
//! uploads from the panel, ShareX and the desktop app keep working without
//! anybody typing a password.

use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;

use super::VaultState;
use super::cipher::{CipherHeader, ObjectCipher, encrypt_stream};
use super::keys::{seal_file_key, server_key, trusted_public_key};
use super::store;
use crate::db::models::FileVault;
use crate::storage::{ByteStream, StorageDriver, bytes_stream};

/// Encrypts for one account (its vault is `encrypting` or `on`).
#[derive(Debug, Clone)]
pub struct Sealer {
    pub user_id: String,
    public_key: [u8; 32],
}

impl Sealer {
    /// From a vault row whose public key carries the server's seal.
    pub fn from_vault(auth_secret: &str, vault: &FileVault) -> anyhow::Result<Sealer> {
        let server = server_key(auth_secret)?;
        let public_key = trusted_public_key(&server, &vault.user_id, &vault.public_key, &vault.public_key_mac)?;
        Ok(Sealer { user_id: vault.user_id.clone(), public_key })
    }

    /// Encrypt `plain` (exactly `plain_size` bytes) into `storage` at `key`
    /// and verify what landed; returns the row's `encryption` header. On
    /// failure nothing is left at `key`.
    pub async fn write_stream(
        &self,
        storage: &dyn StorageDriver,
        key: &str,
        plain: ByteStream,
        plain_size: u64,
    ) -> anyhow::Result<String> {
        let cipher = Arc::new(ObjectCipher::generate(plain_size));
        let sealed_key = seal_file_key(&self.public_key, cipher.key(), key)?;
        let header = CipherHeader { chunk: cipher.chunk, plain_size, prefix: cipher.prefix, sealed_key }.encode();
        let expected = cipher.cipher_size();
        let written = storage.write(key, encrypt_stream(cipher, plain)).await;
        let verified = match written {
            Ok(()) => matches!(storage.stat(key).await, Ok(Some(info)) if info.size == expected),
            Err(err) => {
                let _ = storage.delete(key).await;
                return Err(err);
            }
        };
        if !verified {
            let _ = storage.delete(key).await;
            anyhow::bail!("encrypted object did not arrive whole: {key}");
        }
        Ok(header)
    }

    pub async fn write_bytes(&self, storage: &dyn StorageDriver, key: &str, bytes: Bytes) -> anyhow::Result<String> {
        let size = bytes.len() as u64;
        self.write_stream(storage, key, bytes_stream(bytes), size).await
    }

    /// Encrypt a local file (a finished upload's temp file, an ffmpeg output).
    pub async fn write_local_file(
        &self,
        storage: &dyn StorageDriver,
        key: &str,
        path: &Path,
    ) -> anyhow::Result<String> {
        let size = tokio::fs::metadata(path).await?.len();
        let file = tokio::fs::File::open(path).await?;
        let body: ByteStream = Box::pin(tokio_util::io::ReaderStream::with_capacity(file, 256 * 1024));
        self.write_stream(storage, key, body, size).await
    }
}

/// The sealer of `user_id` when new files of that account must be encrypted
/// (vault `encrypting` or `on`); None otherwise.
pub async fn sealer_for(db: &sqlx::PgPool, auth_secret: &str, user_id: &str) -> anyhow::Result<Option<Sealer>> {
    let Some(vault) = store::load(db, user_id).await? else {
        return Ok(None);
    };
    if !store::state_of(&vault).is_some_and(VaultState::is_gated) {
        return Ok(None);
    }
    Ok(Some(Sealer::from_vault(auth_secret, &vault)?))
}

/// The writer's check inside its own transaction, just before the row goes in
/// (docs/ENCRYPTION.md section 7): `encrypted` says what the writer stored.
///
/// - an encrypted object needs a vault that still exists (switching off may
///   have finished in the meantime: then the write is refused, and retried by
///   the client);
/// - a plaintext object written into a vault that is already `on` sends the
///   vault back to `encrypting`, so the pass picks the row up.
pub async fn confirm_vault_for_write(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_id: &str,
    encrypted: bool,
) -> anyhow::Result<()> {
    let vault = store::load_shared(&mut **tx, owner_id).await?;
    match (vault, encrypted) {
        (None, true) => anyhow::bail!("file encryption was switched off during the upload; try again"),
        (Some(v), false) if store::state_of(&v) == Some(VaultState::On) => {
            sqlx::query(
                r#"update "file_vaults" set "state" = 'encrypting', "updated_at" = now() where "user_id" = $1 and "state" = 'on'"#,
            )
            .bind(owner_id)
            .execute(&mut **tx)
            .await?;
            super::pass::kick();
        }
        _ => {}
    }
    Ok(())
}
