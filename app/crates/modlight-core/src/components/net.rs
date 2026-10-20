//! Where a component, a tool update or an app update may be fetched from, and the HTTP
//! client that fetches it.
//!
//! The rules are the audit's (`docs/audyt/klient.md`, PORT.md section 5): https only,
//! redirects followed only to https, and a GitHub release artefact only from
//! `github.com` itself. Tests run against a server on 127.0.0.1 over plain http; the
//! switch that lets them is [`testing::allow_origin`], which exists only in debug builds,
//! so no release build can be talked into fetching anything over http.

use std::time::Duration;

use url::Url;

/// The only host a GitHub release artefact may come from (`ModlightToolRelease.ArtefactHost`).
pub const ARTEFACT_HOST: &str = "github.com";

/// What the app calls itself on the wire, like the Windows client (`Modlight/<version>`).
pub fn user_agent() -> String {
    format!("Modlight/{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(debug_assertions)]
static TEST_ORIGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

#[cfg(debug_assertions)]
static TEST_GITHUB_API: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Where GitHub's API is asked: `None` means api.github.com. Only a test can move it, and
/// only in a debug build.
pub fn github_api_origin() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        TEST_GITHUB_API.read().ok().and_then(|o| o.clone())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// The origin of a URL as tests register it: `http://127.0.0.1:1234`.
fn origin_of(url: &Url) -> String {
    match url.port() {
        Some(port) => format!(
            "{}://{}:{port}",
            url.scheme(),
            url.host_str().unwrap_or_default()
        ),
        None => format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default()),
    }
}

/// Whether a test registered this URL's origin. Always false in a release build.
pub fn is_test_origin(url: &Url) -> bool {
    #[cfg(debug_assertions)]
    {
        let origin = origin_of(url);
        TEST_ORIGINS
            .read()
            .map(|list| list.contains(&origin))
            .unwrap_or(false)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = origin_of(url);
        false
    }
}

/// Switches for tests. Not compiled into release builds.
#[cfg(debug_assertions)]
pub mod testing {
    /// Let `origin` (only `http://127.0.0.1:<port>`) stand in for an https host and for
    /// `github.com` in this process.
    pub fn allow_origin(origin: &str) {
        assert!(
            origin.starts_with("http://127.0.0.1:"),
            "only a loopback test server may be allowed, not {origin}"
        );
        if let Ok(mut list) = super::TEST_ORIGINS.write()
            && !list.iter().any(|o| o == origin)
        {
            list.push(origin.trim_end_matches('/').to_string());
        }
    }

    /// Ask a loopback stub instead of api.github.com (and allow its origin).
    pub fn set_github_api(origin: Option<&str>) {
        if let Some(origin) = origin {
            allow_origin(origin);
        }
        if let Ok(mut slot) = super::TEST_GITHUB_API.write() {
            *slot = origin.map(|o| o.trim_end_matches('/').to_string());
        }
    }
}

/// Whether we are willing to fetch this address at all: https with a host.
pub fn is_fetchable(value: &str) -> bool {
    match Url::parse(value) {
        Ok(url) => is_fetchable_url(&url),
        Err(_) => false,
    }
}

fn is_fetchable_url(url: &Url) -> bool {
    (url.scheme() == "https" && url.host_str().is_some_and(|h| !h.is_empty()))
        || is_test_origin(url)
}

/// `ModlightToolRelease.IsArtefactUrl`: https and exactly `github.com`.
pub fn is_artefact_url(value: &str) -> bool {
    match Url::parse(value) {
        Ok(url) => {
            (url.scheme() == "https"
                && url
                    .host_str()
                    .is_some_and(|h| h.eq_ignore_ascii_case(ARTEFACT_HOST)))
                || is_test_origin(&url)
        }
        Err(_) => false,
    }
}

/// A client for fetching files: no overall timeout (the stall watchdog of the caller
/// tells slow from dead), redirects followed only to https, no automatic decompression.
pub fn file_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if is_fetchable_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("a redirect left https")
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// A client for small JSON and text answers (release documents, checksum files).
pub fn api_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if is_fetchable_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("a redirect left https")
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_is_fetchable() {
        assert!(is_fetchable("https://github.com/a/b"));
        assert!(!is_fetchable("http://github.com/a/b"));
        assert!(!is_fetchable("ftp://github.com/a"));
        assert!(!is_fetchable("file:///etc/passwd"));
        assert!(!is_fetchable("not a url"));
    }

    #[test]
    fn an_artefact_comes_from_github_com_over_https_only() {
        assert!(is_artefact_url(
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos"
        ));
        assert!(is_artefact_url("https://GitHub.com/x"));
        assert!(!is_artefact_url("http://github.com/x"));
        assert!(!is_artefact_url("https://github.com.evil.example/x"));
        assert!(!is_artefact_url("https://objects.githubusercontent.com/x"));
        assert!(!is_artefact_url("https://evil.example/github.com"));
    }

    #[test]
    fn loopback_http_is_refused_until_a_test_allows_it() {
        assert!(!is_fetchable("http://127.0.0.1:9/x"));
        testing::allow_origin("http://127.0.0.1:9");
        assert!(is_fetchable("http://127.0.0.1:9/x"));
        assert!(is_artefact_url("http://127.0.0.1:9/x"));
        assert!(!is_fetchable("http://127.0.0.1:10/x"));
    }

    #[test]
    #[should_panic]
    fn only_loopback_can_be_allowed() {
        testing::allow_origin("http://example.com");
    }
}
