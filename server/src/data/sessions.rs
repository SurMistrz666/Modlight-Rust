//! Port of `lib/data/sessions.ts`: active sign-ins for the current account,
//! for /dashboard/profile.
//!
//! Better Auth owns the `session` table, so everything here goes through its
//! own operations (`/list-sessions`, `/revoke-session`,
//! `/revoke-other-sessions`, ported in [`crate::auth::server`]) rather than
//! through ad-hoc SQL. That is not politeness: deleting a row underneath it
//! would leave the cookie cache, the session hooks and any future secondary
//! storage believing the session is still alive, so a session could read as
//! "revoked" on this screen while its cookie carried on working.
//!
//! `/list-sessions` is scoped to the CALLER by Better Auth's own session
//! middleware, which is also the entire authorisation story for this file:
//! there is no code path here that can be handed somebody else's user id, so
//! an administrator gets their own devices on this screen and nobody else's.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use sqlx::PgPool;

use crate::auth::session::AuthSession;
use crate::config::js_trim;
use crate::db::models::Session;

/// What kind of thing is holding the session, as far as it will admit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Desktop,
    Mobile,
    Tablet,
    Client,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceLabel {
    /// Browser name plus major version, or None when we cannot tell.
    pub browser: Option<String>,
    /// Operating system, only as precisely as the string honestly says.
    pub os: Option<String>,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    /// Row id. Deliberately NOT the session token, see [`my_session_token`].
    pub id: String,
    /// True for the session that is rendering this page.
    pub current: bool,
    /// ISO. When this session was created, i.e. when someone signed in.
    pub created_at: String,
    /// ISO. When Better Auth last renewed the session.
    ///
    /// This is NOT a last-seen time and must never be labelled as one: the
    /// session row is only written when it is refreshed (once its `updateAge`
    /// of one day has passed), so a device used every hour still stamps this
    /// once a day. Better Auth records no per-request timestamp, and inventing
    /// one would mean a write on every authenticated request.
    pub renewed_at: String,
    /// ISO. When the session stops working on its own.
    pub expires_at: String,
    /// Client address as the reverse proxy reported it, or None.
    pub ip: Option<String>,
    /// The raw User-Agent, clipped. Attacker-controlled free text: render it
    /// as TEXT and nothing else.
    pub user_agent: Option<String>,
    pub device: DeviceLabel,
    /// Admin plugin: this session was opened by someone impersonating the user.
    pub impersonated: bool,
}

/// A User-Agent longer than this is noise; nothing legitimate needs it.
const MAX_UA: usize = 300;

/// JavaScript's `\s` (WhiteSpace and LineTerminator), for the patterns below.
const JS_SPACE: &str =
    r"\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex")
}

/// Non-browser clients, checked first: several of them also carry a browser
/// token to get past naive sniffing, and "curl" is a more useful answer than
/// "Chrome 120".
///
/// The upstream patterns are `/.../i` without the `u` flag: case-insensitive
/// for ASCII letters only, hence `(?i-u)`. `\d` is `[0-9]` in JavaScript.
static CLIENTS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (re(r"(?i-u)Modlight[/ ]?([0-9.]+)?"), "Modlight App"),
        (re(r"(?i-u)ShareX[/ ]?([0-9.]+)?"), "ShareX"),
        (re(r"(?i-u)curl/([0-9.]+)"), "curl"),
        (re(r"(?i-u)Wget/([0-9.]+)"), "Wget"),
        (re(r"(?i-u)python-requests/([0-9.]+)"), "python-requests"),
        (re(r"(?i-u)PostmanRuntime/([0-9.]+)"), "Postman"),
        (re(r"(?i-u)insomnia/([0-9.]+)"), "Insomnia"),
        (re(r"(?i-u)okhttp/([0-9.]+)"), "OkHttp"),
        (re(r"(?i-u)Go-http-client/([0-9.]+)"), "Go"),
        (re(r"(?i-u)PowerShell/([0-9.]+)"), "PowerShell"),
    ]
});

/// Browsers, in the only order that works.
///
/// Every Chromium browser still ships "Chrome/" in its string, and every one
/// of them ships "Safari/" as well, so the specific names have to be tried
/// before the generic ones or Edge and Opera both come out as Chrome. Brave
/// removed its own token on purpose and is genuinely indistinguishable from
/// Chrome, so it is reported as Chrome rather than guessed at.
static BROWSERS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (re(r"Edg(?:iOS|A)?/([0-9]+)"), "Edge"),
        (re(r"OPR/([0-9]+)"), "Opera"),
        (re(r"Opera/([0-9]+)"), "Opera"),
        (re(r"Vivaldi/([0-9]+)"), "Vivaldi"),
        (re(r"SamsungBrowser/([0-9]+)"), "Samsung Internet"),
        (re(r"(?:Firefox|FxiOS)/([0-9]+)"), "Firefox"),
        (re(r"(?:CriOS|Chrome)/([0-9]+)"), "Chrome"),
        (re(&format!(r"Version/([0-9]+)[0-9.]*(?: Mobile/[^{JS_SPACE}]+)? Safari")), "Safari"),
    ]
});

static WINDOWS_NT: LazyLock<Regex> = LazyLock::new(|| re(r"Windows NT ([0-9.]+)"));
static ANDROID_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"Android ([0-9]+)"));
static IOS_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"(?:iPhone|iPad|iPod)[^)]*?OS ([0-9]+)[._]"));

/// The operating system, claimed only as far as the string is honest.
///
/// Windows NT 10.0 is reported by Windows 10 AND Windows 11 (Microsoft never
/// moved the token), so this says "Windows" rather than picking one. macOS has
/// been frozen at 10_15_7 in every browser since 2021 for fingerprinting
/// reasons, so it gets no version either. Android and iOS do report the real
/// number, so those are shown.
fn os_of(ua: &str) -> Option<String> {
    if let Some(nt) = WINDOWS_NT.captures(ua) {
        return Some(
            match &nt[1] {
                "6.1" => "Windows 7",
                "6.2" => "Windows 8",
                "6.3" => "Windows 8.1",
                _ => "Windows",
            }
            .to_string(),
        );
    }
    // CrOS and Android both contain "Linux", so they have to be tested first.
    if ua.contains("CrOS") {
        return Some("ChromeOS".into());
    }
    if let Some(android) = ANDROID_VERSION.captures(ua) {
        return Some(format!("Android {}", &android[1]));
    }
    if let Some(ios) = IOS_VERSION.captures(ua) {
        return Some(format!("iOS {}", &ios[1]));
    }
    if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        return Some("macOS".into());
    }
    if ua.contains("Linux") || ua.contains("X11") {
        return Some("Linux".into());
    }
    None
}

fn is_js_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `/Android(?!.*Mobile)/`: an "Android" with no "Mobile" after it on the same
/// line (JavaScript's `.` stops at line terminators). The regex crate has no
/// lookahead, so the assertion is spelled out.
fn android_without_mobile(ua: &str) -> bool {
    ua.match_indices("Android").any(|(i, m)| {
        let rest = &ua[i + m.len()..];
        let line = rest.split(is_js_line_terminator).next().unwrap_or("");
        !line.contains("Mobile")
    })
}

fn kind_of(ua: &str, os: Option<&str>) -> DeviceKind {
    // An Android tablet is an Android UA WITHOUT the "Mobile" token; that is
    // the documented difference, and the only one there is.
    if ua.contains("iPad") || ua.contains("Tablet") || android_without_mobile(ua) {
        return DeviceKind::Tablet;
    }
    if ["Mobile", "iPhone", "iPod", "Android"].iter().any(|t| ua.contains(t)) {
        return DeviceKind::Mobile;
    }
    if os.is_some() {
        return DeviceKind::Desktop;
    }
    DeviceKind::Unknown
}

/// Best honest reading of a User-Agent. Pure, so it is unit-testable and so
/// nothing about it can reach the database.
///
/// Anything it cannot identify comes back as None rather than as a guess: a
/// wrong device name on this screen is worse than no name, because the whole
/// point of the list is deciding whether a session is yours.
pub fn describe_device(ua: Option<&str>) -> DeviceLabel {
    let raw = js_trim(ua.unwrap_or(""));
    if raw.is_empty() {
        return DeviceLabel { browser: None, os: None, kind: DeviceKind::Unknown };
    }

    for (re, name) in CLIENTS.iter() {
        if let Some(m) = re.captures(raw) {
            let browser = match m.get(1).map(|v| v.as_str()).filter(|v| !v.is_empty()) {
                Some(version) => format!("{name} {version}"),
                None => (*name).to_string(),
            };
            return DeviceLabel { browser: Some(browser), os: os_of(raw), kind: DeviceKind::Client };
        }
    }

    let browser = BROWSERS.iter().find_map(|(re, name)| {
        re.captures(raw).map(|m| match m.get(1).map(|v| v.as_str()).filter(|v| !v.is_empty()) {
            Some(version) => format!("{name} {version}"),
            None => (*name).to_string(),
        })
    });

    let os = os_of(raw);
    let kind = kind_of(raw, os.as_deref());
    DeviceLabel { browser, os, kind }
}

/// `s.slice(0, n)` over UTF-16 code units. A cut through a surrogate pair
/// leaves a lone surrogate in JavaScript; a Rust string cannot hold one, so it
/// becomes U+FFFD (what the upstream string turns into once it is UTF-8).
pub fn js_slice(s: &str, n: usize) -> String {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        let w = c.len_utf16();
        if units + w > n {
            if units < n {
                // Half of a surrogate pair fits.
                return format!("{}\u{fffd}", &s[..i]);
            }
            return s[..i].to_string();
        }
        units += w;
    }
    s.to_string()
}

/// The view of one row (`listMySessions`' map).
fn view_of(row: &Session, current_id: Option<&str>) -> SessionView {
    let ua = row.user_agent.as_deref().filter(|ua| !ua.is_empty()).map(|ua| js_slice(ua, MAX_UA));
    let ip = row.ip_address.as_deref().map(js_trim).filter(|ip| !ip.is_empty()).map(|ip| js_slice(ip, 64));
    SessionView {
        id: row.id.clone(),
        current: Some(row.id.as_str()) == current_id,
        created_at: row.created_at.to_iso_string(),
        renewed_at: row.updated_at.to_iso_string(),
        expires_at: row.expires_at.to_iso_string(),
        ip,
        device: describe_device(ua.as_deref()),
        user_agent: ua,
        impersonated: row.impersonated_by.as_deref().is_some_and(|v| !v.is_empty()),
    }
}

/// The rows of Better Auth's `listSessions` as views, current one first, then
/// the most recently started sign-in (a stable sort, like `Array.prototype.sort`;
/// ISO strings of one format compare like `localeCompare` would).
pub fn session_views(rows: &[Session], current_id: Option<&str>) -> Vec<SessionView> {
    let mut views: Vec<SessionView> = rows.iter().map(|row| view_of(row, current_id)).collect();
    views.sort_by(|a, b| b.current.cmp(&a.current).then_with(|| b.created_at.cmp(&a.created_at)));
    views
}

/// Every live session on the caller's own account, current one first.
///
/// Better Auth hands back the session TOKEN as part of each row. It never
/// leaves this function: the token is the credential the cookie carries, so
/// shipping it to the browser would turn this screen into a way to copy a
/// sign-in out of the page source. The row id is enough to name a session, and
/// [`my_session_token`] turns it back into a token on the server when one is
/// actually needed.
pub async fn list_my_sessions(db: &PgPool, current: &AuthSession) -> sqlx::Result<Vec<SessionView>> {
    let rows = crate::auth::server::list_sessions(db, current).await?;
    Ok(session_views(&rows, Some(current.session.id.as_str())))
}

/// The Better Auth token for one of the CALLER'S OWN sessions, or None.
///
/// This is the ownership check for revocation, and it is structural rather
/// than a comparison somebody has to remember to write: the id is looked up in
/// the caller's own list, so an id belonging to another account simply is not
/// found. An administrator has no more reach here than anyone else.
pub async fn my_session_token(db: &PgPool, current: &AuthSession, id: &str) -> sqlx::Result<Option<String>> {
    if id.is_empty() {
        return Ok(None);
    }
    let rows = crate::auth::server::list_sessions(db, current).await?;
    Ok(rows.into_iter().find(|row| row.id == id).map(|row| row.token))
}

#[cfg(test)]
mod tests {
    //! tests/unit/sessions.test.ts (the Better Auth calls are the caller's;
    //! `session_views` is what `listMySessions` does with their answers).
    use super::*;
    use crate::db::types::JsDate;

    fn d(iso: &str) -> JsDate {
        JsDate::parse_iso(iso).unwrap()
    }

    /// A Better Auth session row, carrying the fields this module reads.
    fn row(id: &str) -> Session {
        Session {
            id: id.into(),
            token: format!("tok-{id}-secret"),
            user_id: "u1".into(),
            created_at: d("2026-08-01T10:00:00Z"),
            updated_at: d("2026-08-18T10:00:00Z"),
            expires_at: d("2026-08-25T10:00:00Z"),
            ip_address: Some("203.0.113.7".into()),
            user_agent: Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/141.0.0.0 Safari/537.36".into(),
            ),
            impersonated_by: None,
        }
    }

    fn label(browser: Option<&str>, os: Option<&str>, kind: DeviceKind) -> DeviceLabel {
        DeviceLabel { browser: browser.map(str::to_string), os: os.map(str::to_string), kind }
    }

    // describe("describeDevice")
    #[test]
    fn reads_chrome_on_windows_without_claiming_a_windows_version() {
        let d = describe_device(Some(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
        ));
        // Windows 10 and Windows 11 send the same token; picking one would be a lie.
        assert_eq!(d, label(Some("Chrome 141"), Some("Windows"), DeviceKind::Desktop));
    }

    #[test]
    fn does_not_report_edge_as_chrome() {
        let d = describe_device(Some(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0",
        ));
        assert_eq!(d.browser.as_deref(), Some("Edge 141"));
    }

    #[test]
    fn does_not_report_opera_as_chrome() {
        let d = describe_device(Some(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/140.0.0.0 Safari/537.36 OPR/125.0.0.0",
        ));
        assert_eq!(d.browser.as_deref(), Some("Opera 125"));
    }

    #[test]
    fn reads_safari_on_an_iphone_as_mobile() {
        let d = describe_device(Some(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1",
        ));
        assert_eq!(d, label(Some("Safari 18"), Some("iOS 18"), DeviceKind::Mobile));
    }

    #[test]
    fn separates_an_android_tablet_from_an_android_phone_by_the_mobile_token() {
        let phone = describe_device(Some(
            "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/141.0.0.0 Mobile Safari/537.36",
        ));
        let tablet = describe_device(Some(
            "Mozilla/5.0 (Linux; Android 15; SM-X710) AppleWebKit/537.36 Chrome/141.0.0.0 Safari/537.36",
        ));
        assert_eq!(phone.kind, DeviceKind::Mobile);
        assert_eq!(tablet.kind, DeviceKind::Tablet);
        assert_eq!(phone.os.as_deref(), Some("Android 15"));
    }

    #[test]
    fn names_a_non_browser_client_instead_of_the_browser_token_it_carries() {
        assert_eq!(describe_device(Some("ShareX/16.1.0")).browser.as_deref(), Some("ShareX 16.1.0"));
        assert_eq!(describe_device(Some("curl/8.9.1")), label(Some("curl 8.9.1"), None, DeviceKind::Client));
    }

    #[test]
    fn returns_nulls_rather_than_a_guess_for_a_string_it_cannot_read() {
        let unknown = label(None, None, DeviceKind::Unknown);
        assert_eq!(describe_device(Some("something-nobody-has-seen")), unknown);
        assert_eq!(describe_device(None), unknown);
        assert_eq!(describe_device(Some("   ")), unknown);
    }

    // Beyond the upstream file: the JavaScript regex details the port has to keep.
    #[test]
    fn javascript_regex_semantics() {
        // `/i` without `u` folds ASCII only: the Kelvin sign is not a `k`.
        assert_eq!(describe_device(Some("o\u{212a}http/4.12")).browser, None);
        assert_eq!(describe_device(Some("OKHTTP/4.12")).browser.as_deref(), Some("OkHttp 4.12"));
        // `\d` is ASCII digits only.
        assert_eq!(describe_device(Some("curl/\u{0663}")).browser, None);
        // A client token without a version, and one that is just dots.
        assert_eq!(describe_device(Some("modlight")).browser.as_deref(), Some("Modlight App"));
        assert_eq!(describe_device(Some("ModLight/.")).browser.as_deref(), Some("Modlight App ."));
        // `.*` in the lookahead stops at a line terminator.
        assert_eq!(describe_device(Some("Android 9\u{2028}Mobile")).kind, DeviceKind::Tablet);
        assert_eq!(describe_device(Some("Android 9 Mobile")).kind, DeviceKind::Mobile);
        // The second Android occurrence decides when the first is followed by Mobile.
        assert_eq!(describe_device(Some("Android Mobile Android")).kind, DeviceKind::Tablet);
        assert_eq!(
            describe_device(Some("Windows NT 6.1; rv:115.0) Gecko Firefox/115.0")).os.as_deref(),
            Some("Windows 7")
        );
        assert_eq!(
            describe_device(Some("Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) Chrome/120.0")),
            label(Some("Chrome 120"), Some("ChromeOS"), DeviceKind::Desktop)
        );
        assert_eq!(
            describe_device(Some(
                "Mozilla/5.0 (iPad; CPU OS 17_1 like Mac OS X) Version/17.1 Mobile/15E148 Safari/604.1"
            )),
            label(Some("Safari 17"), Some("iOS 17"), DeviceKind::Tablet)
        );
        assert_eq!(js_slice("a\u{1F600}b", 2), "a\u{fffd}");
        assert_eq!(js_slice("a\u{1F600}b", 3), "a\u{1F600}");
        assert_eq!(js_slice("abc", 300), "abc");
    }

    // describe("listMySessions")
    #[test]
    fn never_lets_a_session_token_reach_the_caller() {
        let mut s2 = row("s2");
        s2.token = "tok-s2-secret".into();
        let views = session_views(&[row("s1"), s2], Some("s1"));
        let payload = serde_json::to_string(&views).unwrap();
        assert!(!payload.contains("tok-s1-secret"));
        assert!(!payload.contains("tok-s2-secret"));
        assert!(views.iter().all(|v| !serde_json::to_value(v).unwrap().as_object().unwrap().contains_key("token")));
    }

    #[test]
    fn marks_exactly_the_session_doing_the_asking_and_sorts_it_first() {
        let mut s1 = row("s1");
        s1.created_at = d("2026-08-05T10:00:00Z");
        let mut s2 = row("s2");
        s2.created_at = d("2026-08-01T10:00:00Z");
        let views = session_views(&[s1, s2], Some("s2"));
        assert_eq!(views.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["s2", "s1"]);
        assert_eq!(views.iter().filter(|v| v.current).map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["s2"]);
    }

    #[test]
    fn orders_the_rest_newest_sign_in_first() {
        let mut old = row("old");
        old.created_at = d("2026-07-01T10:00:00Z");
        let mut new = row("new");
        new.created_at = d("2026-08-17T10:00:00Z");
        let views = session_views(&[row("s1"), old, new], Some("s1"));
        assert_eq!(views.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["s1", "new", "old"]);
    }

    #[test]
    fn marks_nothing_current_when_the_caller_has_no_session() {
        assert!(!session_views(&[row("s1")], None).iter().any(|v| v.current));
    }

    #[test]
    fn clips_an_absurd_user_agent() {
        let mut r = row("s1");
        r.user_agent = Some("A".repeat(5000));
        let views = session_views(&[r], Some("s1"));
        assert_eq!(views[0].user_agent.as_ref().unwrap().len(), 300);
    }

    #[test]
    fn reports_a_missing_or_blank_address_as_null_rather_than_as_an_empty_string() {
        let mut s1 = row("s1");
        s1.ip_address = Some("   ".into());
        let mut s2 = row("s2");
        s2.ip_address = None;
        let mut s3 = row("s3");
        s3.ip_address = Some(String::new());
        for v in session_views(&[s1, s2, s3], Some("s1")) {
            assert_eq!(v.ip, None);
        }
    }

    #[test]
    fn flags_a_session_opened_by_an_administrator_impersonating_the_account() {
        let mut s2 = row("s2");
        s2.impersonated_by = Some("admin-1".into());
        let views = session_views(&[row("s1"), s2], Some("s1"));
        let by_id = |id: &str| views.iter().find(|v| v.id == id).unwrap().impersonated;
        assert!(!by_id("s1"));
        assert!(by_id("s2"));
    }

    #[test]
    fn emits_iso_timestamps_for_all_three_moments() {
        let views = session_views(&[row("s1")], Some("s1"));
        let v = &views[0];
        assert_eq!(v.created_at, "2026-08-01T10:00:00.000Z");
        // updatedAt is a renewal stamp, not a last-seen time - see the module.
        assert_eq!(v.renewed_at, "2026-08-18T10:00:00.000Z");
        assert_eq!(v.expires_at, "2026-08-25T10:00:00.000Z");
    }

    #[test]
    fn serialises_in_the_upstream_shape() {
        let views = session_views(&[row("s1")], Some("s1"));
        assert_eq!(
            serde_json::to_value(&views[0]).unwrap(),
            serde_json::json!({
                "id": "s1",
                "current": true,
                "createdAt": "2026-08-01T10:00:00.000Z",
                "renewedAt": "2026-08-18T10:00:00.000Z",
                "expiresAt": "2026-08-25T10:00:00.000Z",
                "ip": "203.0.113.7",
                "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/141.0.0.0 Safari/537.36",
                "device": { "browser": "Chrome 141", "os": "Windows", "kind": "desktop" },
                "impersonated": false
            })
        );
    }

    // describe("mySessionToken") needs Better Auth's list; it runs against a
    // database in tests/account_db.rs.
}
