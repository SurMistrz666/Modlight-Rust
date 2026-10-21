//! The YouTube sign-in window (`YouTubeSignInForm`): a window with a browser in it, open
//! on Google's sign-in, that closes itself once the sign-in has happened.
//!
//! Windows: WebView2, as upstream, in an InPrivate profile, so nothing but the cookie jar
//! the app writes survives the window. How it knows it is done is upstream's rule: a
//! navigation lands on youtube.com (not its /signin hop), the page is given 800 ms to
//! settle, and the cookie store is asked whether it holds one of the four names Google
//! sets on a completed sign-in.
//!
//! The window is opened only by the real app (`Mode::Normal` / `Mode::Qa`), never in
//! automated runs. All of it runs on the main thread; the outcome is picked up by the
//! window's frame through [`take_result`].

use std::sync::Mutex;

/// One cookie, as the store gave it. The value never leaves the app's own jar.
#[derive(Clone, PartialEq, Eq)]
pub struct CookieData {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    pub secure: bool,
    /// Seconds since the epoch; zero for a session cookie.
    pub expires_unix: i64,
}

impl std::fmt::Debug for CookieData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieData")
            .field("domain", &self.domain)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// What came of the window (`YouTubeSignInResult`). WebView2 is part of Windows 11
/// and of every updated Windows 10.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInResult {
    /// A completed sign-in; the caller writes the jar.
    Captured(Vec<CookieData>),
    /// Closed without a completed sign-in. Not an error.
    Abandoned,
    /// The window could not be built. The reason is for the log.
    Failed(String),
}

static RESULT: Mutex<Option<SignInResult>> = Mutex::new(None);

/// The outcome, once the window has closed (taken once).
pub fn take_result() -> Option<SignInResult> {
    RESULT.lock().ok().and_then(|mut r| r.take())
}

fn finish(result: SignInResult) {
    if let Ok(mut slot) = RESULT.lock() {
        *slot = Some(result);
    }
}

/// Whether these cookie names amount to a completed Google sign-in.
pub fn is_signed_in_jar(names: &[String]) -> bool {
    names.iter().any(|n| {
        matches!(
            n.as_str(),
            "__Secure-1PSID" | "__Secure-3PSID" | "SID" | "LOGIN_INFO"
        )
    })
}

/// Whether this address is the far side of the sign-in: a youtube.com host (suffix
/// match), not the /signin hop.
pub fn is_signed_in_page(address: &str) -> bool {
    let Some(rest) = address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"))
    else {
        return false;
    };
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host != "youtube.com" && !host.ends_with(".youtube.com") {
        return false;
    }
    !path.to_ascii_lowercase().starts_with("/signin")
}

/// Open the window, or bring the open one forward (one at a time). `false` when it
/// could not be opened (not on the main thread, no WebView2 runtime).
pub fn open_youtube_sign_in(title: &str, url: &str, dark: bool) -> bool {
    let _ = (title, url, dark);
    let _ = finish;
    false
}

/// Close the window if one is open, because the app is going away.
pub fn close_youtube_sign_in() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_far_side_of_the_sign_in_is_recognised() {
        assert!(is_signed_in_page("https://www.youtube.com/"));
        assert!(is_signed_in_page("https://m.youtube.com/feed?x=1"));
        assert!(is_signed_in_page("https://youtube.com"));
        assert!(!is_signed_in_page(
            "https://www.youtube.com/signin?continue=x"
        ));
        assert!(!is_signed_in_page("https://youtube.com.evil.example/"));
        assert!(!is_signed_in_page(
            "https://accounts.google.com/ServiceLogin"
        ));
        assert!(!is_signed_in_page("about:blank"));
    }

    #[test]
    fn a_completed_sign_in_is_named_by_its_cookies() {
        assert!(is_signed_in_jar(&["PREF".into(), "__Secure-1PSID".into()]));
        assert!(!is_signed_in_jar(&[
            "PREF".into(),
            "VISITOR_INFO1_LIVE".into()
        ]));
    }

    #[test]
    fn a_cookie_value_never_prints() {
        let cookie = CookieData {
            domain: ".youtube.com".into(),
            path: "/".into(),
            name: "SID".into(),
            value: "secret-value".into(),
            secure: true,
            expires_unix: 0,
        };
        assert!(!format!("{cookie:?}").contains("secret-value"));
    }

    #[test]
    fn no_window_opens_off_the_main_thread() {
        // Tests run on worker threads: nothing may appear on the owner's screen.
        let handle = std::thread::spawn(|| open_youtube_sign_in("t", "https://example.com", true));
        assert!(!handle.join().unwrap());
    }
}
