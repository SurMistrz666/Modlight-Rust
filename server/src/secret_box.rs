//! Port of `lib/secret-box.ts`: authenticated encryption for the handful of
//! secrets that have to live in the `settings` table (the SMTP password and
//! the registration key).
//!
//! Three decisions worth writing down, because each of them is the answer to a
//! concrete failure rather than a preference:
//!
//!  1. **HKDF, not AUTH_SECRET used directly as a key.** The same secret signs
//!     session cookies, unlock tokens and render tickets. A context label makes
//!     the key that encrypts an SMTP password mathematically independent of the
//!     one that signs a session.
//!  2. **GCM, not CBC.** The ciphertext sits in a row anybody with database
//!     access can rewrite; the tag turns a swapped ciphertext into a loud
//!     failure instead of garbage sent to an SMTP server as a password.
//!  3. **SETTINGS_ENCRYPTION_KEY as an optional override**, so rotating
//!     AUTH_SECRET does not silently invalidate the stored SMTP password.
//!
//! Envelope: `v1.<iv>.<tag>.<ciphertext>`, each base64url without padding
//! (Node's `"base64url"`), 12-byte IV, 16-byte tag. Key: HKDF-SHA256(IKM =
//! the override or AUTH_SECRET's UTF-8 bytes, salt = `modlight/secret-box/2026`,
//! info = `modlight:<purpose>:v1`, 32 bytes).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Sha256;

const VERSION: &str = "v1";
const SALT: &[u8] = b"modlight/secret-box/2026";
const IV_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// Context labels. A new purpose gets a new label, never a reused one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretPurpose {
    Smtp,
    RegistrationKey,
    /// Not upstream: the Cloudflare Turnstile secret key (src/turnstile.rs).
    Turnstile,
    /// Not upstream: per-account file encryption (src/vault/): the server key
    /// mixed into every unlock grant, the MAC of each account's public key, and
    /// the private key kept for the decryption pass of an account switching
    /// encryption off.
    Vault,
}

impl SecretPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            SecretPurpose::Smtp => "smtp",
            SecretPurpose::RegistrationKey => "registration-key",
            SecretPurpose::Turnstile => "turnstile",
            SecretPurpose::Vault => "vault",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SecretBoxError(pub String);

/// `Buffer.from(s, "base64")` / `"base64url"`: both alphabets, padding and any
/// other character ignored, decoding stops at the first `=`.
pub fn node_base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            _ => continue,
        };
        buffer = (buffer << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    out
}

/// The IKM, read per call like upstream (`process.env` at call time).
fn raw_key_material(auth_secret: &str) -> Result<Vec<u8>, SecretBoxError> {
    let override_key = std::env::var("SETTINGS_ENCRYPTION_KEY").unwrap_or_default();
    let override_key = crate::config::js_trim(&override_key);
    if !override_key.is_empty() {
        let buf = node_base64_decode(override_key);
        // A short key here is not a weak key, it is a typo.
        if buf.len() != 32 {
            return Err(SecretBoxError(
                "SETTINGS_ENCRYPTION_KEY must be exactly 32 bytes, base64-encoded (generate one with `openssl rand -base64 32`).".into(),
            ));
        }
        return Ok(buf);
    }
    Ok(auth_secret.as_bytes().to_vec())
}

fn key_for(auth_secret: &str, purpose: SecretPurpose) -> Result<[u8; 32], SecretBoxError> {
    let ikm = raw_key_material(auth_secret)?;
    let info = format!("modlight:{}:{VERSION}", purpose.as_str());
    let hk = Hkdf::<Sha256>::new(Some(SALT), &ikm);
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm).map_err(|e| SecretBoxError(e.to_string()))?;
    Ok(okm)
}

/// The 32-byte key of `purpose` itself (not upstream), for a module that needs
/// key material rather than a sealed string: the file vault's server key.
pub fn derive_key(auth_secret: &str, purpose: SecretPurpose) -> Result<[u8; 32], SecretBoxError> {
    key_for(auth_secret, purpose)
}

/// `encryptSecret(plaintext, purpose)`.
pub fn encrypt_secret(auth_secret: &str, plaintext: &str, purpose: SecretPurpose) -> Result<String, SecretBoxError> {
    let key = key_for(auth_secret, purpose)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| SecretBoxError(e.to_string()))?;
    let mut iv = [0u8; IV_BYTES];
    OsRng.fill_bytes(&mut iv);
    let sealed = cipher
        .encrypt(Nonce::from_slice(&iv), plaintext.as_bytes())
        .map_err(|_| SecretBoxError("encryption failed".into()))?;
    let (ct, tag) = sealed.split_at(sealed.len() - TAG_BYTES);
    Ok([VERSION.to_string(), URL_SAFE_NO_PAD.encode(iv), URL_SAFE_NO_PAD.encode(tag), URL_SAFE_NO_PAD.encode(ct)]
        .join("."))
}

/// `decryptSecret(box, purpose)`: throws on any tampering.
pub fn decrypt_secret(auth_secret: &str, boxed: &str, purpose: SecretPurpose) -> Result<String, SecretBoxError> {
    let parts: Vec<&str> = boxed.split('.').collect();
    if parts.len() != 4 || parts[0] != VERSION {
        return Err(SecretBoxError("Unrecognised secret format.".into()));
    }
    let iv = node_base64_decode(parts[1]);
    let tag = node_base64_decode(parts[2]);
    let ct = node_base64_decode(parts[3]);
    if iv.len() != IV_BYTES || tag.len() != TAG_BYTES {
        return Err(SecretBoxError("Malformed secret envelope.".into()));
    }
    let failed = || SecretBoxError("Stored secret could not be decrypted.".into());
    let key = key_for(auth_secret, purpose).map_err(|_| failed())?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| failed())?;
    let mut sealed = ct;
    sealed.extend_from_slice(&tag);
    let plain = cipher.decrypt(Nonce::from_slice(&iv), sealed.as_slice()).map_err(|_| failed())?;
    // Buffer#toString("utf8") replaces invalid sequences.
    Ok(String::from_utf8_lossy(&plain).into_owned())
}

/// `tryDecryptSecret(box, purpose)`: `None` for a missing or unreadable box.
pub fn try_decrypt_secret(auth_secret: &str, boxed: Option<&str>, purpose: SecretPurpose) -> Option<String> {
    let boxed = boxed.filter(|b| !b.is_empty())?;
    decrypt_secret(auth_secret, boxed, purpose).ok()
}

/// `secretDigest(value, purpose)`: HMAC-SHA256 under the purpose key,
/// base64url. Hashing both sides first makes every comparison a fixed 32-byte
/// one, so a comparison never leaks a length.
pub fn secret_digest(auth_secret: &str, value: &str, purpose: SecretPurpose) -> Result<String, SecretBoxError> {
    let key = key_for(auth_secret, purpose)?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// `digestsEqual(a, b)`: constant-time over equal-length inputs.
pub fn digests_equal(a: &str, b: &str) -> bool {
    crate::auth::crypto::constant_time_eq(a.as_bytes(), b.as_bytes())
}

/// The forms a secret can appear in inside an SMTP error text.
fn encodings(secret: &str) -> Vec<String> {
    let b64 = STANDARD.encode(secret.as_bytes());
    vec![
        secret.to_string(),
        b64.clone(),
        b64.trim_end_matches('=').to_string(),
        URL_SAFE.encode(secret.as_bytes()).trim_end_matches('=').to_string(),
    ]
}

/// `redactSecrets(text, secrets)`: every encoding of every secret (4 chars or
/// longer) replaced with `***`.
pub fn redact_secrets(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        for form in encodings(secret) {
            if form.encode_utf16().count() < 4 {
                continue;
            }
            out = out.split(form.as_str()).collect::<Vec<_>>().join("***");
        }
    }
    out
}

/// `smtpSecretForms(user, password)`: the password, plus the AUTH PLAIN
/// payload built from it.
pub fn smtp_secret_forms(user: &str, password: &str) -> Vec<String> {
    if password.is_empty() {
        return Vec::new();
    }
    let auth_plain = STANDARD.encode(format!("\0{user}\0{password}").as_bytes());
    vec![password.to_string(), auth_plain]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-test-secret-test-secret-0123";

    fn enc(text: &str, purpose: SecretPurpose) -> String {
        encrypt_secret(SECRET, text, purpose).unwrap()
    }

    // tests/unit/secret-box.test.ts: describe("secret box")
    #[test]
    fn round_trips_a_secret() {
        let boxed = enc("hunter2-not-a-real-password", SecretPurpose::Smtp);
        assert!(boxed.starts_with("v1."));
        assert_eq!(decrypt_secret(SECRET, &boxed, SecretPurpose::Smtp).unwrap(), "hunter2-not-a-real-password");
    }

    #[test]
    fn never_stores_the_plaintext_in_the_envelope() {
        assert!(!enc("recognisable-plaintext", SecretPurpose::Smtp).contains("recognisable-plaintext"));
    }

    #[test]
    fn produces_a_different_envelope_every_time() {
        assert_ne!(enc("same", SecretPurpose::Smtp), enc("same", SecretPurpose::Smtp));
    }

    #[test]
    fn rejects_a_tampered_ciphertext_instead_of_returning_garbage() {
        let boxed = enc("original", SecretPurpose::Smtp);
        let mut parts: Vec<String> = boxed.split('.').map(str::to_string).collect();
        parts[3] =
            if parts[3].starts_with('A') { format!("B{}", &parts[3][1..]) } else { format!("A{}", &parts[3][1..]) };
        assert!(decrypt_secret(SECRET, &parts.join("."), SecretPurpose::Smtp).is_err());
    }

    #[test]
    fn refuses_a_truncated_envelope() {
        assert!(decrypt_secret(SECRET, "v1.abc", SecretPurpose::Smtp).is_err());
        assert!(decrypt_secret(SECRET, "nonsense", SecretPurpose::Smtp).is_err());
    }

    #[test]
    fn keeps_purposes_independent() {
        let boxed = enc("shared-text", SecretPurpose::Smtp);
        assert_eq!(try_decrypt_secret(SECRET, Some(&boxed), SecretPurpose::RegistrationKey), None);
        // Nor does another AUTH_SECRET open it.
        assert!(decrypt_secret("another-secret-another-secret-another!!", &boxed, SecretPurpose::Smtp).is_err());
    }

    #[test]
    fn reports_an_unreadable_secret_rather_than_pretending_it_is_empty() {
        assert_eq!(try_decrypt_secret(SECRET, Some("v1.AAAA.BBBB.CCCC"), SecretPurpose::Smtp), None);
        assert_eq!(try_decrypt_secret(SECRET, None, SecretPurpose::Smtp), None);
    }

    #[test]
    fn error_messages_match_upstream() {
        assert_eq!(
            decrypt_secret(SECRET, "v2.a.b.c", SecretPurpose::Smtp).unwrap_err().0,
            "Unrecognised secret format."
        );
        assert_eq!(
            decrypt_secret(SECRET, "v1.YQ.YQ.YQ", SecretPurpose::Smtp).unwrap_err().0,
            "Malformed secret envelope."
        );
        let boxed = enc("x", SecretPurpose::Smtp);
        assert_eq!(
            decrypt_secret(SECRET, &boxed, SecretPurpose::RegistrationKey).unwrap_err().0,
            "Stored secret could not be decrypted."
        );
    }

    // describe("digest comparison")
    #[test]
    fn matches_a_value_against_itself_and_nothing_else() {
        let a = secret_digest(SECRET, "correct-key", SecretPurpose::RegistrationKey).unwrap();
        let same = secret_digest(SECRET, "correct-key", SecretPurpose::RegistrationKey).unwrap();
        let other = secret_digest(SECRET, "correct-keys", SecretPurpose::RegistrationKey).unwrap();
        assert!(digests_equal(&a, &same));
        assert!(!digests_equal(&a, &other));
    }

    #[test]
    fn produces_a_fixed_width_whatever_the_input_length() {
        let short = secret_digest(SECRET, "a", SecretPurpose::RegistrationKey).unwrap();
        let long = secret_digest(SECRET, &"a".repeat(500), SecretPurpose::RegistrationKey).unwrap();
        assert_eq!(short.len(), long.len());
    }

    // describe("redaction")
    const PASSWORD: &str = "s3cr3t-smtp-password";

    #[test]
    fn removes_the_plaintext_password() {
        let out = redact_secrets(&format!("535 auth failed for {PASSWORD}"), &[PASSWORD.into()]);
        assert!(!out.contains(PASSWORD));
        assert!(out.contains("***"));
    }

    #[test]
    fn removes_the_base64_form_a_server_may_echo_back() {
        let b64 = STANDARD.encode(PASSWORD);
        let out = redact_secrets(&format!("501 malformed line: {b64}"), &[PASSWORD.into()]);
        assert!(!out.contains(&b64));
    }

    #[test]
    fn removes_the_auth_plain_payload() {
        let forms = smtp_secret_forms("postmaster@example.test", PASSWORD);
        let auth_plain = forms[1].clone();
        let out = redact_secrets(&format!("500 unrecognised: {auth_plain}"), &forms);
        assert!(!out.contains(&auth_plain));
        assert!(!out.contains(PASSWORD));
    }

    #[test]
    fn leaves_an_ordinary_message_alone() {
        let msg = "535 5.7.8 Username and Password not accepted";
        assert_eq!(redact_secrets(msg, &[PASSWORD.into()]), msg);
    }

    #[test]
    fn ignores_empty_secrets_rather_than_redacting_everything() {
        assert_eq!(redact_secrets("connection refused", &["".into()]), "connection refused");
        assert!(smtp_secret_forms("u", "").is_empty());
        assert_eq!(redact_secrets("abc", &["ab".into()]), "abc");
    }

    #[test]
    fn node_base64_is_lenient() {
        assert_eq!(node_base64_decode("aGk"), b"hi");
        assert_eq!(node_base64_decode("aGk="), b"hi");
        assert_eq!(node_base64_decode("-_8"), vec![0xfb, 0xff]);
        assert_eq!(node_base64_decode("+/8"), vec![0xfb, 0xff]);
    }
}
