//! The key material of a file vault: the account's X25519 key pair, the
//! private key wrapped under the file password (Argon2id) and under each
//! unlock grant, file keys sealed to the public key, and the MAC that ties a
//! stored public key to the server key. See docs/ENCRYPTION.md section 2.
//!
//! Every secret lives in a zeroizing container; every check of a secret is
//! an AEAD tag check (constant time). The password is never compared with
//! anything.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};
use zeroize::Zeroizing;

/// Context strings: one per use, never shared.
const WRAP_AAD: &[u8] = b"modlight vault key v1";
const FILE_KEY_INFO: &[u8] = b"modlight file key v1";
const GRANT_INFO: &str = "modlight vault grant v1";
const PUBLIC_KEY_MAC: &str = "modlight vault pk v1";

const NONCE_SIZE: usize = 24;
const TAG_SIZE: usize = 16;

/// Argon2id parameters, stored with each record so they can be raised later
/// without touching existing vaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    /// RFC 9106's second recommended option, with one lane: 64 MiB, 3 passes.
    pub const DEFAULT: KdfParams = KdfParams { memory_kib: 64 * 1024, iterations: 3, parallelism: 1 };

    /// Cheap parameters for tests only (Argon2's own minimums).
    pub const TEST: KdfParams = KdfParams { memory_kib: 8, iterations: 1, parallelism: 1 };

    /// Refuse parameters no vault of ours was made with: a record must not be
    /// able to make the server allocate gigabytes or spin for minutes.
    pub fn sane(&self) -> bool {
        (8..=1024 * 1024).contains(&self.memory_kib)
            && (1..=10).contains(&self.iterations)
            && (1..=8).contains(&self.parallelism)
    }
}

/// Why a key operation failed. `Wrong` is the only one a user can cause.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The password, the grant secret or the record does not open the key.
    #[error("wrong password or damaged record")]
    Wrong,
    #[error("unsupported key derivation parameters")]
    BadParams,
    #[error("malformed key record")]
    Malformed,
    #[error("the account's public key does not carry the server's seal")]
    UntrustedPublicKey,
}

/// The account's private key. Wiped on drop (dalek's `StaticSecret`).
pub struct AccountSecret(StaticSecret);

impl AccountSecret {
    pub fn generate() -> AccountSecret {
        AccountSecret(StaticSecret::random_from_rng(OsRng))
    }

    pub fn public_key(&self) -> [u8; 32] {
        PublicKey::from(&self.0).to_bytes()
    }

    fn bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    fn from_bytes(bytes: Zeroizing<Vec<u8>>) -> Result<AccountSecret, KeyError> {
        let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| KeyError::Malformed)?;
        let arr = Zeroizing::new(arr);
        Ok(AccountSecret(StaticSecret::from(*arr)))
    }
}

impl std::fmt::Debug for AccountSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountSecret(..)")
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}

fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; NONCE_SIZE] = random();
    let aead = XChaCha20Poly1305::new(key.into());
    let ct = aead.encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad }).expect("XChaCha20 seals any length");
    let mut out = Vec::with_capacity(NONCE_SIZE + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

fn open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    if sealed.len() < NONCE_SIZE + TAG_SIZE {
        return Err(KeyError::Malformed);
    }
    let (nonce, ct) = sealed.split_at(NONCE_SIZE);
    let aead = XChaCha20Poly1305::new(key.into());
    aead.decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad }).map(Zeroizing::new).map_err(|_| KeyError::Wrong)
}

fn wrap_aad(user_id: &str, public_key: &[u8]) -> Vec<u8> {
    let mut aad = WRAP_AAD.to_vec();
    aad.push(b'|');
    aad.extend_from_slice(user_id.as_bytes());
    aad.push(b'|');
    aad.extend_from_slice(public_key);
    aad
}

// ── The file password ───────────────────────────────────────────────────────

/// What a password field may hold: NFKC (so "ą" typed on any keyboard is the
/// same password), as Better Auth normalises sign-in passwords.
pub fn normalize_password(password: &str) -> Zeroizing<String> {
    Zeroizing::new(password.nfkc().collect())
}

/// Argon2id(password, salt) -> 32 bytes. Blocking: call through
/// [`derive_password_key`], which bounds how many run at once.
pub fn password_key(password: &str, salt: &[u8], params: KdfParams) -> Result<Zeroizing<[u8; 32]>, KeyError> {
    if !params.sane() || salt.len() < 16 {
        return Err(KeyError::BadParams);
    }
    let p = Params::new(params.memory_kib, params.iterations, params.parallelism, Some(32))
        .map_err(|_| KeyError::BadParams)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let normalized = normalize_password(password);
    let mut out = Zeroizing::new([0u8; 32]);
    argon.hash_password_into(normalized.as_bytes(), salt, out.as_mut()).map_err(|_| KeyError::BadParams)?;
    Ok(out)
}

/// At most this many Argon2 derivations run at once in the process (each
/// takes 64 MiB and a fraction of a second); the rest wait their turn.
const KDF_CONCURRENCY: usize = 2;

fn kdf_permits() -> &'static tokio::sync::Semaphore {
    static PERMITS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    PERMITS.get_or_init(|| tokio::sync::Semaphore::new(KDF_CONCURRENCY))
}

/// [`password_key`] on the blocking pool, behind the process-wide limit.
pub async fn derive_password_key(
    password: &str,
    salt: Vec<u8>,
    params: KdfParams,
) -> anyhow::Result<Result<Zeroizing<[u8; 32]>, KeyError>> {
    let _permit = kdf_permits().acquire().await?;
    let password = Zeroizing::new(password.to_string());
    Ok(tokio::task::spawn_blocking(move || password_key(&password, &salt, params)).await?)
}

/// A new random Argon2 salt.
pub fn new_salt() -> Vec<u8> {
    random::<16>().to_vec()
}

/// The private key sealed under the password key.
pub fn wrap_with_password_key(
    secret: &AccountSecret,
    password_key: &[u8; 32],
    user_id: &str,
    public_key: &[u8],
) -> Vec<u8> {
    seal(password_key, &wrap_aad(user_id, public_key), secret.bytes().as_ref())
}

/// Open the private key with the password key. A wrong password is
/// [`KeyError::Wrong`]; a key that opens but does not match the stored public
/// key is refused too (a damaged or swapped record).
pub fn unwrap_with_password_key(
    wrapped: &[u8],
    password_key: &[u8; 32],
    user_id: &str,
    public_key: &[u8],
) -> Result<AccountSecret, KeyError> {
    let plain = open(password_key, &wrap_aad(user_id, public_key), wrapped)?;
    let secret = AccountSecret::from_bytes(plain)?;
    if secret.public_key().as_slice() != public_key {
        return Err(KeyError::Wrong);
    }
    Ok(secret)
}

// ── The server key and the public key's seal ────────────────────────────────

/// The vault's server key: HKDF of SETTINGS_ENCRYPTION_KEY or AUTH_SECRET.
pub fn server_key(auth_secret: &str) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    Ok(Zeroizing::new(crate::secret_box::derive_key(auth_secret, crate::secret_box::SecretPurpose::Vault)?))
}

fn public_key_mac_with(server_key: &[u8; 32], user_id: &str, public_key: &[u8]) -> Hmac<Sha256> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(server_key).expect("any key length");
    mac.update(PUBLIC_KEY_MAC.as_bytes());
    mac.update(b"|");
    mac.update(user_id.as_bytes());
    mac.update(b"|");
    mac.update(public_key);
    mac
}

/// HMAC that only the server can make: a database writer cannot swap an
/// account's public key for one of their own and collect its future uploads.
pub fn public_key_mac(server_key: &[u8; 32], user_id: &str, public_key: &[u8]) -> Vec<u8> {
    public_key_mac_with(server_key, user_id, public_key).finalize().into_bytes().to_vec()
}

/// The stored public key, if its seal is the server's (constant time).
pub fn trusted_public_key(
    server_key: &[u8; 32],
    user_id: &str,
    public_key: &[u8],
    mac: &[u8],
) -> Result<[u8; 32], KeyError> {
    public_key_mac_with(server_key, user_id, public_key).verify_slice(mac).map_err(|_| KeyError::UntrustedPublicKey)?;
    public_key.try_into().map_err(|_| KeyError::Malformed)
}

// ── File keys ───────────────────────────────────────────────────────────────

fn file_key_aad(storage_key: &str) -> Vec<u8> {
    let mut aad = FILE_KEY_INFO.to_vec();
    aad.push(b'|');
    aad.extend_from_slice(storage_key.as_bytes());
    aad
}

fn file_wrap_key(shared: &[u8; 32], eph: &[u8; 32], pk: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(eph);
    salt[32..].copy_from_slice(pk);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(FILE_KEY_INFO, out.as_mut()).expect("32 bytes is a valid HKDF length");
    out
}

/// Seal `file_key` to the account's public key, bound to the object it
/// encrypts (`storage_key`): `eph_pk || nonce || ciphertext`.
pub fn seal_file_key(public_key: &[u8; 32], file_key: &[u8; 32], storage_key: &str) -> Result<Vec<u8>, KeyError> {
    let pk = PublicKey::from(*public_key);
    let eph = EphemeralSecret::random_from_rng(OsRng);
    let eph_pk = PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&pk);
    if !shared.was_contributory() {
        return Err(KeyError::Malformed);
    }
    let wrap = file_wrap_key(shared.as_bytes(), &eph_pk, public_key);
    let mut out = eph_pk.to_vec();
    out.extend_from_slice(&seal(&wrap, &file_key_aad(storage_key), file_key));
    Ok(out)
}

/// Open a sealed file key with the account's private key.
pub fn open_file_key(
    secret: &AccountSecret,
    sealed: &[u8],
    storage_key: &str,
) -> Result<Zeroizing<[u8; 32]>, KeyError> {
    if sealed.len() != 32 + NONCE_SIZE + 32 + TAG_SIZE {
        return Err(KeyError::Malformed);
    }
    let (eph, rest) = sealed.split_at(32);
    let eph: [u8; 32] = eph.try_into().map_err(|_| KeyError::Malformed)?;
    let shared = secret.0.diffie_hellman(&PublicKey::from(eph));
    if !shared.was_contributory() {
        return Err(KeyError::Malformed);
    }
    let wrap = file_wrap_key(shared.as_bytes(), &eph, &secret.public_key());
    let plain = open(&wrap, &file_key_aad(storage_key), rest)?;
    let key: [u8; 32] = plain.as_slice().try_into().map_err(|_| KeyError::Malformed)?;
    Ok(Zeroizing::new(key))
}

// ── Grants ──────────────────────────────────────────────────────────────────

/// A grant's secret: 32 random bytes, only ever in the holder's cookie.
pub fn new_grant_secret() -> Zeroizing<[u8; 32]> {
    Zeroizing::new(random())
}

fn grant_key(server_key: &[u8; 32], secret: &[u8; 32], grant_id: &str, user_id: &str) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(server_key), secret);
    let info = format!("{GRANT_INFO}|{grant_id}|{user_id}");
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(info.as_bytes(), out.as_mut()).expect("32 bytes is a valid HKDF length");
    out
}

fn grant_aad(grant_id: &str, user_id: &str) -> Vec<u8> {
    format!("{GRANT_INFO}|{grant_id}|{user_id}").into_bytes()
}

/// The private key sealed for one grant.
pub fn wrap_for_grant(
    secret: &AccountSecret,
    server_key: &[u8; 32],
    grant_secret: &[u8; 32],
    grant_id: &str,
    user_id: &str,
) -> Vec<u8> {
    let key = grant_key(server_key, grant_secret, grant_id, user_id);
    seal(&key, &grant_aad(grant_id, user_id), secret.bytes().as_ref())
}

/// Open a grant's copy of the private key with the secret from its cookie.
pub fn unwrap_grant(
    wrapped: &[u8],
    server_key: &[u8; 32],
    grant_secret: &[u8; 32],
    grant_id: &str,
    user_id: &str,
    public_key: &[u8],
) -> Result<AccountSecret, KeyError> {
    let key = grant_key(server_key, grant_secret, grant_id, user_id);
    let plain = open(&key, &grant_aad(grant_id, user_id), wrapped)?;
    let secret = AccountSecret::from_bytes(plain)?;
    if secret.public_key().as_slice() != public_key {
        return Err(KeyError::Wrong);
    }
    Ok(secret)
}

/// The private key in the secret box, for the decryption pass of an account
/// switching encryption off (docs/ENCRYPTION.md section 7).
pub fn box_for_job(secret: &AccountSecret, auth_secret: &str) -> anyhow::Result<String> {
    use base64::Engine;
    let text = Zeroizing::new(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret.bytes().as_ref()));
    Ok(crate::secret_box::encrypt_secret(auth_secret, &text, crate::secret_box::SecretPurpose::Vault)?)
}

pub fn unbox_for_job(boxed: &str, auth_secret: &str, public_key: &[u8]) -> Result<AccountSecret, KeyError> {
    use base64::Engine;
    let text = Zeroizing::new(
        crate::secret_box::decrypt_secret(auth_secret, boxed, crate::secret_box::SecretPurpose::Vault)
            .map_err(|_| KeyError::Wrong)?,
    );
    let bytes = Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.as_bytes()).map_err(|_| KeyError::Malformed)?,
    );
    let secret = AccountSecret::from_bytes(bytes)?;
    if secret.public_key().as_slice() != public_key {
        return Err(KeyError::Wrong);
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "user_1";

    #[test]
    fn the_password_wraps_and_only_the_password_unwraps() {
        let secret = AccountSecret::generate();
        let pk = secret.public_key();
        let salt = new_salt();
        let key = password_key("correct horse", &salt, KdfParams::TEST).unwrap();
        let wrapped = wrap_with_password_key(&secret, &key, USER, &pk);
        let back = unwrap_with_password_key(&wrapped, &key, USER, &pk).unwrap();
        assert_eq!(back.public_key(), pk);

        let wrong = password_key("correct horse!", &salt, KdfParams::TEST).unwrap();
        assert_eq!(unwrap_with_password_key(&wrapped, &wrong, USER, &pk).unwrap_err(), KeyError::Wrong);
        // Same password, other salt: another key.
        let other_salt = password_key("correct horse", &new_salt(), KdfParams::TEST).unwrap();
        assert!(unwrap_with_password_key(&wrapped, &other_salt, USER, &pk).is_err());
        // Bound to the account and to its public key.
        assert!(unwrap_with_password_key(&wrapped, &key, "user_2", &pk).is_err());
        let other_pk = AccountSecret::generate().public_key();
        assert!(unwrap_with_password_key(&wrapped, &key, USER, &other_pk).is_err());
        // Tampered or cut.
        let mut bad = wrapped.clone();
        bad[30] ^= 1;
        assert!(unwrap_with_password_key(&bad, &key, USER, &pk).is_err());
        assert_eq!(unwrap_with_password_key(&wrapped[..20], &key, USER, &pk).unwrap_err(), KeyError::Malformed);
    }

    #[test]
    fn passwords_are_normalized() {
        let salt = new_salt();
        // "ą" precomposed and as "a" + combining ogonek.
        let a = password_key("z\u{105}b", &salt, KdfParams::TEST).unwrap();
        let b = password_key("za\u{328}b", &salt, KdfParams::TEST).unwrap();
        assert_eq!(*a, *b);
    }

    #[test]
    fn kdf_parameters_are_bounded() {
        assert!(KdfParams::DEFAULT.sane());
        assert!(KdfParams::TEST.sane());
        let huge = KdfParams { memory_kib: 4 * 1024 * 1024, ..KdfParams::DEFAULT };
        assert_eq!(password_key("x", &new_salt(), huge).unwrap_err(), KeyError::BadParams);
        let slow = KdfParams { iterations: 1000, ..KdfParams::DEFAULT };
        assert_eq!(password_key("x", &new_salt(), slow).unwrap_err(), KeyError::BadParams);
        assert_eq!(password_key("x", &[1, 2, 3], KdfParams::TEST).unwrap_err(), KeyError::BadParams);
    }

    #[test]
    fn file_keys_open_only_with_the_account_key_and_for_their_object() {
        let secret = AccountSecret::generate();
        let pk = secret.public_key();
        let file_key = [42u8; 32];
        let sealed = seal_file_key(&pk, &file_key, "tag/abc.png").unwrap();
        assert_eq!(sealed.len(), 32 + 24 + 32 + 16);
        assert_eq!(*open_file_key(&secret, &sealed, "tag/abc.png").unwrap(), file_key);
        // Another object, another account, a damaged record.
        assert!(open_file_key(&secret, &sealed, "tag/other.png").is_err());
        assert!(open_file_key(&AccountSecret::generate(), &sealed, "tag/abc.png").is_err());
        let mut bad = sealed.clone();
        bad[70] ^= 1;
        assert!(open_file_key(&secret, &bad, "tag/abc.png").is_err());
        assert_eq!(open_file_key(&secret, &sealed[1..], "tag/abc.png").unwrap_err(), KeyError::Malformed);
        // Fresh randomness every time.
        assert_ne!(sealed, seal_file_key(&pk, &file_key, "tag/abc.png").unwrap());
    }

    #[test]
    fn grants_need_the_cookie_the_row_and_the_server_key() {
        let secret = AccountSecret::generate();
        let pk = secret.public_key();
        let server = [5u8; 32];
        let cookie = new_grant_secret();
        let id = "0b9a3c4e-0000-4000-8000-000000000001";
        let wrapped = wrap_for_grant(&secret, &server, &cookie, id, USER);
        assert_eq!(unwrap_grant(&wrapped, &server, &cookie, id, USER, &pk).unwrap().public_key(), pk);
        // Another cookie, another server key, another grant id or account.
        assert!(unwrap_grant(&wrapped, &server, &new_grant_secret(), id, USER, &pk).is_err());
        assert!(unwrap_grant(&wrapped, &[6u8; 32], &cookie, id, USER, &pk).is_err());
        assert!(unwrap_grant(&wrapped, &server, &cookie, "0b9a3c4e-0000-4000-8000-000000000002", USER, &pk).is_err());
        assert!(unwrap_grant(&wrapped, &server, &cookie, id, "user_2", &pk).is_err());
    }

    #[test]
    fn public_keys_carry_the_servers_seal() {
        let server = [9u8; 32];
        let pk = AccountSecret::generate().public_key();
        let mac = public_key_mac(&server, USER, &pk);
        assert_eq!(trusted_public_key(&server, USER, &pk, &mac).unwrap(), pk);
        let swapped = AccountSecret::generate().public_key();
        assert_eq!(trusted_public_key(&server, USER, &swapped, &mac).unwrap_err(), KeyError::UntrustedPublicKey);
        assert!(trusted_public_key(&[1u8; 32], USER, &pk, &mac).is_err());
        assert!(trusted_public_key(&server, "user_2", &pk, &mac).is_err());
    }

    #[test]
    fn the_job_box_round_trips() {
        let secret = AccountSecret::generate();
        let pk = secret.public_key();
        let auth = "job-box-test-secret-0123456789abcdef";
        let boxed = box_for_job(&secret, auth).unwrap();
        assert_eq!(unbox_for_job(&boxed, auth, &pk).unwrap().public_key(), pk);
        assert!(unbox_for_job(&boxed, "another-secret-0123456789abcdef0123", &pk).is_err());
        assert!(unbox_for_job(&boxed, auth, &AccountSecret::generate().public_key()).is_err());
    }
}
