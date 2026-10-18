//! Better Auth's cookies: names, attributes and the signed-value format.
//!
//! From better-auth 1.6.13 `cookies/index.mjs` (`createCookieGetter`,
//! `getCookies`) and better-call 1.3.5 (`signCookieValue`, `getSignedCookie`):
//!
//! - names are `better-auth.<name>`, prefixed with `__Secure-` when the base
//!   URL (APP_URL) is https;
//! - attributes: `Secure` (with the prefix), `SameSite=Lax`, `Path=/`,
//!   `HttpOnly`; `session_token` lives 7 days (`session.expiresIn`), the
//!   cookie-cache cookies 300 s, `dont_remember` is a session cookie;
//! - a signed value is `encodeURIComponent(value + "." + base64(HMAC-SHA256(secret, value)))`,
//!   standard base64 WITH padding (44 characters ending in `=`).

use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::config::Config;
use crate::http::set_cookie::{SameSite, SetCookie};

pub const SECURE_COOKIE_PREFIX: &str = "__Secure-";
pub const COOKIE_PREFIX: &str = "better-auth";

/// `session.expiresIn` default: 7 days, in seconds.
pub const SESSION_EXPIRES_IN: i64 = 60 * 60 * 24 * 7;
/// `session.updateAge` default: refresh at most once a day.
pub const SESSION_UPDATE_AGE: i64 = 60 * 60 * 24;
/// `session.freshAge` default: 1 day.
pub const SESSION_FRESH_AGE: i64 = 60 * 60 * 24;
/// `session.cookieCache.maxAge` default.
pub const COOKIE_CACHE_MAX_AGE: i64 = 300;

/// One Better Auth cookie: its full name and default attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCookie {
    pub name: String,
    pub secure: bool,
    pub max_age: Option<i64>,
}

impl AuthCookie {
    fn new(base: &str, secure: bool, max_age: Option<i64>) -> AuthCookie {
        let prefix = if secure { SECURE_COOKIE_PREFIX } else { "" };
        AuthCookie { name: format!("{prefix}{COOKIE_PREFIX}.{base}"), secure, max_age }
    }

    /// A `Set-Cookie` with this cookie's attributes and the given value.
    pub fn set(&self, value: impl Into<String>) -> SetCookie {
        let mut c = SetCookie::new(self.name.clone(), value)
            .path("/")
            .http_only(true)
            .secure(self.secure)
            .same_site(SameSite::Lax);
        c.max_age = self.max_age;
        c
    }

    /// `expireCookie`: empty value, `Max-Age=0`, same attributes.
    pub fn expire(&self) -> SetCookie {
        self.set("").expired()
    }
}

/// The cookie set of one deployment (`ctx.context.authCookies`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCookies {
    pub session_token: AuthCookie,
    pub session_data: AuthCookie,
    pub dont_remember: AuthCookie,
    pub account_data: AuthCookie,
    /// HMAC key for signed cookies (`ctx.context.secret`).
    pub secret: String,
}

impl AuthCookies {
    pub fn new(secure: bool, secret: impl Into<String>) -> AuthCookies {
        AuthCookies {
            session_token: AuthCookie::new("session_token", secure, Some(SESSION_EXPIRES_IN)),
            session_data: AuthCookie::new("session_data", secure, Some(COOKIE_CACHE_MAX_AGE)),
            dont_remember: AuthCookie::new("dont_remember", secure, None),
            account_data: AuthCookie::new("account_data", secure, Some(COOKIE_CACHE_MAX_AGE)),
            secret: secret.into(),
        }
    }

    /// Better Auth's `baseURL` is APP_URL's origin; https means `__Secure-`.
    pub fn for_config(config: &Config) -> AuthCookies {
        AuthCookies::new(config.secure_cookies(), config.auth.secret.clone())
    }
}

type HmacSha256 = Hmac<Sha256>;

/// `atob` accepts any padding bits; so does this.
const LENIENT_STANDARD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// `makeSignature`: standard base64 (padded) of HMAC-SHA256(secret, value).
pub fn make_signature(value: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// `signCookieValue`: the cookie value as written (already URI-encoded).
pub fn sign_cookie_value(value: &str, secret: &str) -> String {
    crate::http::cookies::encode_uri_component(&format!("{value}.{}", make_signature(value, secret)))
}

/// The value part of a signed cookie BEFORE URI encoding (`value.signature`);
/// what [`SetCookie`] expects, since it encodes on serialization.
pub fn signed_value(value: &str, secret: &str) -> String {
    format!("{value}.{}", make_signature(value, secret))
}

/// What `ctx.getSignedCookie()` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedCookie {
    /// `null`: no cookie, or not shaped like a signed value.
    Absent,
    /// `false`: shaped like a signed value, but the signature is wrong.
    Invalid,
    /// The signed value.
    Valid(String),
}

/// Verify a signed cookie value as parsed (URI-decoded) from the request.
pub fn verify_signed_cookie(value: Option<&str>, secret: &str) -> SignedCookie {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return SignedCookie::Absent;
    };
    let Some(pos) = value.rfind('.') else {
        return SignedCookie::Absent;
    };
    if pos < 1 {
        return SignedCookie::Absent;
    }
    let signed = &value[..pos];
    let signature = &value[pos + 1..];
    if signature.encode_utf16().count() != 44 || !signature.ends_with('=') {
        return SignedCookie::Absent;
    }
    let Ok(sig_bytes) = LENIENT_STANDARD.decode(signature) else {
        return SignedCookie::Invalid;
    };
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(signed.as_bytes());
    // Constant-time comparison, like WebCrypto's verify.
    if mac.verify_slice(&sig_bytes).is_ok() { SignedCookie::Valid(signed.to_string()) } else { SignedCookie::Invalid }
}

/// `@better-auth/utils` `base64Url.decode` / `base64.decode`: picks the URL-safe
/// alphabet when the input contains `-` or `_`, stops at the first `=`, and
/// fails on any other character.
pub fn better_auth_base64_decode(data: &str) -> Option<Vec<u8>> {
    let url_safe = data.contains('-') || data.contains('_');
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for c in data.chars() {
        if c == '=' {
            break;
        }
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' if url_safe => 62,
            '_' if url_safe => 63,
            '+' if !url_safe => 62,
            '/' if !url_safe => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// `createHMAC("SHA-256", "base64urlnopad").sign(secret, data)`.
pub fn hmac_base64url_nopad(secret: &str, data: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// `createHMAC("SHA-256", "base64urlnopad").verify(secret, data, signature)`.
pub fn hmac_base64url_verify(secret: &str, data: &str, signature: &str) -> Option<bool> {
    let sig = better_auth_base64_decode(signature)?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    Some(mac.verify_slice(&sig).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_base_url_scheme() {
        let plain = AuthCookies::new(false, "s");
        assert_eq!(plain.session_token.name, "better-auth.session_token");
        assert_eq!(plain.dont_remember.name, "better-auth.dont_remember");
        let secure = AuthCookies::new(true, "s");
        assert_eq!(secure.session_token.name, "__Secure-better-auth.session_token");
        assert_eq!(secure.session_data.name, "__Secure-better-auth.session_data");
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let secret = "0123456789abcdef0123456789abcdef";
        let signed = signed_value("tok123", secret);
        let sig = signed.rsplit('.').next().unwrap();
        assert_eq!(sig.len(), 44);
        assert!(sig.ends_with('='));
        assert_eq!(verify_signed_cookie(Some(&signed), secret), SignedCookie::Valid("tok123".into()));
        assert_eq!(verify_signed_cookie(Some(&signed), "another-secret-another-secret-xx"), SignedCookie::Invalid);
        assert_eq!(verify_signed_cookie(Some("tok123"), secret), SignedCookie::Absent);
        assert_eq!(verify_signed_cookie(Some(".abc"), secret), SignedCookie::Absent);
        assert_eq!(verify_signed_cookie(Some("tok.short="), secret), SignedCookie::Absent);
        assert_eq!(verify_signed_cookie(None, secret), SignedCookie::Absent);
        // The written cookie is URI-encoded.
        assert!(sign_cookie_value("tok123", secret).ends_with("%3D"));
    }

    /// `printf value | openssl dgst -sha256 -hmac secret -binary | base64`
    #[test]
    fn matches_a_known_vector() {
        assert_eq!(make_signature("value", "secret"), "UOA+vmW+mLuL8RuiyJLVTAeayisNOwFidpxtdXolQ08=");
    }

    #[test]
    fn better_auth_base64_is_lenient_like_upstream() {
        assert_eq!(better_auth_base64_decode("aGk").unwrap(), b"hi");
        assert_eq!(better_auth_base64_decode("aGk=garbage").unwrap(), b"hi");
        assert_eq!(better_auth_base64_decode("-_8").unwrap(), vec![0xfb, 0xff]);
        assert!(better_auth_base64_decode("a!b").is_none());
    }
}
