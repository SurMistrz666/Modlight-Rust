//! Port of `lib/authorize/loopback.ts`: signing a desktop client in without
//! ever showing it a password.
//!
//! The ShareX fork needs an API token. The obvious way to get one is an
//! email + password form inside the app, and that is wrong here for a concrete
//! reason: Modlight has TOTP, so such a form breaks for exactly the accounts
//! that bothered to protect themselves. It also teaches people to type their
//! password into a native window, which is the habit every credential phish
//! depends on.
//!
//! So the browser does the authenticating, and the app only ever receives the
//! token: the loopback redirect of RFC 8252, which is what `gh auth login`,
//! `aws sso login` and every other desktop client already do:
//!
//!   1. the app binds an ephemeral port on 127.0.0.1 and starts listening;
//!   2. it opens `<host>/authorize?port=NNNNN&name=<client>&state=<random>`;
//!   3. the page requires a real session, says in plain language what is being
//!      handed over, and waits for an explicit click;
//!   4. on approve, the server mints a named token and the PAGE posts it to
//!      `http://127.0.0.1:<port>/`, body `{"state":"...","token":"..."}`,
//!      content-type `text/plain;charset=UTF-8`;
//!   5. the app checks `state` against the value it generated, keeps the token,
//!      answers 204 and stops listening.
//!
//! A refusal is delivered the same way, as `{"state":"...","error":"access_denied"}`,
//! so the app can stop waiting instead of hanging until its own timeout.
//!
//! `text/plain` is not sloppiness: it makes the POST a CORS-*simple* request,
//! so no preflight is sent and the twenty-line listener inside the desktop app
//! needs no CORS headers at all. The page uses `mode: "no-cors"` and never
//! reads the reply, so delivery is all it can learn, which is why the UI keeps
//! a manual fallback for the case where the listener is already gone.
//!
//! This module is the part with no I/O in it: what a request has to look like
//! before any of the above may happen.
//!
//! Query values arrive as JSON ([`QueryValue`]): a string, an array for a
//! repeated key (`SearchParams::to_value`), or anything an RPC caller sent.
//!
//! JavaScript strings are UTF-16, and `slice(0, CLIENT_NAME_MAX)` counts code
//! units, so it can cut an astral character in half and keep a lone high
//! surrogate. A Rust string cannot hold one: the sanitized name shows it as
//! U+FFFD (what a browser renders and what UTF-8 encoders write for it, so
//! `authorize_path` and the stored token name come out byte-identical), and
//! [`LoopbackRequest::cut_surrogate`] remembers that it happened, for the two
//! places where upstream behaves differently because of it.

use std::sync::LazyLock;

use serde::Serialize;
use serde::ser::SerializeStruct;
use serde_json::Value;

use crate::config::{is_js_whitespace, js_trim};

/// Ports below this are privileged; a loopback listener never binds one.
pub const MIN_PORT: u16 = 1024;
pub const MAX_PORT: u16 = 65535;

/// How much of a client's self-chosen name we are willing to render (UTF-16
/// code units, as `String.prototype.slice` counts).
pub const CLIENT_NAME_MAX: usize = 64;

/// `api_tokens.name` is varchar(120).
pub const TOKEN_NAME_MAX: usize = 120;

const STATE_MIN: usize = 8;
const STATE_MAX: usize = 128;

/// The query shape Next hands a page: a value, a repeated value, or nothing.
pub type QueryValue<'a> = Option<&'a Value>;

/// A validated authorization request, ready to be shown to a human.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackRequest {
    /// Loopback port, as a number, never as the string that arrived.
    pub port: u16,
    /// The client's nonce, echoed back untouched.
    pub state: String,
    /// Sanitized display name; empty when the client did not send a usable one.
    pub client_name: String,
    /// Where the token goes. Built here, from the port, and nowhere else.
    pub callback: String,
    /// `slice(0, CLIENT_NAME_MAX)` split a surrogate pair: upstream's name ends
    /// in a lone high surrogate, shown here as the final U+FFFD.
    #[serde(skip)]
    pub cut_surrogate: bool,
    /// Not upstream: the app's PKCE-style challenge for the server-side pickup
    /// ([`crate::authorize::pickup`]). Absent for older apps, and then left out
    /// of the JSON so their requests look exactly as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<String>,
}

impl LoopbackRequest {
    /// The name as upstream's next `sanitizeClientName` pass sees it: the lone
    /// surrogate is `\p{C}`, so it turns into a space and is trimmed away.
    pub fn client_name_for_token(&self) -> &str {
        if self.cut_surrogate {
            self.client_name.strip_suffix('\u{FFFD}').unwrap_or(&self.client_name)
        } else {
            &self.client_name
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorizeRequestIssue {
    InvalidPort,
    InvalidState,
    /// Not upstream: a `challenge` was sent and is not 43 base64url characters.
    InvalidChallenge,
}

/// `{ ok: true, request } | { ok: false, reason }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedAuthorizeRequest {
    Ok(LoopbackRequest),
    Invalid(AuthorizeRequestIssue),
}

impl ParsedAuthorizeRequest {
    pub fn request(&self) -> Option<&LoopbackRequest> {
        match self {
            ParsedAuthorizeRequest::Ok(r) => Some(r),
            ParsedAuthorizeRequest::Invalid(_) => None,
        }
    }
}

impl Serialize for ParsedAuthorizeRequest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("ParsedAuthorizeRequest", 2)?;
        match self {
            ParsedAuthorizeRequest::Ok(request) => {
                s.serialize_field("ok", &true)?;
                s.serialize_field("request", request)?;
            }
            ParsedAuthorizeRequest::Invalid(reason) => {
                s.serialize_field("ok", &false)?;
                s.serialize_field("reason", reason)?;
            }
        }
        s.end()
    }
}

/// One value, or nothing.
///
/// A repeated parameter (`?port=8080&port=1`) is refused rather than resolved.
/// No client sends one by accident, and picking "the first" is precisely how
/// two parties end up reading the same URL differently, which is the entire
/// trick behind parameter smuggling.
fn single(value: QueryValue<'_>) -> Option<&str> {
    match value {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// The loopback port, or `None`.
///
/// The spelling check runs BEFORE the number conversion because `Number()` is
/// far too forgiving for something that becomes part of a URL: it reads
/// `0x1f90`, `1e4`, `8080\n` and `+8080` as numbers, and each of those is a
/// string somebody chose to write that way. Four or five digits with no
/// leading zero (`/^[1-9][0-9]{3,4}$/`) is what a port looks like.
pub fn parse_loopback_port(raw: QueryValue<'_>) -> Option<u16> {
    let value = js_trim(single(raw)?);
    let bytes = value.as_bytes();
    let spelled_like_a_port = (4..=5).contains(&bytes.len())
        && (b'1'..=b'9').contains(&bytes[0])
        && bytes[1..].iter().all(u8::is_ascii_digit);
    if !spelled_like_a_port {
        return None;
    }
    let port: u32 = value.parse().ok()?;
    (u32::from(MIN_PORT)..=u32::from(MAX_PORT)).contains(&port).then_some(port as u16)
}

/// The client's nonce, or `None`.
///
/// Its job is to let the app tell its own request apart from one somebody
/// injected, so a one-character "state" is not one: a minimum length is part
/// of the contract, not politeness. The charset is the URL-unreserved set
/// (`/^[A-Za-z0-9._~-]{8,128}$/`) because this value is echoed into a JSON body
/// handed to a local listener and rendered on the page: no quotes, no angle
/// brackets, no newlines, nothing to escape anywhere downstream.
pub fn parse_state(raw: QueryValue<'_>) -> Option<String> {
    let value = js_trim(single(raw)?);
    let ok = (STATE_MIN..=STATE_MAX).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-'));
    ok.then(|| value.to_string())
}

/// `\p{C}`: controls, format characters (the bidirectional overrides and the
/// zero-width joiners among them), private use and unassigned code points.
static OTHER: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\p{C}").expect("valid regex"));

/// The sanitized name and whether the 64-unit cut split a surrogate pair.
fn sanitize(value: &str) -> (String, bool) {
    // .replace(/\p{C}/gu, " ")
    let replaced = OTHER.replace_all(value, " ");
    // .replace(/\s+/g, " ")
    let mut collapsed = String::with_capacity(replaced.len());
    let mut in_space = false;
    for c in replaced.chars() {
        if is_js_whitespace(c) {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    // .trim().slice(0, CLIENT_NAME_MAX)
    let mut out = String::new();
    let mut units = 0;
    let mut cut_surrogate = false;
    for c in js_trim(&collapsed).chars() {
        let n = c.len_utf16();
        if units + n > CLIENT_NAME_MAX {
            cut_surrogate = units < CLIENT_NAME_MAX;
            break;
        }
        out.push(c);
        units += n;
    }
    if cut_surrogate {
        // A trailing lone surrogate is not white space: the second trim has
        // nothing to remove at the end.
        out.push('\u{FFFD}');
        (out, true)
    } else {
        (js_trim(&out).to_string(), false)
    }
}

/// The name shown in the prompt, cleaned up.
///
/// Whoever wrote the link chose this string, and it is displayed inside a
/// permission dialog, which makes it the most valuable text on the page to an
/// attacker. React escapes markup, so the danger left is the characters that
/// change what the surrounding text MEANS: `\p{C}` covers the C0/C1 controls,
/// the zero-width joiners, and (the one that matters) the bidirectional
/// overrides, which can render "moc.live" as "evil.com" in a browser without a
/// single suspicious byte. Whitespace is collapsed for the same reason: a name
/// padded with fifty newlines can push the rest of the prompt off the screen.
///
/// Truncation is last, so the cap counts characters a person can actually see.
pub fn sanitize_client_name(raw: QueryValue<'_>) -> String {
    match single(raw) {
        None | Some("") => String::new(),
        Some(value) => sanitize(value).0,
    }
}

/// Why [`loopback_callback`] refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("loopbackCallback: refusing to build a callback for a non-loopback port")]
pub struct NotALoopbackPort;

/// The one place a callback URL is ever built.
///
/// Note the input: a NUMBER that has already been through
/// [`parse_loopback_port`], never the string from the query. That is the
/// whole open-redirect defence. Interpolating the raw parameter would accept
/// `8080@attacker.example` and `8080/../..@evil` as a host, and this URL is
/// where an API token gets posted: an open redirect here is a credential
/// handed to whoever wrote the link.
///
/// IPv4 loopback only. `localhost` resolves through the host's name resolution
/// (and can be pointed elsewhere), and `[::1]` is a second syntax to get wrong
/// for a listener that can always bind 127.0.0.1 instead.
pub fn loopback_callback(port: f64) -> Result<String, NotALoopbackPort> {
    if port.fract() != 0.0 || !(f64::from(MIN_PORT)..=f64::from(MAX_PORT)).contains(&port) {
        return Err(NotALoopbackPort);
    }
    Ok(format!("http://127.0.0.1:{}/", port as u16))
}

/// Length of a pickup challenge: base64url of a SHA-256 digest, no padding.
pub const CHALLENGE_LEN: usize = 43;

/// The pickup challenge: `Ok(None)` when none was sent (an older app),
/// `Err(())` when one was sent and is not exactly 43 base64url characters.
///
/// Malformed is a refusal rather than "no challenge": an app that sent one is
/// counting on the pickup, and quietly dropping it would turn a broken link
/// into a sign-in that waits ten minutes for a key that never comes.
#[allow(clippy::result_unit_err)]
pub fn parse_challenge(raw: QueryValue<'_>) -> Result<Option<String>, ()> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => {
            let ok = s.len() == CHALLENGE_LEN && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if ok { Ok(Some(s.clone())) } else { Err(()) }
        }
        Some(_) => Err(()),
    }
}

/// Validate a whole `/authorize` query without a pickup challenge (upstream's
/// signature).
pub fn parse_authorize_request(
    port: QueryValue<'_>,
    state: QueryValue<'_>,
    name: QueryValue<'_>,
) -> ParsedAuthorizeRequest {
    parse_authorize_request_with_challenge(port, state, name, None)
}

/// Validate a whole `/authorize` query. Both the page and the action use this.
pub fn parse_authorize_request_with_challenge(
    port: QueryValue<'_>,
    state: QueryValue<'_>,
    name: QueryValue<'_>,
    challenge: QueryValue<'_>,
) -> ParsedAuthorizeRequest {
    let Some(port) = parse_loopback_port(port) else {
        return ParsedAuthorizeRequest::Invalid(AuthorizeRequestIssue::InvalidPort);
    };
    let Some(state) = parse_state(state) else {
        return ParsedAuthorizeRequest::Invalid(AuthorizeRequestIssue::InvalidState);
    };
    let Ok(challenge) = parse_challenge(challenge) else {
        return ParsedAuthorizeRequest::Invalid(AuthorizeRequestIssue::InvalidChallenge);
    };
    let (client_name, cut_surrogate) = match single(name) {
        None | Some("") => (String::new(), false),
        Some(value) => sanitize(value),
    };
    ParsedAuthorizeRequest::Ok(LoopbackRequest {
        port,
        state,
        client_name,
        callback: loopback_callback(f64::from(port)).expect("a parsed port is a loopback port"),
        cut_surrogate,
        challenge,
    })
}

/// The link that reproduces this request, for `?next=` on the way to /login.
///
/// Rebuilt from the parsed values rather than passed through as the raw query
/// string: what we hand to the login page then comes from values this module
/// has already vouched for, and there is no second path by which an unchecked
/// parameter reaches the prompt. (`URLSearchParams#toString`: the
/// application/x-www-form-urlencoded serializer.)
pub fn authorize_path(request: &LoopbackRequest) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("port", &request.port.to_string());
    query.append_pair("state", &request.state);
    if !request.client_name.is_empty() {
        query.append_pair("name", &request.client_name);
    }
    // Carried through the sign-in round trip like the rest: losing it there
    // would leave the app polling for a pickup that can never happen.
    if let Some(challenge) = &request.challenge {
        query.append_pair("challenge", challenge);
    }
    format!("/authorize?{}", query.finish())
}

/// What the token is called in the Integrations list.
///
/// Prefixed, so a year later it is obvious the token was issued by a desktop
/// app asking for it rather than typed in by hand, and so a client that names
/// itself "ShareX" cannot sit in the list looking like the config the user
/// made themselves. Clipped to the column width because the name is
/// client-supplied and a value longer than varchar(120) is a failed INSERT,
/// i.e. a sign-in that mysteriously does not work.
pub fn token_name_for(client_name: &str) -> String {
    let name = if client_name.is_empty() { String::new() } else { sanitize(client_name).0 };
    if name.is_empty() {
        return "Desktop client".to_string();
    }
    // `.slice(0, TOKEN_NAME_MAX)`: the name is at most 64 code units, so this
    // never cuts anything; kept for the same reason upstream keeps it.
    let full = format!("Desktop: {name}");
    let mut out = String::new();
    let mut units = 0;
    for c in full.chars() {
        units += c.len_utf16();
        if units > TOKEN_NAME_MAX {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    //! tests/unit/authorize-loopback.test.ts
    use serde_json::json;

    use super::*;

    /// The invisible characters this file is about, spelled by code point: a
    /// test whose subject is a character you cannot see must not contain one.
    const RTL_OVERRIDE: char = '\u{202E}';
    const ZERO_WIDTH: char = '\u{200B}';
    const NBSP: char = '\u{00A0}';

    fn s(v: &str) -> Value {
        json!(v)
    }

    fn port(v: &str) -> Option<u16> {
        parse_loopback_port(Some(&s(v)))
    }

    fn state(v: &str) -> Option<String> {
        parse_state(Some(&s(v)))
    }

    fn name(v: &str) -> String {
        sanitize_client_name(Some(&s(v)))
    }

    // Not upstream: the pickup challenge.
    #[test]
    fn a_challenge_is_optional_exact_and_survives_the_sign_in_round_trip() {
        let c = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        let ok = parse_authorize_request_with_challenge(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), None, Some(&s(c)));
        let request = ok.request().unwrap();
        assert_eq!(request.challenge.as_deref(), Some(c));
        assert_eq!(serde_json::to_value(request).unwrap()["challenge"], c);
        assert_eq!(authorize_path(request), format!("/authorize?port=49732&state=s7Kd93jf0aQ2&challenge={c}"));

        // None, or empty: exactly the old request, without the key in the JSON.
        for none in [None, Some(s(""))] {
            let old = parse_authorize_request_with_challenge(
                Some(&s("49732")),
                Some(&s("s7Kd93jf0aQ2")),
                None,
                none.as_ref(),
            );
            assert_eq!(old, parse_authorize_request(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), None));
            assert!(serde_json::to_value(old.request().unwrap()).unwrap().get("challenge").is_none());
        }
        for bad in [s("short"), s(&"!".repeat(43)), s(&"a".repeat(44)), json!(["x"])] {
            assert_eq!(
                parse_authorize_request_with_challenge(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), None, Some(&bad)),
                ParsedAuthorizeRequest::Invalid(AuthorizeRequestIssue::InvalidChallenge)
            );
        }
    }

    // describe("loopback port")
    #[test]
    fn accepts_the_ephemeral_ports_a_desktop_listener_actually_binds() {
        assert_eq!(port("1024"), Some(MIN_PORT));
        assert_eq!(port("8420"), Some(8420));
        assert_eq!(port("65535"), Some(MAX_PORT));
        // Whitespace around a value is a copy-paste artefact, not an attack.
        assert_eq!(port(" 49732 "), Some(49732));
    }

    #[test]
    fn refuses_privileged_ports() {
        for p in ["22", "631", "1023", "0"] {
            assert_eq!(port(p), None, "{p}");
        }
    }

    #[test]
    fn refuses_every_spelling_of_a_number_that_is_not_plain_digits() {
        for p in ["0x1f90", "1e4", "+8080", "8080.0", "08080", "8_080"] {
            assert_eq!(port(p), None, "{p}");
        }
    }

    #[test]
    fn refuses_anything_that_is_not_a_port_at_all() {
        for p in ["", "65536", "999999", "8080@evil.example", "8080/../.."] {
            assert_eq!(port(p), None, "{p}");
        }
        assert_eq!(parse_loopback_port(None), None);
        // Not a string at all (an RPC caller can send anything).
        assert_eq!(parse_loopback_port(Some(&json!(8080))), None);
    }

    #[test]
    fn refuses_a_repeated_parameter_instead_of_picking_one() {
        assert_eq!(parse_loopback_port(Some(&json!(["8080", "1"]))), None);
        assert_eq!(parse_state(Some(&json!(["abcdefgh", "ijklmnop"]))), None);
    }

    // describe("state")
    #[test]
    fn accepts_an_opaque_url_safe_nonce() {
        assert_eq!(state("KZ9-x_2Q.tilde~ok").as_deref(), Some("KZ9-x_2Q.tilde~ok"));
        assert_eq!(state(&"a".repeat(128)).map(|v| v.len()), Some(128));
    }

    #[test]
    fn refuses_a_value_too_short_to_tell_two_requests_apart() {
        assert_eq!(state("1"), None);
        assert_eq!(state("abcdefg"), None);
        assert_eq!(state(&"a".repeat(129)), None);
    }

    #[test]
    fn refuses_characters_that_would_need_escaping_downstream() {
        for bad in ["abcdefgh\"", "abcdefgh<script>", "abcdefgh\nabcdefgh", "abcdefgh}", "abcd efgh", "abcdefgh%00"] {
            assert_eq!(state(bad), None, "{bad}");
        }
        assert_eq!(state("abcdefghé"), None);
    }

    // describe("client name")
    #[test]
    fn keeps_an_ordinary_name_in_any_script() {
        assert_eq!(name("Modlight Desktop"), "Modlight Desktop");
        assert_eq!(name("Modlight Pulpit"), "Modlight Pulpit");
        assert_eq!(name("Zażółć gęślą"), "Zażółć gęślą");
    }

    #[test]
    fn strips_bidirectional_overrides_and_other_invisibles() {
        assert_eq!(name(&format!("evil{RTL_OVERRIDE} moc.live")), "evil moc.live");
        assert_eq!(name(&format!("Mod{ZERO_WIDTH}light")), "Mod light");
        assert_eq!(name(&format!("Modlight{NBSP}")), "Modlight");
    }

    #[test]
    fn collapses_whitespace_so_a_name_cannot_push_the_prompt_off_screen() {
        assert_eq!(name("Modlight\n\n\n\n\n\n\nDesktop"), "Modlight Desktop");
    }

    #[test]
    fn truncates_to_a_length_a_person_can_read() {
        assert_eq!(name(&"x".repeat(500)).len(), CLIENT_NAME_MAX);
    }

    #[test]
    fn treats_a_missing_or_repeated_name_as_no_name_at_all() {
        assert_eq!(sanitize_client_name(None), "");
        assert_eq!(sanitize_client_name(Some(&json!(["a", "b"]))), "");
        assert_eq!(name("   "), "");
    }

    #[test]
    fn counts_utf16_code_units_like_slice() {
        // 63 units + an astral character: slice(0, 64) keeps its high surrogate.
        let raw = format!("{}\u{1F600}", "a".repeat(63));
        let parsed = parse_authorize_request(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), Some(&s(&raw)));
        let request = parsed.request().unwrap();
        assert!(request.cut_surrogate);
        assert_eq!(request.client_name, format!("{}\u{FFFD}", "a".repeat(63)));
        assert_eq!(request.client_name_for_token(), "a".repeat(63));
        // 62 units + the astral character fits exactly.
        let fits = format!("{}\u{1F600}", "a".repeat(62));
        assert_eq!(name(&fits), fits);
        // A white space before the cut stays inside the name.
        let spaced = format!("{} \u{1F600}", "a".repeat(62));
        assert_eq!(name(&spaced), format!("{} \u{FFFD}", "a".repeat(62)));
    }

    // describe("callback URL")
    #[test]
    fn is_built_from_the_parsed_number_so_it_can_only_ever_be_loopback() {
        assert_eq!(loopback_callback(49732.0).unwrap(), "http://127.0.0.1:49732/");
    }

    #[test]
    fn throws_rather_than_emit_a_url_for_a_port_that_never_passed_the_parser() {
        for p in [80.0, 70000.0, 8080.5, f64::NAN] {
            assert!(loopback_callback(p).is_err(), "{p}");
        }
    }

    // describe("whole request")
    #[test]
    fn accepts_a_well_formed_one_and_reports_what_it_will_do() {
        let parsed = parse_authorize_request(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), Some(&s("Modlight Desktop")));
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            json!({
                "ok": true,
                "request": {
                    "port": 49732,
                    "state": "s7Kd93jf0aQ2",
                    "clientName": "Modlight Desktop",
                    "callback": "http://127.0.0.1:49732/"
                }
            })
        );
    }

    #[test]
    fn still_accepts_a_request_with_no_client_name() {
        let parsed = parse_authorize_request(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), None);
        assert_eq!(parsed.request().unwrap().client_name, "");
    }

    #[test]
    fn names_which_half_of_the_request_was_wrong() {
        assert_eq!(
            serde_json::to_value(parse_authorize_request(Some(&s("80")), Some(&s("s7Kd93jf0aQ2")), None)).unwrap(),
            json!({ "ok": false, "reason": "invalid-port" })
        );
        assert_eq!(
            serde_json::to_value(parse_authorize_request(Some(&s("49732")), Some(&s("no")), None)).unwrap(),
            json!({ "ok": false, "reason": "invalid-state" })
        );
    }

    // describe("round trip through the sign-in redirect")
    #[test]
    fn rebuilds_a_link_that_parses_back_to_the_same_request() {
        let original =
            parse_authorize_request(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), Some(&s("Modlight Desktop")));
        let path = authorize_path(original.request().unwrap());
        let query = path.split_once('?').unwrap().1;
        let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        let get = |k: &str| pairs.iter().find(|(key, _)| key == k).map(|(_, v)| json!(v));
        let (p, st, n) = (get("port"), get("state"), get("name"));
        let again = parse_authorize_request(p.as_ref(), st.as_ref(), n.as_ref());
        assert_eq!(again, original);
    }

    #[test]
    fn is_a_relative_in_app_path_which_is_what_login_will_accept() {
        let parsed = parse_authorize_request(Some(&s("49732")), Some(&s("s7Kd93jf0aQ2")), None);
        let path = authorize_path(parsed.request().unwrap());
        assert!(path.starts_with("/authorize?"));
        assert!(!path.starts_with("//"));
        assert_eq!(path, "/authorize?port=49732&state=s7Kd93jf0aQ2");
    }

    #[test]
    fn serializes_the_name_like_url_search_params() {
        let parsed =
            parse_authorize_request(Some(&s("8080")), Some(&s("abc~def.gh_-")), Some(&s("My App\u{202E}!*'()~")));
        assert_eq!(
            authorize_path(parsed.request().unwrap()),
            "/authorize?port=8080&state=abc%7Edef.gh_-&name=My+App+%21*%27%28%29%7E"
        );
    }

    // describe("token name")
    #[test]
    fn marks_where_the_token_came_from() {
        assert_eq!(token_name_for("Modlight Desktop"), "Desktop: Modlight Desktop");
        assert_eq!(token_name_for(""), "Desktop client");
    }

    #[test]
    fn fits_the_column_whatever_the_client_calls_itself() {
        assert!(token_name_for(&"y".repeat(400)).encode_utf16().count() <= TOKEN_NAME_MAX);
    }

    #[test]
    fn sanitizes_again_because_the_name_is_client_supplied_on_this_path_too() {
        assert_eq!(token_name_for(&format!("She{RTL_OVERRIDE}ll")), "Desktop: She ll");
    }
}
