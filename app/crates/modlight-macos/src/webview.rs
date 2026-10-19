//! The YouTube sign-in window (`YouTubeSignInForm`): a window with a browser in it, open
//! on Google's sign-in, that closes itself once the sign-in has happened.
//!
//! macOS substitution: upstream hosts WebView2 with a profile folder of its own; this is
//! a native `NSWindow` holding a `WKWebView` whose web data store is NON-PERSISTENT, so
//! nothing but the cookie jar the app writes survives the window (no browser profile on
//! disk to wipe on sign-out). It is never the person's Safari profile. How it knows it is
//! done is upstream's rule: a navigation lands on youtube.com (not its /signin hop), the
//! page is given 800 ms to settle, and the cookie store is asked whether it holds one of
//! the four names Google sets on a completed sign-in.
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

/// What came of the window (`YouTubeSignInResult`). There is no "runtime missing" on a
/// Mac: WebKit is part of the system.
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
    #[cfg(target_os = "macos")]
    mac::release_retired();
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
/// could not be opened (not on the main thread, or not on macOS).
pub fn open_youtube_sign_in(title: &str, url: &str, dark: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::open(title, url, dark)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, url, dark);
        false
    }
}

/// Close the window if one is open, because the app is going away.
pub fn close_youtube_sign_in() {
    #[cfg(target_os = "macos")]
    mac::close();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::{Cell, RefCell};
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSBackingStoreType, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    };
    use objc2_foundation::{
        NSArray, NSHTTPCookie, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
        NSString, NSTimer, NSURL, NSURLRequest,
    };
    use objc2_web_kit::{
        WKNavigation, WKNavigationDelegate, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
    };

    use super::{CookieData, SignInResult, finish, is_signed_in_jar, is_signed_in_page};

    /// How long YouTube is given to settle before the jar is read.
    const SETTLE_SECONDS: f64 = 0.8;

    /// The tail of Safari's user agent.
    const SAFARI_PRODUCT: &str = "Version/26.0 Safari/605.1.15";

    struct Session {
        window: Retained<NSWindow>,
        web_view: Retained<WKWebView>,
        delegate: Retained<Delegate>,
    }

    thread_local! {
        static CURRENT: RefCell<Option<Session>> = const { RefCell::new(None) };
        /// A closed window's objects, released outside their own callbacks.
        static RETIRED: RefCell<Option<Session>> = const { RefCell::new(None) };
    }

    /// Let go of a closed window's objects (from the app's frame, never from a callback
    /// of theirs).
    pub fn release_retired() {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let _ = RETIRED.try_with(|r| r.borrow_mut().take());
    }

    #[derive(Default)]
    struct Ivars {
        /// A harvest is running or has succeeded.
        settled: Cell<bool>,
        /// The outcome was already reported (the window's close must not overwrite it).
        reported: Cell<bool>,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; Delegate does not implement Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ModlightYouTubeSignInDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl WKNavigationDelegate for Delegate {
            // The event's own success flag is deliberately ignored, as upstream does: the
            // sign-in is a chain of redirects, and the address and the jar decide.
            #[unsafe(method(webView:didFinishNavigation:))]
            fn did_finish(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
                self.check(web_view);
            }
        }

        unsafe impl NSWindowDelegate for Delegate {
            #[unsafe(method(windowWillClose:))]
            fn window_will_close(&self, _notification: &NSNotification) {
                if !self.ivars().reported.replace(true) {
                    finish(SignInResult::Abandoned);
                }
                // Not dropped here: this delegate is the object whose method is running.
                // The session is retired and let go of on the next frame.
                let _ = CURRENT.try_with(|c| {
                    if let Some(session) = c.borrow_mut().take() {
                        // SAFETY: detaching our own delegate from views we own.
                        unsafe {
                            session.web_view.stopLoading();
                            session.web_view.setNavigationDelegate(None);
                        }
                        session.window.setDelegate(None);
                        let _ = RETIRED.try_with(|r| *r.borrow_mut() = Some(session));
                    }
                });
            }
        }
    );

    impl Delegate {
        fn new(mtm: MainThreadMarker) -> Retained<Delegate> {
            let this = Delegate::alloc(mtm).set_ivars(Ivars::default());
            // SAFETY: NSObject's designated initializer.
            unsafe { msg_send![super(this), init] }
        }

        fn check(&self, web_view: &WKWebView) {
            if self.ivars().settled.get() {
                return;
            }
            // SAFETY: plain property read on the main thread.
            let address = unsafe { web_view.URL() }
                .and_then(|url| url.absoluteString())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if !is_signed_in_page(&address) {
                return;
            }
            // Claimed before waiting: navigations arrive in bursts.
            self.ivars().settled.set(true);
            let block = RcBlock::new(move |_timer: NonNull<NSTimer>| harvest());
            // SAFETY: a one-shot timer on the main run loop with a block that only touches
            // main-thread state.
            let _ = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_repeats_block(SETTLE_SECONDS, false, &block)
            };
        }
    }

    /// Read every cookie of the store: the profile has only ever been to Google's sign-in
    /// and to YouTube, whose session spans .google.com and .youtube.com.
    fn harvest() {
        let store = CURRENT.with(|c| {
            c.borrow().as_ref().map(|s| {
                // SAFETY: property reads on the main thread.
                unsafe {
                    s.web_view
                        .configuration()
                        .websiteDataStore()
                        .httpCookieStore()
                }
            })
        });
        let Some(store) = store else {
            return;
        };
        let block = RcBlock::new(|cookies: NonNull<NSArray<NSHTTPCookie>>| {
            // SAFETY: WebKit hands a valid array for the call's duration.
            let cookies = unsafe { cookies.as_ref() };
            let jar: Vec<CookieData> = cookies.iter().map(|c| to_data(&c)).collect();
            let names: Vec<String> = jar.iter().map(|c| c.name.clone()).collect();
            let delegate_reported = CURRENT.with(|c| {
                c.borrow()
                    .as_ref()
                    .map(|s| s.delegate.ivars().reported.get())
                    .unwrap_or(true)
            });
            if delegate_reported {
                return;
            }
            if !is_signed_in_jar(&names) || jar.is_empty() {
                // On youtube.com but not signed in: the person carries on.
                CURRENT.with(|c| {
                    if let Some(s) = c.borrow().as_ref() {
                        s.delegate.ivars().settled.set(false);
                    }
                });
                return;
            }
            CURRENT.with(|c| {
                if let Some(s) = c.borrow().as_ref() {
                    s.delegate.ivars().reported.set(true);
                }
            });
            finish(SignInResult::Captured(jar));
            close();
        });
        // SAFETY: the block lives until WebKit calls it (RcBlock is copied by WebKit).
        unsafe { store.getAllCookies(&block) };
    }

    fn to_data(cookie: &NSHTTPCookie) -> CookieData {
        let expires = if cookie.isSessionOnly() {
            0
        } else {
            cookie
                .expiresDate()
                .map(|d| d.timeIntervalSince1970() as i64)
                .filter(|s| *s > 0)
                .unwrap_or(0)
        };
        CookieData {
            domain: cookie.domain().to_string(),
            path: cookie.path().to_string(),
            name: cookie.name().to_string(),
            value: cookie.value().to_string(),
            secure: cookie.isSecure(),
            expires_unix: expires,
        }
    }

    pub fn open(title: &str, url: &str, dark: bool) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        release_retired();
        let open = CURRENT.with(|c| {
            c.borrow().as_ref().map(|s| {
                s.window.makeKeyAndOrderFront(None);
            })
        });
        if open.is_some() {
            return true;
        }
        let Some(address) = NSURL::URLWithString(&NSString::from_str(url)) else {
            finish(SignInResult::Failed(
                "the sign-in address does not parse".into(),
            ));
            return false;
        };
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1000.0, 760.0));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // SAFETY: standard NSWindow construction on the main thread.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window is retained by the session, not released on close.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.setMinSize(NSSize::new(560.0, 480.0));
        // The app's own theme: a dark title bar in the dark theme.
        // SAFETY: the appearance names are system constants.
        let appearance = unsafe {
            NSAppearance::appearanceNamed(if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            })
        };
        window.setAppearance(appearance.as_deref());

        // SAFETY: WebKit objects created and used on the main thread.
        let web_view = unsafe {
            let configuration = WKWebViewConfiguration::new(mtm);
            // Nothing of this browser outlives the window.
            configuration.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
            // WebKit's bare user agent names no browser, and Google's sign-in refuses
            // what it cannot recognise; this appends Safari's product token, as Safari's
            // own user agent ends.
            configuration
                .setApplicationNameForUserAgent(Some(&NSString::from_str(SAFARI_PRODUCT)));
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
        };
        let delegate = Delegate::new(mtm);
        // SAFETY: the delegate is retained by the session for as long as the views live.
        unsafe {
            web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.setContentView(Some(&web_view));
        window.center();
        let request = NSURLRequest::requestWithURL(&address);
        // SAFETY: loading a request in a view we own.
        let _ = unsafe { web_view.loadRequest(&request) };
        window.makeKeyAndOrderFront(None);
        CURRENT.with(|c| {
            *c.borrow_mut() = Some(Session {
                window,
                web_view,
                delegate,
            })
        });
        true
    }

    pub fn close() {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let window = CURRENT.with(|c| c.borrow().as_ref().map(|s| s.window.clone()));
        if let Some(window) = window {
            window.close();
        }
    }
}

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
