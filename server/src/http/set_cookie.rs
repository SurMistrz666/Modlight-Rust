//! `Set-Cookie` values, serialized the two ways upstream produces them:
//!
//! - [`SetCookie::to_next_string`]: Next's `cookies().set()` (route handlers,
//!   server actions, and every cookie Better Auth sets through its
//!   `nextCookies()` plugin). Next fills in `Path=/` and an `Expires` derived
//!   from `maxAge`, and orders the attributes Path, Expires, Max-Age, Domain,
//!   Secure, HttpOnly, SameSite, Partitioned, Priority.
//! - [`SetCookie::to_better_call_string`]: Better Auth's own handler
//!   (`/api/auth/*`), which orders them Max-Age, Domain, Path, Expires,
//!   HttpOnly, Secure, SameSite, Partitioned.
//!
//! Both encode the value with `encodeURIComponent`.

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, HeaderValue, header};
use chrono::{DateTime, Utc};

use crate::http::cookies::encode_uri_component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Lax,
    Strict,
    None,
}

impl SameSite {
    fn capitalized(self) -> &'static str {
        match self {
            SameSite::Lax => "Lax",
            SameSite::Strict => "Strict",
            SameSite::None => "None",
        }
    }
    fn lower(self) -> &'static str {
        match self {
            SameSite::Lax => "lax",
            SameSite::Strict => "strict",
            SameSite::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetCookie {
    pub name: String,
    /// The value BEFORE `encodeURIComponent`.
    pub value: String,
    pub max_age: Option<i64>,
    pub expires: Option<DateTime<Utc>>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<SameSite>,
    pub partitioned: bool,
}

impl SetCookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> SetCookie {
        SetCookie {
            name: name.into(),
            value: value.into(),
            max_age: None,
            expires: None,
            domain: None,
            path: None,
            secure: false,
            http_only: false,
            same_site: None,
            partitioned: false,
        }
    }

    pub fn max_age(mut self, seconds: i64) -> Self {
        self.max_age = Some(seconds);
        self
    }
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn http_only(mut self, on: bool) -> Self {
        self.http_only = on;
        self
    }
    pub fn secure(mut self, on: bool) -> Self {
        self.secure = on;
        self
    }
    pub fn same_site(mut self, s: SameSite) -> Self {
        self.same_site = Some(s);
        self
    }

    /// `name=; Max-Age=0` with the same attributes (Better Auth `expireCookie`).
    pub fn expired(mut self) -> Self {
        self.value = String::new();
        self.max_age = Some(0);
        self
    }

    /// Next's `stringifyCookie` after `normalizeCookie`, at time `now`.
    pub fn to_next_string_at(&self, now: DateTime<Utc>) -> String {
        let mut attrs: Vec<String> = Vec::new();
        let path = self.path.clone().unwrap_or_else(|| "/".to_string());
        attrs.push(format!("Path={path}"));
        // normalizeCookie: a truthy maxAge becomes an Expires date.
        let expires = match self.max_age {
            Some(m) if m != 0 => Some(now + chrono::Duration::milliseconds(m * 1000)),
            _ => self.expires,
        };
        if let Some(e) = expires {
            attrs.push(format!("Expires={}", http_date(e)));
        }
        if let Some(m) = self.max_age {
            attrs.push(format!("Max-Age={m}"));
        }
        if let Some(d) = &self.domain {
            attrs.push(format!("Domain={d}"));
        }
        if self.secure {
            attrs.push("Secure".into());
        }
        if self.http_only {
            attrs.push("HttpOnly".into());
        }
        if let Some(s) = self.same_site {
            // Next prints the value it was given; Better Auth's parsed cookies
            // arrive lower-cased (`SameSite=lax`).
            attrs.push(format!("SameSite={}", s.lower()));
        }
        if self.partitioned {
            attrs.push("Partitioned".into());
        }
        format!("{}={}; {}", self.name, encode_uri_component(&self.value), attrs.join("; "))
    }

    pub fn to_next_string(&self) -> String {
        self.to_next_string_at(Utc::now())
    }

    /// better-call `serializeCookie`.
    pub fn to_better_call_string(&self) -> String {
        let mut cookie = format!("{}={}", self.name, encode_uri_component(&self.value));
        let secure = self.secure || self.name.starts_with("__Secure-") || self.name.starts_with("__Host-");
        if let Some(m) = self.max_age
            && m >= 0
        {
            cookie.push_str(&format!("; Max-Age={m}"));
        }
        if let Some(d) = &self.domain {
            cookie.push_str(&format!("; Domain={d}"));
        }
        if let Some(p) = &self.path {
            cookie.push_str(&format!("; Path={p}"));
        }
        if let Some(e) = self.expires {
            cookie.push_str(&format!("; Expires={}", http_date(e)));
        }
        if self.http_only {
            cookie.push_str("; HttpOnly");
        }
        if secure {
            cookie.push_str("; Secure");
        }
        if let Some(s) = self.same_site {
            cookie.push_str(&format!("; SameSite={}", s.capitalized()));
        }
        if self.partitioned {
            cookie.push_str("; Partitioned");
        }
        cookie
    }
}

/// `Date.prototype.toUTCString()`.
pub fn http_date(d: DateTime<Utc>) -> String {
    d.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

/// Cookies a request wants to set, collected by whatever ran (session refresh,
/// an action, a route handler) and written onto the response by the front door.
/// Mirrors Next's mutable `cookies()` store for one request.
#[derive(Clone, Default, Debug)]
pub struct ResponseCookies(Arc<Mutex<Vec<SetCookie>>>);

impl ResponseCookies {
    /// `cookies().set(...)`: a cookie of the same name set earlier in the
    /// request is replaced in place (Next keeps one entry per name).
    pub fn push(&self, cookie: SetCookie) {
        let mut list = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match list.iter_mut().find(|c| c.name == cookie.name) {
            Some(existing) => *existing = cookie,
            None => list.push(cookie),
        }
    }

    pub fn take(&self) -> Vec<SetCookie> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Append every collected cookie as a `Set-Cookie` header (Next format).
    pub fn write_to(&self, headers: &mut HeaderMap) {
        let now = Utc::now();
        for cookie in self.take() {
            if let Ok(v) = HeaderValue::from_str(&cookie.to_next_string_at(now)) {
                headers.append(header::SET_COOKIE, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn next_format() {
        let now = Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap();
        let c = SetCookie::new("better-auth.session_token", "tok.sig=")
            .max_age(604799)
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax);
        assert_eq!(
            c.to_next_string_at(now),
            "better-auth.session_token=tok.sig%3D; Path=/; Expires=Sun, 25 Oct 2026 11:59:59 GMT; Max-Age=604799; HttpOnly; SameSite=lax"
        );
        let gone = SetCookie::new("ml_x", "v").path("/").expired();
        assert_eq!(gone.to_next_string_at(now), "ml_x=; Path=/; Max-Age=0");
    }

    #[test]
    fn one_entry_per_name_like_nexts_cookie_store() {
        let jar = ResponseCookies::default();
        jar.push(SetCookie::new("a", "1"));
        jar.push(SetCookie::new("b", "2"));
        jar.push(SetCookie::new("a", "3"));
        let names: Vec<(String, String)> = jar.take().into_iter().map(|c| (c.name, c.value)).collect();
        assert_eq!(names, vec![("a".to_string(), "3".to_string()), ("b".to_string(), "2".to_string())]);
    }

    #[test]
    fn better_call_format() {
        let c = SetCookie::new("__Secure-better-auth.session_token", "tok.sig=")
            .max_age(604800)
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax);
        assert_eq!(
            c.to_better_call_string(),
            "__Secure-better-auth.session_token=tok.sig%3D; Max-Age=604800; Path=/; HttpOnly; Secure; SameSite=Lax"
        );
    }
}
