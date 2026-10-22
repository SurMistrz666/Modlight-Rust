//! Turning what yt-dlp printed into something a person can act on (`YtDlpErrorKind`,
//! `YtDlpErrorReader`, `YtDlpError`, `YtDlpFailure`).
//!
//! The reading is pure and ordered: the most specific cause wins, the "report this bug"
//! block yt-dlp attaches to unexpected errors is read LAST and only ever as "yt-dlp
//! blames itself" - whether that means "out of date" needs a version number
//! (`version_check`). Anything unrecognised keeps yt-dlp's own words.

use crate::l10n::{Arg, format, s, t};

/// What kind of failure yt-dlp reported, before anybody words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Unknown,
    BotCheck,
    AgeRestricted,
    GeoBlocked,
    IpBlocked,
    NeedsAccount,
    Gone,
    Unavailable,
    NoMedia,
    RateLimited,
    Unsupported,
    Network,
    Forbidden,
    SelfReportedBug,
}

/// How the yt-dlp on this computer stands against the newest published build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Standing {
    /// Not asked, or the answer did not arrive. Nothing may be claimed either way.
    #[default]
    Unknown,
    Behind,
    Current,
}

/// The standing and both numbers behind it (`YtDlpVersionFacts`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionFacts {
    pub standing: Standing,
    pub installed: String,
    pub latest: String,
}

impl VersionFacts {
    pub fn unchecked() -> VersionFacts {
        VersionFacts::default()
    }
}

/// Longer than this and the message stops being read.
const MAX_LENGTH: usize = 300;

const AGE_RESTRICTED: &[&str] = &[
    "confirm your age",
    "age-restricted",
    "age restricted",
    "inappropriate for some users",
];

const BOT_CHECK: &[&str] = &[
    "confirm you're not a bot",
    "confirm you are not a bot",
    "sign in to confirm",
];

const GEO_BLOCKED: &[&str] = &[
    "not available in your country",
    "not available in your region",
    "not available from your location",
    "made this video available in your country",
    "blocked it in your country",
    "blocked in your country",
    "geo restricted",
    "geo-restricted",
    "region locked",
    "region-locked",
];

const IP_BLOCKED: &[&str] = &["ip address is blocked", "blocked from accessing this post"];

const NEEDS_ACCOUNT: &[&str] = &[
    "private video",
    "video is private",
    "account is private",
    "this post is private",
    "members-only",
    "join this channel",
    "login required",
    "log into an account that has access",
    "log in for access",
    "requested format is not available",
    "sign in to view",
    "sign in if you've been granted access",
    "--cookies",
];

const GONE: &[&str] = &[
    "video unavailable",
    "has been removed",
    "no longer available",
    "has been deleted",
    "account has been terminated",
    "http error 404",
    "httperror 404",
    "error 404",
    "404: not found",
];

const UNAVAILABLE: &[&str] = &[
    "video not available, status code",
    "video not available",
    "video is unavailable",
    "this video is not available",
    "post is not available",
    "content isn't available",
    "content is not available",
    "media is not available",
];

const NO_MEDIA: &[&str] = &[
    "no video could be found",
    "there is no video in this",
    "there's no video in this",
    "no video formats found",
    "no media found",
];

const RATE_LIMITED: &[&str] = &[
    "too many requests",
    "http error 429",
    "httperror 429",
    "429: too many requests",
    "error 429",
    "rate-limit",
    "rate limit",
];

const UNSUPPORTED: &[&str] = &["unsupported url", "is not a valid url"];

const NETWORK: &[&str] = &[
    "failed to resolve",
    "getaddrinfo failed",
    "name or service not known",
    "temporary failure in name resolution",
    "connection refused",
    "connection reset",
    "connection aborted",
    "network is unreachable",
    "read operation timed out",
    "connection timed out",
    "certificate verify failed",
    "unable to connect to proxy",
    "transporterror",
    // The macOS spelling of a failed lookup (Python's socket.gaierror on Darwin).
    "nodename nor servname provided",
];

const FORBIDDEN: &[&str] = &[
    "http error 403",
    "httperror 403",
    "403: forbidden",
    "403 forbidden",
];

const SELF_REPORTED_BUG: &[&str] = &[
    "confirm you are on the latest version",
    "confirm you're on the latest version",
    "please report this issue on",
    "unexpected response from webpage",
    "unable to extract",
    "cannot parse data",
    "nsig extraction failed",
    "signature extraction failed",
];

fn contains(probe: &str, markers: &[&str]) -> bool {
    markers.iter().any(|m| probe.contains(m))
}

/// Which kind of failure this is (`YtDlpErrorReader.Classify`).
pub fn classify(raw: &str) -> ErrorKind {
    let line = clean(raw);
    if line.is_empty() {
        return ErrorKind::Unknown;
    }
    let probe = line.to_lowercase();
    // The raw line: the transport failure, the status code and the report block all
    // live in the parts `clean` cuts.
    let full = raw.to_lowercase();
    if contains(&probe, AGE_RESTRICTED) {
        ErrorKind::AgeRestricted
    } else if contains(&probe, BOT_CHECK) {
        ErrorKind::BotCheck
    } else if contains(&probe, GEO_BLOCKED) {
        ErrorKind::GeoBlocked
    } else if contains(&probe, IP_BLOCKED) {
        ErrorKind::IpBlocked
    } else if contains(&probe, NEEDS_ACCOUNT) {
        ErrorKind::NeedsAccount
    } else if contains(&probe, GONE) {
        ErrorKind::Gone
    } else if contains(&probe, UNAVAILABLE) {
        ErrorKind::Unavailable
    } else if contains(&probe, NO_MEDIA) {
        ErrorKind::NoMedia
    } else if contains(&probe, RATE_LIMITED) {
        ErrorKind::RateLimited
    } else if contains(&probe, UNSUPPORTED) {
        ErrorKind::Unsupported
    } else if contains(&full, NETWORK) {
        ErrorKind::Network
    } else if contains(&full, FORBIDDEN) {
        ErrorKind::Forbidden
    } else if contains(&full, SELF_REPORTED_BUG) {
        ErrorKind::SelfReportedBug
    } else {
        ErrorKind::Unknown
    }
}

/// Whether finding out how old this yt-dlp is could change what the reader is told.
pub fn wants_version_check(kind: ErrorKind) -> bool {
    kind == ErrorKind::SelfReportedBug
}

/// Whether the row should offer to fetch a newer yt-dlp.
pub fn offers_update(kind: ErrorKind, standing: Standing) -> bool {
    kind == ErrorKind::SelfReportedBug && standing != Standing::Current
}

/// yt-dlp's line without the parts that are for us: the `ERROR:` prefix and the
/// `(caused by ...)` tail; at most 300 characters.
pub fn clean(raw: &str) -> String {
    let mut line = raw.trim();
    if super::run::starts_with_ignore_case(line, "ERROR:") {
        line = line[6..].trim();
    }
    let lower = line.to_lowercase();
    let mut line = match lower.find("(caused by") {
        Some(at) if at > 0 && line.is_char_boundary(at) => line[..at].trim().to_string(),
        _ => line.to_string(),
    };
    if line.chars().count() > MAX_LENGTH {
        line = line.chars().take(MAX_LENGTH).collect::<String>();
        line = line.trim_end().to_string();
        line.push('…');
    }
    line
}

/// A failure in the three things the queue needs to know about one (`YtDlpFailure`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    /// A YouTube sign-in would plausibly fix this, so the row offers the button.
    pub needs_sign_in: bool,
    /// A newer yt-dlp is worth fetching for this failure.
    pub needs_update: bool,
}

impl Failure {
    fn new(message: impl Into<String>, needs_sign_in: bool) -> Failure {
        Failure {
            message: message.into(),
            needs_sign_in,
            needs_update: false,
        }
    }
}

/// What went wrong, worded, and what the row should offer (`YtDlpError.Describe`).
pub fn describe(
    kind: ErrorKind,
    raw: &str,
    youtube: bool,
    signed_in: bool,
    version: &VersionFacts,
) -> Failure {
    match kind {
        ErrorKind::BotCheck => sign_in(t(s::Download_ErrorBotCheck), youtube, signed_in),
        ErrorKind::AgeRestricted => {
            if youtube {
                Failure::new(t(s::Download_ErrorAgeRestrictedSignIn), true)
            } else {
                Failure::new(t(s::Download_ErrorAgeRestricted), false)
            }
        }
        ErrorKind::GeoBlocked => Failure::new(t(s::Download_ErrorGeo), false),
        ErrorKind::IpBlocked => Failure::new(t(s::Download_ErrorIpBlocked), false),
        ErrorKind::NeedsAccount => sign_in(t(s::Download_ErrorNeedsAccount), youtube, signed_in),
        ErrorKind::Gone => Failure::new(t(s::Download_ErrorGone), false),
        ErrorKind::Unavailable => Failure::new(t(s::Download_ErrorUnavailable), false),
        ErrorKind::NoMedia => Failure::new(t(s::Download_ErrorNoMedia), false),
        ErrorKind::RateLimited => Failure::new(t(s::Download_ErrorRateLimited), false),
        ErrorKind::Unsupported => Failure::new(t(s::Download_ErrorUnsupported), false),
        ErrorKind::Network => Failure::new(t(s::Download_ErrorNetwork), false),
        ErrorKind::Forbidden => sign_in(t(s::Download_ErrorForbidden), youtube, signed_in),
        ErrorKind::SelfReportedBug => self_reported_bug(raw, version),
        ErrorKind::Unknown => {
            let line = clean(raw);
            if line.is_empty() {
                Failure::new(t(s::Download_ErrorUnknown), false)
            } else {
                Failure::new(format(t(s::Download_ErrorRaw), &[Arg::from(line)]), false)
            }
        }
    }
}

fn self_reported_bug(raw: &str, version: &VersionFacts) -> Failure {
    let line = clean(raw);
    let needs_update = offers_update(ErrorKind::SelfReportedBug, version.standing);
    let message = match version.standing {
        Standing::Behind => format(
            t(s::Download_ErrorToolBehind),
            &[
                Arg::from(version.installed.as_str()),
                Arg::from(version.latest.as_str()),
            ],
        ),
        Standing::Current => format(
            t(s::Download_ErrorToolCurrent),
            &[Arg::from(version.installed.as_str()), Arg::from(line)],
        ),
        Standing::Unknown => format(t(s::Download_ErrorToolUnchecked), &[Arg::from(line)]),
    };
    Failure {
        message,
        needs_sign_in: false,
        needs_update,
    }
}

/// Off YouTube the generic sentence stands and no button is offered; on YouTube the
/// message names the remedy, which depends on whether a session was already in play.
fn sign_in(generic: &str, youtube: bool, signed_in: bool) -> Failure {
    if !youtube {
        return Failure::new(generic, false);
    }
    Failure::new(
        if signed_in {
            t(s::Download_ErrorSignInStale)
        } else {
            t(s::Download_ErrorSignIn)
        },
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_messages_land_in_their_class() {
        let cases = [
            (
                "ERROR: [youtube] Tq92D6wQ1mg: Sign in to confirm your age. Use --cookies-from-browser or --cookies for the authentication.",
                ErrorKind::AgeRestricted,
            ),
            (
                "ERROR: [youtube] abc: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies",
                ErrorKind::BotCheck,
            ),
            (
                "ERROR: [TikTok] 1234567890123456789: Your IP address is blocked from accessing this post",
                ErrorKind::IpBlocked,
            ),
            (
                "ERROR: [TikTok] 7679863973902077206: Video not available, status code 10240; please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template. Confirm you are on the latest version using  yt-dlp -U",
                ErrorKind::Unavailable,
            ),
            (
                "ERROR: [youtube] AAAAAAAAAAA: This video is unavailable",
                ErrorKind::Unavailable,
            ),
            (
                "ERROR: [youtube] x: Video unavailable. This video has been removed by the uploader",
                ErrorKind::Gone,
            ),
            (
                "ERROR: [twitter] 1234567890123456789: No video could be found in this tweet",
                ErrorKind::NoMedia,
            ),
            (
                "ERROR: [generic] 1: Unable to download webpage: HTTPSConnection(host='example.invalid', port=443): Failed to resolve 'example.invalid' ([Errno 8] nodename nor servname provided, or not known) (caused by TransportError('...'))",
                ErrorKind::Network,
            ),
            (
                "ERROR: [facebook] 1234567890: Cannot parse data; please report this issue on https://github.com/yt-dlp/yt-dlp/issues",
                ErrorKind::SelfReportedBug,
            ),
            (
                "ERROR: unable to download video data: HTTP Error 403: Forbidden",
                ErrorKind::Forbidden,
            ),
            (
                "ERROR: [youtube] x: Private video. Sign in if you've been granted access to this video",
                ErrorKind::NeedsAccount,
            ),
            (
                "ERROR: [youtube] x: The uploader has not made this video available in your country",
                ErrorKind::GeoBlocked,
            ),
            (
                "ERROR: HTTP Error 429: Too Many Requests",
                ErrorKind::RateLimited,
            ),
            (
                "ERROR: Unsupported URL: https://example.com/",
                ErrorKind::Unsupported,
            ),
            ("ERROR: something entirely new", ErrorKind::Unknown),
            ("", ErrorKind::Unknown),
        ];
        for (line, kind) in cases {
            assert_eq!(classify(line), kind, "{line}");
        }
    }

    #[test]
    fn a_number_inside_an_id_is_not_a_status() {
        assert_eq!(
            classify("ERROR: [TikTok] 7404291234294299: Something odd"),
            ErrorKind::Unknown
        );
    }

    #[test]
    fn the_line_loses_its_prefix_and_its_python_tail() {
        assert_eq!(
            clean("ERROR: [x] y: Nope (caused by HTTPError 403)"),
            "[x] y: Nope"
        );
        let long = format!("ERROR: {}", "a".repeat(400));
        let cleaned = clean(&long);
        assert_eq!(cleaned.chars().count(), 301);
        assert!(cleaned.ends_with('…'));
    }

    #[test]
    fn the_sign_in_is_offered_only_for_youtube() {
        let facts = VersionFacts::unchecked();
        let yt = describe(ErrorKind::BotCheck, "x", true, false, &facts);
        assert!(yt.needs_sign_in);
        assert_eq!(yt.message, t(s::Download_ErrorSignIn));
        let stale = describe(ErrorKind::BotCheck, "x", true, true, &facts);
        assert_eq!(stale.message, t(s::Download_ErrorSignInStale));
        let other = describe(ErrorKind::BotCheck, "x", false, false, &facts);
        assert!(!other.needs_sign_in);
        assert_eq!(other.message, t(s::Download_ErrorBotCheck));
        let age = describe(ErrorKind::AgeRestricted, "x", true, true, &facts);
        assert!(age.needs_sign_in);
        assert_eq!(age.message, t(s::Download_ErrorAgeRestrictedSignIn));
    }

    #[test]
    fn the_update_is_offered_unless_the_build_is_known_to_be_current() {
        let raw = "ERROR: [x] y: Unable to extract data; please report this issue on ...";
        let behind = VersionFacts {
            standing: Standing::Behind,
            installed: "2026.06.14".into(),
            latest: "2026.08.19".into(),
        };
        let failure = describe(ErrorKind::SelfReportedBug, raw, false, false, &behind);
        assert!(failure.needs_update && !failure.needs_sign_in);
        assert!(failure.message.contains("2026.06.14") && failure.message.contains("2026.08.19"));
        let current = VersionFacts {
            standing: Standing::Current,
            ..behind.clone()
        };
        assert!(!describe(ErrorKind::SelfReportedBug, raw, false, false, &current).needs_update);
        assert!(
            describe(
                ErrorKind::SelfReportedBug,
                raw,
                false,
                false,
                &VersionFacts::unchecked()
            )
            .needs_update
        );
        assert!(!offers_update(ErrorKind::Gone, Standing::Behind));
        assert!(wants_version_check(ErrorKind::SelfReportedBug));
        assert!(!wants_version_check(ErrorKind::Gone));
    }

    #[test]
    fn an_unknown_failure_keeps_yt_dlps_words() {
        let failure = describe(
            ErrorKind::Unknown,
            "ERROR: brand new failure",
            true,
            false,
            &VersionFacts::unchecked(),
        );
        assert!(failure.message.contains("brand new failure"));
        assert_eq!(
            describe(
                ErrorKind::Unknown,
                "",
                true,
                false,
                &VersionFacts::unchecked()
            )
            .message,
            t(s::Download_ErrorUnknown)
        );
    }
}
