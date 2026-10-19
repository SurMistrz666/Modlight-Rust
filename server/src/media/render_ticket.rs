//! Port of `lib/media/render-ticket.ts`: authorization for the sandboxed
//! HTML/SVG preview.
//!
//! /raw stays what it has always been: uploaded markup is served as text/plain,
//! inert whatever it contains. Rendering it for real happens on a separate route
//! (/{tag}/{code}/render), inside an iframe with no allow-same-origin and a CSP
//! `sandbox`, and only when the viewer asked for it by clicking.
//!
//! The ticket is what stops that route from becoming a bypass of the gates the
//! preview page enforces. The page is rendered only AFTER published/password/
//! expiry/view-budget have all been cleared, so the page is the right place to
//! issue proof that they were - a short-lived HMAC over the upload id, with no
//! server-side state to keep. The render route re-checks every gate anyway; the
//! ticket is what makes a hand-crafted /render URL useless on its own.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Mac;

use crate::auth::crypto::constant_time_eq;
use crate::config::{js_number, js_trim};
use crate::http::cookies::encode_uri_component;
use crate::media::part_a::{DocxKind, MAX_DOCX_BYTES, docx_support};

/// Ticket lifetime. Long enough to click the toggle, short enough to not travel.
pub const RENDER_TICKET_TTL: i64 = 15 * 60;

/// Largest file offered for rendering. A preview is a look at a page, not a way
/// to stream a multi-megabyte bundle through an iframe.
pub const MAX_RENDER_BYTES: i64 = 5 * 1024 * 1024;

/// What a rendered document is allowed to be.
///
/// `docx` is the odd one out and deliberately so: the other two serve the
/// uploaded BYTES, while docx serves markup we generate from them with mammoth.
/// It is in this table rather than in a mechanism of its own because the property
/// that matters is identical - it is HTML derived from a file somebody else
/// uploaded, so it belongs in an opaque origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxShape {
    Html,
    Svg,
    Docx,
}

impl SandboxShape {
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxShape::Html => "html",
            SandboxShape::Svg => "svg",
            SandboxShape::Docx => "docx",
        }
    }
}

/// Allowlist, not a blocklist: everything here executes script in the origin it
/// is served from, which is exactly why it may only ever be served into an opaque
/// one. Nothing else is renderable - notably NOT multipart/related (.mhtml),
/// which can smuggle a whole site into one file.
///
/// (Upstream looks this up in a plain object, so a stored type spelled like an
/// `Object.prototype` member - `constructor`, `toString` - came back truthy and
/// was offered the toggle as markup. Only the four real keys match here.)
fn renderable(mime: &str) -> Option<SandboxShape> {
    match mime {
        "text/html" | "application/xhtml+xml" => Some(SandboxShape::Html),
        "image/svg+xml" => Some(SandboxShape::Svg),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some(SandboxShape::Docx),
        _ => None,
    }
}

/// Shapes that must never be granted `allow-scripts` on their iframe.
///
/// SVG because script in a drawing is essentially always an attack, and docx
/// because a converted document has nothing to run. Sandbox flags from the
/// iframe attribute and from the response policy INTERSECT, so this cannot be
/// loosened by the config-level CSP that grants allow-scripts for uploaded HTML.
pub fn allows_scripts(shape: SandboxShape) -> bool {
    shape == SandboxShape::Html
}

fn normalize_mime(mime: &str) -> String {
    js_trim(mime.split(';').next().unwrap_or("")).to_lowercase()
}

/// `sandboxShape(mime, ext)`: the shape for a renderable type, None for
/// everything else.
pub fn sandbox_shape(mime: Option<&str>, ext: Option<&str>) -> Option<SandboxShape> {
    if let Some(shape) = mime.filter(|m| !m.is_empty()).and_then(|m| renderable(&normalize_mime(m))) {
        return Some(shape);
    }
    // Only the document case falls back to the extension. Uploaded HTML and SVG
    // are decided by their stored type alone, on purpose: promoting a file to
    // "executable markup" on the strength of its NAME would let a .html suffix
    // turn any upload into a page, and those bytes are deliberately served as
    // inert text/plain everywhere else.
    (docx_support(mime, ext) == Some(DocxKind::Docx)).then_some(SandboxShape::Docx)
}

/// `canRenderInline(media)`: may this upload be offered a "render" toggle at all?
///
/// `ext` is consulted because the stored type is so often useless: a .docx
/// stored as octet-stream is still a .docx. The size ceiling is per shape: a
/// document is a ZIP that expands, so it answers to its own limit.
pub fn can_render_inline(mime_type: &str, ext: Option<&str>, size: i64) -> bool {
    let Some(shape) = sandbox_shape(Some(mime_type), ext) else {
        return false;
    };
    size <= if shape == SandboxShape::Docx { MAX_DOCX_BYTES } else { MAX_RENDER_BYTES }
}

// ── Ticket ──────────────────────────────────────────────────────────────────

fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

fn sign(secret: &str, media_id: &str, exp: i64) -> String {
    let message = ["render", media_id, &exp.to_string()].map(encode_uri_component).join("|");
    let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(message.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// `mintRenderTicket(mediaId, ttl)`: `<mediaId>.<expiry>.<HMAC>` - ids are
/// uuids, so the separator is unambiguous.
pub fn mint_render_ticket(secret: &str, media_id: &str, ttl_seconds: Option<i64>) -> String {
    let exp = now_ms().div_euclid(1000) + ttl_seconds.unwrap_or(RENDER_TICKET_TTL);
    format!("{media_id}.{exp}.{}", sign(secret, media_id, exp))
}

/// `verifyRenderTicket(ticket)`: the upload id this ticket authorizes, or None
/// when it proves nothing.
pub fn verify_render_ticket(secret: &str, ticket: Option<&str>) -> Option<String> {
    let ticket = ticket.filter(|t| !t.is_empty())?;
    let parts: Vec<&str> = ticket.split('.').collect();
    let [media_id, raw_exp, signature] = parts.as_slice() else {
        return None;
    };
    if media_id.is_empty() || signature.is_empty() {
        return None;
    }
    let exp = js_number(raw_exp);
    // Number.isSafeInteger(exp) && exp > 0
    if !(exp.is_finite() && exp.fract() == 0.0 && exp.abs() <= 9_007_199_254_740_991.0) || exp <= 0.0 {
        return None;
    }
    if exp * 1000.0 <= now_ms() as f64 {
        return None;
    }
    constant_time_eq(signature.as_bytes(), sign(secret, media_id, exp as i64).as_bytes()).then(|| media_id.to_string())
}

// ── Request admission ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderAdmission {
    Allow {
        media_id: String,
    },
    /// A genuine top-level visit: hand the visitor the preview page instead.
    NotAFrame,
    /// The client did not say what it was fetching for, so we cannot tell a
    /// top-level visit from a frame. Distinct from `NotAFrame` because the two
    /// deserve opposite answers: redirecting is friendly for a real navigation and
    /// harmful for a frame, where it would load the whole preview page INSIDE the
    /// frame and spend a second view on every click of the toggle.
    UnknownDest,
    BadTicket,
}

/// `renderAdmission({ secFetchDest, ticket })`: is this request allowed to
/// receive executable markup?
///
/// `Sec-Fetch-Dest: iframe` is the load-bearing half. Without it a /render URL
/// could be pasted into a chat and opened as a TOP-LEVEL document on our domain,
/// with our certificate in the address bar - the perfect phishing page. The
/// header cannot be set from JavaScript, and anything that omits it (curl, an
/// old client) is treated as "not a frame", which is the safe default.
pub fn render_admission(secret: &str, sec_fetch_dest: Option<&str>, ticket: Option<&str>) -> RenderAdmission {
    let Some(dest) = sec_fetch_dest.filter(|d| !d.is_empty()) else {
        return RenderAdmission::UnknownDest;
    };
    if dest != "iframe" {
        return RenderAdmission::NotAFrame;
    }
    match verify_render_ticket(secret, ticket) {
        Some(media_id) => RenderAdmission::Allow { media_id },
        None => RenderAdmission::BadTicket,
    }
}

// ── Response headers ────────────────────────────────────────────────────────

/// `renderCsp(shape)`: the policy that makes this safe at all.
///
/// `sandbox` in a RESPONSE header drops the document into an opaque origin
/// whether or not it ended up in a frame. Measured upstream rather than assumed:
/// on the live route this policy is usually NOT the one the browser sees,
/// because the next.config rule for `/:userCode/:mediaCode/render` is applied
/// first and wins (`http::headers`); it is kept as the fallback that engages if
/// that rule is ever narrowed or removed.
pub fn render_csp(shape: SandboxShape) -> String {
    let mut parts = vec![
        if allows_scripts(shape) { "sandbox allow-scripts" } else { "sandbox" },
        "default-src 'none'",
        "img-src data: blob:",
        "media-src data: blob:",
        "font-src data:",
        "style-src 'unsafe-inline'",
    ];
    if allows_scripts(shape) {
        parts.push("script-src 'unsafe-inline'");
    }
    parts.extend(["form-action 'none'", "base-uri 'none'", "object-src 'none'", "frame-ancestors 'self'"]);
    parts.join("; ")
}

pub fn render_content_type(shape: SandboxShape) -> &'static str {
    if shape == SandboxShape::Svg { "image/svg+xml; charset=utf-8" } else { "text/html; charset=utf-8" }
}

/// Is this shape the uploaded bytes, or markup we generate from them?
pub fn is_generated(shape: SandboxShape) -> bool {
    shape == SandboxShape::Docx
}

#[cfg(test)]
mod tests {
    //! tests/unit/render-ticket.test.ts
    use super::*;
    use crate::media::mime::serve_decision;

    const SECRET: &str = "render-ticket-test-secret-0123456789abcdef";
    const MEDIA: &str = "9d2e1a54-0000-4000-8000-000000000001";
    const OTHER: &str = "9d2e1a54-0000-4000-8000-000000000002";

    fn mint(id: &str) -> String {
        mint_render_ticket(SECRET, id, None)
    }

    fn verify(t: Option<&str>) -> Option<String> {
        verify_render_ticket(SECRET, t)
    }

    #[test]
    fn accepts_only_the_markup_types() {
        assert_eq!(sandbox_shape(Some("text/html"), None), Some(SandboxShape::Html));
        assert_eq!(sandbox_shape(Some("Text/HTML; charset=UTF-8"), None), Some(SandboxShape::Html));
        assert_eq!(sandbox_shape(Some("application/xhtml+xml"), None), Some(SandboxShape::Html));
        assert_eq!(sandbox_shape(Some("image/svg+xml"), None), Some(SandboxShape::Svg));
    }

    #[test]
    fn refuses_everything_else() {
        for mime in [
            Some("text/plain"),
            Some("text/xml"),
            Some("application/xml"),
            Some("application/json"),
            Some("application/javascript"),
            Some("multipart/related"),
            Some("message/rfc822"),
            Some("application/pdf"),
            Some("image/png"),
            Some(""),
            None,
            // Plain-object keys upstream would have found on the prototype.
            Some("constructor"),
            Some("__proto__"),
        ] {
            assert_eq!(sandbox_shape(mime, None), None, "{mime:?}");
        }
    }

    #[test]
    fn documents_fall_back_to_the_extension_and_markup_does_not() {
        assert_eq!(sandbox_shape(Some("application/octet-stream"), Some("docx")), Some(SandboxShape::Docx));
        assert_eq!(sandbox_shape(Some("application/octet-stream"), Some("html")), None);
        assert_eq!(sandbox_shape(Some("application/octet-stream"), Some("svg")), None);
        assert_eq!(sandbox_shape(Some("application/msword"), Some("doc")), None);
    }

    #[test]
    fn offers_the_toggle_only_up_to_the_size_ceiling() {
        assert!(can_render_inline("text/html", None, 1024));
        assert!(can_render_inline("text/html", None, MAX_RENDER_BYTES));
        assert!(!can_render_inline("text/html", None, MAX_RENDER_BYTES + 1));
        assert!(!can_render_inline("application/pdf", None, 10));
        assert!(can_render_inline("application/octet-stream", Some("docx"), MAX_DOCX_BYTES));
        assert!(!can_render_inline("application/octet-stream", Some("docx"), MAX_DOCX_BYTES + 1));
    }

    #[test]
    fn accepts_a_freshly_minted_ticket_and_reports_the_file_it_names() {
        assert_eq!(verify(Some(&mint(MEDIA))).as_deref(), Some(MEDIA));
        let other = mint(OTHER);
        assert_eq!(verify(Some(&other)).as_deref(), Some(OTHER));
    }

    #[test]
    fn rejects_a_swapped_id_under_a_valid_signature() {
        let t = mint(MEDIA);
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(verify(Some(&format!("{OTHER}.{}.{}", parts[1], parts[2]))), None);
    }

    #[test]
    fn rejects_a_tampered_signature() {
        let t = mint(MEDIA);
        let p: Vec<&str> = t.split('.').collect();
        let sig = p[2];
        let flipped = match sig.strip_prefix('A') {
            Some(rest) => format!("B{rest}"),
            None => format!("A{}", &sig[1..]),
        };
        assert_eq!(verify(Some(&format!("{}.{}.{flipped}", p[0], p[1]))), None);
        assert_eq!(verify(Some(&format!("{}.{}.{}", p[0], p[1], &sig[..sig.len() - 2]))), None);
        assert_eq!(verify(Some(&format!("{}.{}.{sig}xx", p[0], p[1]))), None);
        // A different secret proves nothing either.
        assert_eq!(verify_render_ticket("another-secret", Some(&t)), None);
    }

    #[test]
    fn rejects_an_extended_expiry_because_the_signature_covers_it() {
        let t = mint(MEDIA);
        let p: Vec<&str> = t.split('.').collect();
        let far = now_ms() / 1000 + 60 * 60 * 24 * 365;
        assert_eq!(verify(Some(&format!("{}.{far}.{}", p[0], p[2]))), None);
    }

    #[test]
    fn rejects_an_expired_ticket() {
        assert_eq!(verify(Some(&mint_render_ticket(SECRET, MEDIA, Some(-1)))), None);
    }

    #[test]
    fn rejects_malformed_input_instead_of_throwing() {
        for junk in ["", "garbage", ".", "..", "a.b", "a.b.c.d", "id.NaN.sig", "id.-1.sig", "id.1e400.sig"] {
            assert_eq!(verify(Some(junk)), None, "{junk}");
        }
        assert_eq!(verify(None), None);
    }

    #[test]
    fn the_message_is_the_upstream_one() {
        // HMAC-SHA256(secret, "render|<id>|<exp>") as base64url without padding,
        // each part percent-encoded (node: createHmac(...).digest("base64url")).
        let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(format!("render|{MEDIA}|2000000000").as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_eq!(sign(SECRET, MEDIA, 2_000_000_000), expected);
        assert_eq!(
            verify(Some(&format!("{MEDIA}.2000000000.{expected}"))).as_deref(),
            Some(MEDIA),
            "a ticket minted by the TypeScript server verifies here"
        );
    }

    #[test]
    fn lets_a_real_frame_load_through() {
        let t = mint(MEDIA);
        assert_eq!(
            render_admission(SECRET, Some("iframe"), Some(&t)),
            RenderAdmission::Allow { media_id: MEDIA.to_string() }
        );
    }

    #[test]
    fn refuses_a_top_level_navigation_even_with_a_perfect_ticket() {
        let t = mint(MEDIA);
        for dest in ["document", "empty", "object", "embed"] {
            assert_eq!(render_admission(SECRET, Some(dest), Some(&t)), RenderAdmission::NotAFrame, "{dest}");
        }
    }

    #[test]
    fn separates_a_missing_destination_from_a_declared_top_level_one() {
        let t = mint(MEDIA);
        assert_eq!(render_admission(SECRET, Some(""), Some(&t)), RenderAdmission::UnknownDest);
        assert_eq!(render_admission(SECRET, None, Some(&t)), RenderAdmission::UnknownDest);
    }

    #[test]
    fn refuses_a_frame_without_a_valid_ticket() {
        assert_eq!(render_admission(SECRET, Some("iframe"), None), RenderAdmission::BadTicket);
        let expired = mint_render_ticket(SECRET, MEDIA, Some(-1));
        assert_eq!(render_admission(SECRET, Some("iframe"), Some(&expired)), RenderAdmission::BadTicket);
    }

    #[test]
    fn puts_every_rendered_document_in_an_opaque_origin() {
        assert!(render_csp(SandboxShape::Html).contains("sandbox allow-scripts"));
        assert!(render_csp(SandboxShape::Svg).contains("sandbox"));
        assert!(!render_csp(SandboxShape::Svg).contains("allow-scripts"));
        assert!(!render_csp(SandboxShape::Svg).contains("script-src"));
        assert!(!render_csp(SandboxShape::Docx).contains("allow-scripts"));
    }

    #[test]
    fn never_grants_same_origin_forms_popups_or_top_navigation() {
        for shape in [SandboxShape::Html, SandboxShape::Svg] {
            let csp = render_csp(shape);
            for banned in [
                "allow-same-origin",
                "allow-forms",
                "allow-popups",
                "allow-top-navigation",
                "allow-downloads",
                "allow-modals",
            ] {
                assert!(!csp.contains(banned), "{banned}");
            }
            assert!(csp.contains("default-src 'none'"));
            assert!(csp.contains("form-action 'none'"));
            assert!(csp.contains("frame-ancestors 'self'"));
        }
        assert_eq!(
            render_csp(SandboxShape::Html),
            "sandbox allow-scripts; default-src 'none'; img-src data: blob:; media-src data: blob:; font-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'self'"
        );
    }

    #[test]
    fn labels_the_document_as_what_it_is() {
        assert_eq!(render_content_type(SandboxShape::Html), "text/html; charset=utf-8");
        assert_eq!(render_content_type(SandboxShape::Svg), "image/svg+xml; charset=utf-8");
        assert_eq!(render_content_type(SandboxShape::Docx), "text/html; charset=utf-8");
        assert!(is_generated(SandboxShape::Docx));
        assert!(!is_generated(SandboxShape::Html));
    }

    #[test]
    fn the_raw_contract_is_untouched() {
        for mime in ["text/html", "image/svg+xml", "application/xhtml+xml"] {
            let d = serve_decision(Some(mime), None);
            assert_eq!(d.disposition, "inline");
            assert_eq!(d.content_type, "text/plain; charset=utf-8");
        }
    }
}
