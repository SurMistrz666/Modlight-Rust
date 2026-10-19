//! Port of `lib/link-info/parse.ts`: taking a Modlight URL apart without
//! asking the database anything.
//!
//! The grammar is the one the app serves:
//!
//! ```text
//!   /{tag}/{code}[.ext]                 preview page
//!   /{tag}/{code}/raw[.ext]             the bytes
//!   /{tag}/{code}/download | /convert | /render
//!   /{code}[.ext], /{code}/raw[.ext]    the tagless forms
//!   /s/{code}, /s/{tag}/{name}          short links
//!   /a/{tag}/{slug}[/zip], /u/{tag}     galleries
//! ```
//!
//! Two rules are load-bearing: the extension is DECORATIVE (the code is cut at
//! the first dot), and `/{tag}/{code}` vs `/{code}/raw` are ambiguous by shape,
//! so both readings come back, tagged first, the order the app tries them in.

use url::Url;

use crate::config::{Config, js_trim};
use crate::http::cookies::decode_uri_component;
use crate::media::paths::code_from_media_code;
use crate::naming::is_reserved_tag;

/// Longest input we will even try to parse (UTF-16 units). A URL is not a payload.
pub const MAX_INPUT: usize = 2048;

/// The hosts a parse is judged against (`config.appUrl`, `appDomains`,
/// `shortUrl`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkHosts {
    pub app_url: String,
    pub app_domains: Vec<String>,
    pub short_url: String,
}

impl LinkHosts {
    pub fn of(config: &Config) -> LinkHosts {
        LinkHosts {
            app_url: config.app_url.clone(),
            app_domains: config.app_domains.clone(),
            short_url: config.short_url.clone(),
        }
    }

    /// `knownOrigins()`: every origin this deployment answers on, including
    /// the shortener domain.
    pub fn known_origins(&self) -> Vec<String> {
        let mut out = self.app_domains.clone();
        if !self.short_url.is_empty() && !out.iter().any(|o| o.to_lowercase() == self.short_url.to_lowercase()) {
            out.push(self.short_url.clone());
        }
        out
    }

    /// Host comparison ignores the scheme (a pasted http:// link is the same file).
    fn is_known_host(&self, host: &str) -> bool {
        let wanted = host.to_lowercase();
        self.known_origins().iter().any(|o| safe_url(o).is_some_and(|u| js_host(&u).to_lowercase() == wanted))
    }

    /// `shortHostname()`: the host where every path IS a short link, or `None`
    /// on the ordinary single-domain install (shortUrl falls back to APP_URL,
    /// and proxy.ts installs no rewrite then).
    fn short_hostname(&self) -> Option<String> {
        let short = safe_url(&self.short_url).and_then(|u| u.host_str().map(str::to_lowercase))?;
        let app = safe_url(&self.app_url).and_then(|u| u.host_str().map(str::to_lowercase));
        (Some(&short) != app.as_ref()).then_some(short)
    }
}

/// `MediaShape`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaShape {
    Preview,
    Raw,
    Download,
    Convert,
    Render,
}

impl MediaShape {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaShape::Preview => "preview",
            MediaShape::Raw => "raw",
            MediaShape::Download => "download",
            MediaShape::Convert => "convert",
            MediaShape::Render => "render",
        }
    }
}

/// `MediaRef`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    /// Owner tag taken from the URL; `None` when the URL carried no tag.
    pub tag: Option<String>,
    /// The bare code, extension already stripped.
    pub code: String,
    pub shape: MediaShape,
    /// Whatever followed the first dot, for reporting only.
    pub url_ext: String,
}

/// `LinkTarget`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Media { primary: MediaRef, fallback: Option<MediaRef> },
    ShortLink { stored: String, unlock_gate: bool },
    FolderGallery { tag: String, slug: String, zip: bool },
    UserGallery { tag: String },
    AppRoute { path: String, label: String },
}

/// `ParseRefusal`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseRefusal {
    MissingUrl,
    Unparseable,
    ForeignHost,
    UnsupportedShape,
    CapabilityUrl,
}

impl ParseRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            ParseRefusal::MissingUrl => "missing-url",
            ParseRefusal::Unparseable => "unparseable",
            ParseRefusal::ForeignHost => "foreign-host",
            ParseRefusal::UnsupportedShape => "unsupported-shape",
            ParseRefusal::CapabilityUrl => "capability-url",
        }
    }
}

/// `ParsedOk`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOk {
    /// Origin the URL was read on.
    pub origin: String,
    /// Path after the shortener-host rewrite, if any.
    pub path: String,
    pub search: String,
    pub target: LinkTarget,
    pub warnings: Vec<String>,
}

/// What a parse can come to: the link, a refusal, or the `URIError` upstream's
/// unguarded `codeFromMediaCode` throws on a malformed escape (a 500 there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Ok(Box<ParsedOk>),
    Refused { reason: ParseRefusal, message: String },
    Malformed,
}

fn refuse(reason: ParseRefusal, message: impl Into<String>) -> Parsed {
    Parsed::Refused { reason, message: message.into() }
}

/// `new URL(raw)` that answers `None` instead of throwing.
fn safe_url(raw: &str) -> Option<Url> {
    Url::parse(raw).ok()
}

/// WHATWG `url.host`: hostname plus a non-default port.
pub fn js_host(u: &Url) -> String {
    let host = u.host_str().unwrap_or("");
    match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    }
}

/// `decodeURIComponent(segment)`, or the segment itself when malformed.
fn decode(segment: &str) -> String {
    decode_uri_component(segment).unwrap_or_else(|| segment.to_string())
}

/// Everything after the FIRST dot (a leading dot is not an extension).
fn url_ext_of(segment: &str) -> String {
    let s = decode(segment);
    match s.find('.') {
        Some(dot) if dot > 0 => s[dot + 1..].to_string(),
        _ => String::new(),
    }
}

/// `/^raw\.[a-z0-9]+$/` on an already lowercased segment.
fn is_raw_with_ext(s: &str) -> bool {
    s.strip_prefix("raw.")
        .is_some_and(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn shape_of_action(segment: &str) -> Option<MediaShape> {
    let s = decode(segment).to_lowercase();
    if s == "raw" || is_raw_with_ext(&s) {
        return Some(MediaShape::Raw);
    }
    match s.as_str() {
        "download" => Some(MediaShape::Download),
        "convert" => Some(MediaShape::Convert),
        "render" => Some(MediaShape::Render),
        _ => None,
    }
}

/// `TAGLESS_ACTIONS`: `/^(raw(\.[a-z0-9]+)?|download|convert)$/i` (render is
/// not read taglessly by the app, so it is not here either).
fn is_tagless_action(segment: &str) -> bool {
    let s = segment.to_lowercase();
    // The `i` flag folds only ASCII here: `[a-z0-9]` after lowercasing.
    s == "raw" || s == "download" || s == "convert" || is_raw_with_ext(&s)
}

/// `SHORT_HOST_PASSTHROUGH`: paths that keep their own meaning on the
/// shortener host (proxy.ts PASSTHROUGH).
fn is_short_host_passthrough(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    ["_next", "api", "s/", "favicon", "icon", "apple-icon", "robots", "sitemap", ".well-known"]
        .iter()
        .any(|p| rest.starts_with(p))
}

/// A human label for a path that is a page of the application, not a file.
pub fn app_route_label(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    let trimmed = lower.trim_end_matches('/');
    let p = if trimmed.is_empty() { "/" } else { trimmed };
    match p {
        "/" => "the application home page",
        "/download" => "the desktop app download page",
        "/download/app" => "a redirect to the current desktop app build",
        "/api/releases/app" => "the desktop app release manifest (public JSON)",
        "/oembed" => "the oEmbed endpoint",
        "/api/link-info" => "this endpoint",
        _ if p.starts_with("/embed/") => "a drawn file-type card (names a category, never a file)",
        _ if p.starts_with("/avatar/") => "an account avatar",
        _ if p.starts_with("/bg/") => "an account wallpaper",
        _ if p.starts_with("/api/") => "an API endpoint",
        _ if p.starts_with("/dashboard") => "a panel page (requires a session)",
        _ => "a page of the application, not a file",
    }
}

/// `/^[a-z0-9.-]+(?::\d+)?$/i`
fn looks_like_host(first: &str) -> bool {
    let (host, port) = match first.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (first, None),
    };
    !host.is_empty()
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `/^([a-zA-Z][a-zA-Z0-9+.-]*):/`: the scheme, when the input starts with one.
fn scheme_of(input: &str) -> Option<&str> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    for (i, c) in chars {
        if c == ':' {
            return Some(&input[..i]);
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
            return None;
        }
    }
    None
}

/// `parseLinkUrl(raw)`: turn any string a caller pasted (full URL, bare path,
/// scheme-less host/path) into a target on a known origin.
pub fn parse_link_url(hosts: &LinkHosts, raw: Option<&str>) -> Parsed {
    let Some(raw) = raw.filter(|r| !js_trim(r).is_empty()) else {
        return refuse(ParseRefusal::MissingUrl, "Pass the URL to describe as the `url` query parameter.");
    };
    let input = js_trim(raw);
    if input.encode_utf16().count() > MAX_INPUT {
        return refuse(
            ParseRefusal::Unparseable,
            format!("That is longer than {MAX_INPUT} characters, so it is not a URL."),
        );
    }

    let url = if let Some(scheme) = scheme_of(input) {
        let proto = scheme.to_lowercase();
        if proto != "http" && proto != "https" {
            return refuse(
                ParseRefusal::Unparseable,
                format!("Only http and https URLs can be described, not {proto}:."),
            );
        }
        safe_url(input)
    } else if input.starts_with('/') {
        safe_url(&format!("{}{input}", hosts.app_url))
    } else {
        let first = input.split('/').next().unwrap_or("");
        if first.contains('.') && looks_like_host(first) {
            safe_url(&format!("https://{input}"))
        } else {
            safe_url(&format!("{}/{input}", hosts.app_url))
        }
    };
    let Some(url) = url else {
        return refuse(ParseRefusal::Unparseable, "That is not a URL this endpoint can read.");
    };
    let host = js_host(&url);
    if !hosts.is_known_host(&host) {
        return refuse(
            ParseRefusal::ForeignHost,
            format!("{host} is not a host of this deployment, so there is nothing here to describe."),
        );
    }

    let mut warnings = Vec::new();
    let mut path = url.path().to_string();
    if let Some(short) = hosts.short_hostname()
        && url.host_str().map(str::to_lowercase).as_deref() == Some(short.as_str())
        && path != "/"
        && !is_short_host_passthrough(&path)
    {
        path = format!("/s{path}");
        warnings.push(format!("{host} is the short link domain, where every path is a short link. Read as {path}."));
    }

    let target = match classify(&path) {
        Classified::Target(t) => t,
        Classified::Refused(reason, message) => return refuse(reason, message),
        Classified::Malformed => return Parsed::Malformed,
    };
    let search = match url.query() {
        Some(q) if !q.is_empty() => format!("?{q}"),
        _ => String::new(),
    };
    Parsed::Ok(Box::new(ParsedOk { origin: format!("{}://{host}", url.scheme()), path, search, target, warnings }))
}

enum Classified {
    Target(LinkTarget),
    Refused(ParseRefusal, String),
    Malformed,
}

fn classify(path: &str) -> Classified {
    use Classified::*;
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Target(LinkTarget::AppRoute { path: "/".into(), label: app_route_label("/").into() });
    }
    let no = |reason, message: &str| Refused(reason, message.to_string());
    let (a, b, c, d) = (segs[0], segs.get(1).copied(), segs.get(2).copied(), segs.get(3).copied());
    let code_of = |s: &str| code_from_media_code(s);

    // A deletion link is a capability, refused by shape before any lookup.
    if segs.len() == 4 && c.is_some_and(|c| decode(c).to_lowercase() == "delete") {
        return no(
            ParseRefusal::CapabilityUrl,
            "That is a deletion link. The token in it authorises destroying the file, so it is not described here. Do not POST to it.",
        );
    }

    if a == "s" {
        let rest = &segs[1..];
        if rest.is_empty() {
            return no(ParseRefusal::UnsupportedShape, "A short link URL needs a code.");
        }
        let unlock_gate = decode(rest[rest.len() - 1]).to_lowercase() == "unlock";
        let parts = if unlock_gate { &rest[..rest.len() - 1] } else { rest };
        if parts.is_empty() || parts.len() > 2 {
            return no(ParseRefusal::UnsupportedShape, "A short link is /s/{code} or /s/{tag}/{name}.");
        }
        let stored = parts.iter().map(|p| decode(p)).collect::<Vec<_>>().join("/");
        return Target(LinkTarget::ShortLink { stored, unlock_gate });
    }

    if a == "a" {
        let (Some(b), Some(c)) = (b, c) else {
            return no(ParseRefusal::UnsupportedShape, "A folder gallery is /a/{tag}/{slug}.");
        };
        return Target(LinkTarget::FolderGallery {
            tag: decode(b),
            slug: decode(c),
            zip: decode(d.unwrap_or("")).to_lowercase() == "zip",
        });
    }

    if a == "u" {
        let Some(b) = b else {
            return no(ParseRefusal::UnsupportedShape, "A profile gallery is /u/{tag}.");
        };
        return Target(LinkTarget::UserGallery { tag: decode(b) });
    }

    if is_reserved_tag(&decode(a)) {
        return Target(LinkTarget::AppRoute { path: path.to_string(), label: app_route_label(path).into() });
    }

    match segs.len() {
        1 => {
            let Some(code) = code_of(a) else { return Malformed };
            if code.is_empty() || a.starts_with('.') || is_reserved_tag(&code) {
                return no(ParseRefusal::UnsupportedShape, "That single path segment is not a media code.");
            }
            Target(LinkTarget::Media {
                primary: MediaRef { tag: None, code, shape: MediaShape::Preview, url_ext: url_ext_of(a) },
                fallback: None,
            })
        }
        2 => {
            let b = b.unwrap_or("");
            let Some(code) = code_of(b) else { return Malformed };
            let primary = MediaRef { tag: Some(decode(a)), code, shape: MediaShape::Preview, url_ext: url_ext_of(b) };
            let Some(tagless_code) = code_of(a) else { return Malformed };
            let fallback = (is_tagless_action(&decode(b))
                && !tagless_code.is_empty()
                && !is_reserved_tag(&tagless_code))
            .then(|| MediaRef {
                tag: None,
                code: tagless_code,
                shape: shape_of_action(b).unwrap_or(MediaShape::Raw),
                url_ext: url_ext_of(a),
            });
            Target(LinkTarget::Media { primary, fallback })
        }
        3 => {
            let c = c.unwrap_or("");
            let Some(shape) = shape_of_action(c) else {
                return Refused(
                    ParseRefusal::UnsupportedShape,
                    format!("\"{}\" is not an action this deployment serves.", decode(c)),
                );
            };
            let b = b.unwrap_or("");
            let Some(code) = code_of(b) else { return Malformed };
            Target(LinkTarget::Media {
                primary: MediaRef { tag: Some(decode(a)), code, shape, url_ext: url_ext_of(b) },
                fallback: None,
            })
        }
        _ => no(ParseRefusal::UnsupportedShape, "That path is deeper than any URL shape this deployment serves."),
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/link-info-parse.test.ts
    use super::*;

    const HOST: &str = "https://host.example.com";
    const H: &str = "https://h.example.com";
    const S: &str = "https://s.example.com";

    fn hosts(short: Option<&str>) -> LinkHosts {
        LinkHosts {
            app_url: HOST.into(),
            app_domains: vec![HOST.into(), H.into()],
            // config.shortUrl falls back to APP_URL when SHORTLINK_URL is unset.
            short_url: short.unwrap_or(HOST).into(),
        }
    }

    fn parse(url: &str) -> Parsed {
        parse_link_url(&hosts(Some(S)), Some(url))
    }

    fn ok(p: Parsed) -> ParsedOk {
        match p {
            Parsed::Ok(o) => *o,
            other => panic!("expected a parse, got {other:?}"),
        }
    }

    fn media(p: Parsed) -> (MediaRef, Option<MediaRef>) {
        match ok(p).target {
            LinkTarget::Media { primary, fallback } => (primary, fallback),
            other => panic!("expected media, got {other:?}"),
        }
    }

    fn reason(p: Parsed) -> ParseRefusal {
        match p {
            Parsed::Refused { reason, .. } => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_decorative_extension() {
        for spelling in ["romuz3", "romuz3.png", "romuz3.zzz", "romuz3.tar.gz", "romuz3."] {
            let (p, _) = media(parse(&format!("{H}/alicja/{spelling}")));
            assert_eq!((p.code.as_str(), p.tag.as_deref(), p.shape), ("romuz3", Some("alicja"), MediaShape::Preview));
        }
        let (p, _) = media(parse(&format!("{H}/alicja/romuz3.tar.gz")));
        assert_eq!((p.url_ext.as_str(), p.code.as_str()), ("tar.gz", "romuz3"));
    }

    #[test]
    fn the_shapes_the_deployment_serves() {
        for (segment, shape) in [
            ("raw", MediaShape::Raw),
            ("raw.png", MediaShape::Raw),
            ("download", MediaShape::Download),
            ("convert", MediaShape::Convert),
            ("render", MediaShape::Render),
        ] {
            assert_eq!(media(parse(&format!("{H}/alicja/romuz3/{segment}"))).0.shape, shape);
        }
        assert_eq!(reason(parse(&format!("{H}/alicja/romuz3/thumbnail"))), ParseRefusal::UnsupportedShape);
        let (p, _) = media(parse(&format!("{H}/romuz3.png")));
        assert_eq!((p.tag, p.code.as_str()), (None, "romuz3"));
        assert_eq!(
            ok(parse(&format!("{HOST}/s/promo"))).target,
            LinkTarget::ShortLink { stored: "promo".into(), unlock_gate: false }
        );
        assert_eq!(
            ok(parse(&format!("{HOST}/s/alicja/promo"))).target,
            LinkTarget::ShortLink { stored: "alicja/promo".into(), unlock_gate: false }
        );
        assert_eq!(
            ok(parse(&format!("{HOST}/s/alicja/promo/unlock"))).target,
            LinkTarget::ShortLink { stored: "alicja/promo".into(), unlock_gate: true }
        );
        assert_eq!(
            ok(parse(&format!("{HOST}/a/alicja/wakacje"))).target,
            LinkTarget::FolderGallery { tag: "alicja".into(), slug: "wakacje".into(), zip: false }
        );
        assert!(matches!(
            ok(parse(&format!("{HOST}/a/alicja/wakacje/zip"))).target,
            LinkTarget::FolderGallery { zip: true, .. }
        ));
        assert_eq!(ok(parse(&format!("{HOST}/u/alicja"))).target, LinkTarget::UserGallery { tag: "alicja".into() });
    }

    #[test]
    fn the_two_segment_ambiguity() {
        let (p, f) = media(parse(&format!("{H}/romuz3/raw.png")));
        assert_eq!((p.tag.as_deref(), p.code.as_str(), p.shape), (Some("romuz3"), "raw", MediaShape::Preview));
        let f = f.unwrap();
        assert_eq!((f.tag, f.code.as_str(), f.shape), (None, "romuz3", MediaShape::Raw));
        assert!(media(parse(&format!("{H}/alicja/romuz3.png"))).1.is_none());
        assert!(media(parse(&format!("{H}/romuz3/render"))).1.is_none());
    }

    #[test]
    fn reserved_names() {
        for path in ["/login", "/dashboard/files", "/api/uploads", "/oembed"] {
            assert!(matches!(ok(parse(&format!("{HOST}{path}"))).target, LinkTarget::AppRoute { .. }), "{path}");
        }
        assert!(!matches!(parse(&format!("{HOST}/login.png")), Parsed::Ok(_)));
        match ok(parse(&format!("{HOST}/download/app"))).target {
            LinkTarget::AppRoute { label, .. } => assert!(label.contains("desktop app")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_shortener_host() {
        let r = ok(parse(&format!("{S}/promo")));
        assert_eq!(r.target, LinkTarget::ShortLink { stored: "promo".into(), unlock_gate: false });
        assert_eq!(r.path, "/s/promo");
        assert!(r.warnings.join(" ").contains("short link domain"));
        let r = ok(parse(&format!("{S}/api/health")));
        assert_eq!(r.path, "/api/health");
        assert!(matches!(r.target, LinkTarget::AppRoute { .. }));
        assert!(matches!(ok(parse(&format!("{H}/promo"))).target, LinkTarget::Media { .. }));
        let single = hosts(None);
        let login = ok(parse_link_url(&single, Some(&format!("{HOST}/login"))));
        assert_eq!(login.path, "/login");
        assert!(matches!(login.target, LinkTarget::AppRoute { .. }));
        let file = ok(parse_link_url(&single, Some(&format!("{HOST}/alicja/romuz3.png"))));
        assert_eq!(file.path, "/alicja/romuz3.png");
        assert!(matches!(file.target, LinkTarget::Media { .. }));
        assert!(file.warnings.is_empty());
    }

    #[test]
    fn refusals() {
        match parse("https://evil.example.com/alicja/romuz3.png") {
            Parsed::Refused { reason, message } => {
                assert_eq!(reason, ParseRefusal::ForeignHost);
                assert!(message.contains("evil.example.com"));
            }
            other => panic!("{other:?}"),
        }
        for url in ["javascript:alert(1)", "data:text/html,<b>", "file:///etc/passwd"] {
            assert_eq!(reason(parse(url)), ParseRefusal::Unparseable, "{url}");
        }
        match parse(&format!("{H}/alicja/romuz3/delete/sometoken")) {
            Parsed::Refused { reason, message } => {
                assert_eq!(reason, ParseRefusal::CapabilityUrl);
                assert!(message.contains("Do not POST"));
            }
            other => panic!("{other:?}"),
        }
        assert!(!matches!(parse(""), Parsed::Ok(_)));
        assert!(!matches!(parse(&format!("{H}/{}", "a".repeat(4000))), Parsed::Ok(_)));
        assert_eq!(parse(&format!("{H}/%E0%A4%A")), Parsed::Malformed);
    }

    #[test]
    fn input_shapes_people_paste() {
        let r = ok(parse("/alicja/romuz3.png"));
        assert_eq!(r.origin, HOST);
        assert!(matches!(r.target, LinkTarget::Media { .. }));
        assert_eq!(ok(parse("h.example.com/alicja/romuz3.png")).origin, H);
        assert!(matches!(parse("http://h.example.com/alicja/romuz3.png"), Parsed::Ok(_)));
        assert_eq!(ok(parse(&format!("{H}/alicja/romuz3/convert?to=webp&check=1"))).search, "?to=webp&check=1");
    }
}
