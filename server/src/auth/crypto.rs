//! Better Auth's crypto primitives, byte-compatible with the values the
//! TypeScript server wrote:
//!
//! - random identifiers and strings (`@better-auth/utils/random`
//!   `createRandomStringGenerator`, `@better-auth/core/utils/id` `generateId`,
//!   better-auth `crypto/random.mjs` `generateRandomString`);
//! - the symmetric encryption of two-factor secrets and backup codes
//!   (`crypto/index.mjs` `symmetricEncrypt` / `symmetricDecrypt`): key =
//!   SHA-256 of the secret's UTF-8 bytes, XChaCha20-Poly1305 with a random
//!   24-byte nonce prepended (`@noble/ciphers` `managedNonce`), lower-case hex;
//! - the HS256 JWT of e-mail verification links (`crypto/jwt.mjs` `signJWT`,
//!   and `jose` 6.2.3 `jwtVerify` for the checks);
//! - TOTP (`@better-auth/utils/otp`): HMAC-SHA1 keyed with the UTF-8 bytes of
//!   the secret STRING (not its base32 decoding), 6 digits, 30 s, a window of
//!   one step either side; the `otpauth://` URI carries the base32 of those
//!   bytes.
//!
//! `BETTER_AUTH_SECRETS` (versioned secrets, the `$ba$<v>$` envelope) is not
//! supported: lib/auth.ts passes `secret` and Modlight never sets that
//! variable, so every value upstream wrote is the bare-hex form.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rand::rngs::OsRng;
use serde_json::{Map, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};

// ── Random strings ──────────────────────────────────────────────────────────

pub const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
pub const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
pub const DIGITS: &str = "0123456789";

/// `createRandomStringGenerator(...)(length)`: rejection sampling over random
/// bytes so every character of `alphabet` is equally likely.
pub fn random_string(length: usize, alphabet: &str) -> String {
    let chars: Vec<char> = alphabet.chars().collect();
    assert!(!chars.is_empty(), "No valid characters provided for random string generation.");
    let n = chars.len();
    let max_valid = (256 / n) * n;
    let mut out = String::with_capacity(length);
    let mut buf = vec![0u8; length.max(1) * 2];
    let mut index = buf.len();
    while out.chars().count() < length {
        if index >= buf.len() {
            OsRng.fill_bytes(&mut buf);
            index = 0;
        }
        let r = buf[index] as usize;
        index += 1;
        if r < max_valid {
            out.push(chars[r % n]);
        }
    }
    out
}

/// `generateId(size)` from @better-auth/core: `a-z`, `A-Z`, `0-9` (32 by
/// default). Row ids, session tokens, verification tokens.
pub fn generate_id(size: usize) -> String {
    random_string(size, &format!("{LOWER}{UPPER}{DIGITS}"))
}

/// better-auth's `generateRandomString(length)`: `a-z`, `0-9`, `A-Z`, `-_`.
/// The TOTP secret and the 2FA / trust-device identifiers.
pub fn generate_random_string(length: usize) -> String {
    random_string(length, &format!("{LOWER}{DIGITS}{UPPER}-_"))
}

// ── Symmetric encryption (two-factor secrets and backup codes) ──────────────

/// `symmetricEncrypt({ key: secret, data })`.
pub fn symmetric_encrypt(secret: &str, data: &str) -> String {
    let key = Sha256::digest(secret.as_bytes());
    let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("a SHA-256 digest is 32 bytes");
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let sealed = cipher.encrypt(XNonce::from_slice(&nonce), data.as_bytes()).expect("encryption cannot fail");
    let mut out = Vec::with_capacity(24 + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    hex::encode(out)
}

/// Why a stored ciphertext could not be opened.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecryptError {
    #[error("hex string expected")]
    NotHex,
    #[error("invalid tag")]
    BadCiphertext,
    #[error("decrypted value is not UTF-8")]
    NotUtf8,
}

/// `symmetricDecrypt({ key: secret, data })`.
pub fn symmetric_decrypt(secret: &str, hex_data: &str) -> Result<String, DecryptError> {
    let bytes = hex::decode(hex_data).map_err(|_| DecryptError::NotHex)?;
    if bytes.len() < 24 + 16 {
        return Err(DecryptError::BadCiphertext);
    }
    let key = Sha256::digest(secret.as_bytes());
    let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("a SHA-256 digest is 32 bytes");
    let (nonce, sealed) = bytes.split_at(24);
    let plain = cipher.decrypt(XNonce::from_slice(nonce), sealed).map_err(|_| DecryptError::BadCiphertext)?;
    // TextDecoder is lossy; a value we encrypted ourselves is always UTF-8.
    String::from_utf8(plain).map_err(|_| DecryptError::NotUtf8)
}

// ── HMAC ────────────────────────────────────────────────────────────────────

type HmacSha256 = Hmac<Sha256>;

/// `createHMAC("SHA-256", "base64urlnopad").sign(secret, data)`.
pub fn hmac_sha256_base64url(secret: &str, data: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Compare two strings in constant time (`constantTimeEqual`).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

// ── JWT (HS256), e-mail verification tokens ─────────────────────────────────

/// `signJWT(payload, secret, expiresIn)`: `new SignJWT(payload)
/// .setProtectedHeader({ alg: "HS256" }).setIssuedAt().setExpirationTime(now + expiresIn)`.
/// The payload keeps its own key order, then `iat`, then `exp`, as jose
/// serialises it; `undefined` fields are simply absent.
pub fn sign_jwt(payload: &Map<String, Value>, secret: &str, expires_in_secs: i64, now_ms: i64) -> String {
    let now = now_ms.div_euclid(1000);
    let mut claims = payload.clone();
    claims.insert("iat".into(), Value::from(now));
    claims.insert("exp".into(), Value::from(now + expires_in_secs));
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256"}"#);
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Value::Object(claims)).unwrap_or_default());
    let signing_input = format!("{header}.{body}");
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(signing_input.as_bytes());
    format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// Why `jwtVerify` refused a token: Better Auth only tells expiry apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwtError {
    /// `JWTExpired`
    Expired,
    /// Anything else (format, algorithm, signature, claims).
    Invalid,
}

fn b64url_decode_strict(s: &str) -> Option<Vec<u8>> {
    if s.contains('=') {
        return None;
    }
    URL_SAFE_NO_PAD.decode(s).ok()
}

/// `jwtVerify(token, secret, { algorithms: ["HS256"] })` with jose 6's claim
/// checks (`iat`/`nbf`/`exp` types, `nbf` in the future, `exp <= now`).
pub fn verify_jwt(token: &str, secret: &str, now_ms: i64) -> Result<Map<String, Value>, JwtError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(JwtError::Invalid);
    }
    let header: Value = serde_json::from_slice(&b64url_decode_strict(parts[0]).ok_or(JwtError::Invalid)?)
        .map_err(|_| JwtError::Invalid)?;
    let Value::Object(header) = header else {
        return Err(JwtError::Invalid);
    };
    if header.get("alg").and_then(Value::as_str) != Some("HS256") {
        return Err(JwtError::Invalid);
    }
    if header.contains_key("crit") || header.get("b64").is_some_and(|b| b != &Value::Bool(true)) {
        return Err(JwtError::Invalid);
    }
    let signature = b64url_decode_strict(parts[2]).ok_or(JwtError::Invalid)?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
    mac.verify_slice(&signature).map_err(|_| JwtError::Invalid)?;
    let payload: Value = serde_json::from_slice(&b64url_decode_strict(parts[1]).ok_or(JwtError::Invalid)?)
        .map_err(|_| JwtError::Invalid)?;
    let Value::Object(claims) = payload else {
        return Err(JwtError::Invalid);
    };
    let now = (now_ms.div_euclid(1000)) as f64;
    if let Some(iat) = claims.get("iat")
        && !iat.is_number()
    {
        return Err(JwtError::Invalid);
    }
    if let Some(nbf) = claims.get("nbf") {
        let nbf = nbf.as_f64().ok_or(JwtError::Invalid)?;
        if nbf > now {
            return Err(JwtError::Invalid);
        }
    }
    if let Some(exp) = claims.get("exp") {
        let exp = exp.as_f64().ok_or(JwtError::Invalid)?;
        if exp <= now {
            return Err(JwtError::Expired);
        }
    }
    Ok(claims)
}

// ── TOTP ────────────────────────────────────────────────────────────────────

pub const TOTP_DIGITS: u32 = 6;
pub const TOTP_PERIOD: i64 = 30;

/// `generateHOTP(secret, { counter, digits })` with HMAC-SHA1 keyed by the
/// UTF-8 bytes of `secret`.
pub fn hotp(secret: &str, counter: u64, digits: u32) -> String {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let offset = (h[h.len() - 1] & 0x0f) as usize;
    let truncated = ((u32::from(h[offset]) & 0x7f) << 24)
        | (u32::from(h[offset + 1]) << 16)
        | (u32::from(h[offset + 2]) << 8)
        | u32::from(h[offset + 3]);
    let otp = truncated % 10u32.pow(digits);
    format!("{otp:0width$}", width = digits as usize)
}

/// `createOTP(secret).totp()` at `now_ms`.
pub fn totp(secret: &str, now_ms: i64) -> String {
    let counter = now_ms.div_euclid(TOTP_PERIOD * 1000);
    hotp(secret, counter as u64, TOTP_DIGITS)
}

/// `createOTP(secret, { period: 30, digits: 6 }).verify(code)`: the codes of
/// the previous, current and next step are accepted (exact string match).
pub fn verify_totp(secret: &str, code: &str, now_ms: i64) -> bool {
    let counter = now_ms.div_euclid(TOTP_PERIOD * 1000);
    (-1i64..=1).any(|i| {
        let c = counter + i;
        c >= 0 && hotp(secret, c as u64, TOTP_DIGITS) == code
    })
}

/// RFC 4648 base32 (upper case), unpadded: `base32.encode(data, { padding: false })`.
pub fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// `URLSearchParams` serialisation of one value
/// (application/x-www-form-urlencoded: space as `+`, `*-._` and alphanumerics
/// kept, everything else percent-encoded from UTF-8).
pub fn form_urlencode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// `createOTP(secret, { digits, period }).url(issuer, account)`:
/// `otpauth://totp/<issuer>:<account>?secret=..&issuer=..&digits=6&period=30`.
pub fn totp_uri(secret: &str, issuer: &str, account: &str) -> String {
    use crate::http::cookies::encode_uri_component;
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&digits={}&period={}",
        encode_uri_component(issuer),
        encode_uri_component(account),
        form_urlencode(&base32_encode(secret.as_bytes())),
        form_urlencode(issuer),
        TOTP_DIGITS,
        TOTP_PERIOD
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_use_the_better_auth_alphabets() {
        let id = generate_id(32);
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        let s = generate_random_string(500);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let digits = random_string(6, DIGITS);
        assert!(digits.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn symmetric_round_trip_and_tamper_detection() {
        let secret = "an-auth-secret-of-sufficient-length-123";
        let boxed = symmetric_encrypt(secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(boxed.len(), (24 + 16 + 16) * 2);
        assert!(boxed.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(symmetric_decrypt(secret, &boxed).unwrap(), "JBSWY3DPEHPK3PXP");
        assert_eq!(symmetric_decrypt("another-secret", &boxed), Err(DecryptError::BadCiphertext));
        assert_eq!(symmetric_decrypt(secret, "zz"), Err(DecryptError::NotHex));
    }

    // RFC 4226 appendix D: HOTP("12345678901234567890").
    #[test]
    fn hotp_matches_the_rfc_vectors() {
        let expected = ["755224", "287082", "359152", "969429", "338314"];
        for (i, code) in expected.iter().enumerate() {
            assert_eq!(hotp("12345678901234567890", i as u64, 6), *code);
        }
    }

    #[test]
    fn totp_accepts_one_step_of_drift() {
        let secret = "abcdefghijklmnopqrstuvwxyz012345";
        let now = 1_760_000_000_000;
        let code = totp(secret, now);
        assert!(verify_totp(secret, &code, now));
        assert!(verify_totp(secret, &code, now + 30_000));
        assert!(verify_totp(secret, &code, now - 30_000));
        assert!(!verify_totp(secret, &code, now + 90_000));
    }

    #[test]
    fn base32_and_uri() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "MY");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert_eq!(
            totp_uri("abc", "Modlight", "a@b.co"),
            "otpauth://totp/Modlight:a%40b.co?secret=MFRGG&issuer=Modlight&digits=6&period=30"
        );
        assert_eq!(form_urlencode("My App"), "My+App");
    }

    #[test]
    fn jwt_round_trip_and_expiry() {
        let mut payload = Map::new();
        payload.insert("email".into(), Value::from("a@b.co"));
        let now = 1_760_000_000_123;
        let token = sign_jwt(&payload, "secret", 3600, now);
        assert!(token.starts_with("eyJhbGciOiJIUzI1NiJ9."));
        let claims = verify_jwt(&token, "secret", now).unwrap();
        assert_eq!(claims["email"], "a@b.co");
        assert_eq!(claims["iat"], 1_760_000_000);
        assert_eq!(claims["exp"], 1_760_003_600);
        let keys: Vec<&String> = claims.keys().collect();
        assert_eq!(keys, ["email", "iat", "exp"]);
        assert_eq!(verify_jwt(&token, "other", now), Err(JwtError::Invalid));
        assert_eq!(verify_jwt(&token, "secret", now + 3_600_000), Err(JwtError::Expired));
        assert_eq!(verify_jwt("a.b", "secret", now), Err(JwtError::Invalid));
    }
}
