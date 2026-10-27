//! The locked answers of a file vault (docs/ENCRYPTION.md sections 4 and 6).
//!
//! | Caller | Answer |
//! |---|---|
//! | an API client (bearer token) | `423 Locked`, JSON `{"error": "files_locked", ...}` (401 would read as "token invalid, sign in again" to the desktop app) |
//! | a browser opening the URL as a page | `401` with a small HTML unlock form in the Graphite look |
//! | anything else (an `<img>`, a player, `fetch`) | `401 text/plain` |
//!
//! Every one carries `X-Modlight-Locked: 1` and `Cache-Control: private,
//! no-store`, and none names the file.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;

use super::LOCKED_HEADER;
use crate::api_token::bearer_from_request;
use crate::i18n::{Arg, Locale};
use crate::state::AppState;

/// The policy of the unlock form when it answers on a raw URL, put on the
/// response by `http.rs` after the path's own rule (the raw policy sandboxes
/// the document, and a sandboxed document cannot submit a form). No script at
/// all; only a form posting to this origin and inline styles.
pub const GATE_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; font-src 'self'; \
form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// Response extension: replace the path's Content-Security-Policy with this.
#[derive(Debug, Clone)]
pub struct GateDocumentCsp(pub HeaderValue);

/// What the locked answer should look like for this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Api,
    Document,
    Subresource,
}

pub fn caller_of(headers: &HeaderMap) -> Caller {
    if bearer_from_request(headers).is_some() || headers.get(super::UNLOCK_HEADER).is_some() {
        return Caller::Api;
    }
    let dest = headers.get("sec-fetch-dest").and_then(|v| v.to_str().ok()).unwrap_or("");
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
    if dest == "document" || (dest.is_empty() && accept.contains("text/html")) {
        return Caller::Document;
    }
    if dest.is_empty() && accept.contains("application/json") {
        return Caller::Api;
    }
    Caller::Subresource
}

fn base(status: StatusCode, content_type: &'static str, body: Body) -> Response {
    let mut res = Response::new(body);
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    h.insert(header::VARY, HeaderValue::from_static("Cookie, Authorization"));
    h.insert(LOCKED_HEADER, HeaderValue::from_static("1"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    res
}

/// The JSON answer of a locked file (API clients).
pub fn locked_json(owner_tag: &str) -> Response {
    let body = serde_json::json!({
        "error": "files_locked",
        "message": "This account's files are encrypted. Unlock them with the file password (POST /api/encryption/unlock) and send the grant as X-Modlight-Unlock.",
        "owner": owner_tag,
    });
    base(StatusCode::from_u16(423).expect("423"), "application/json; charset=utf-8", Body::from(body.to_string()))
}

/// The locked answer for this request (see the module docs). `next` is the
/// path the unlock form returns to.
pub fn locked_response(state: &AppState, headers: &HeaderMap, owner_tag: &str, next: &str) -> Response {
    match caller_of(headers) {
        Caller::Api => locked_json(owner_tag),
        Caller::Subresource => base(
            StatusCode::UNAUTHORIZED,
            "text/plain; charset=utf-8",
            Body::from("Files are encrypted: unlock required"),
        ),
        Caller::Document => {
            let locale = crate::i18n::request::request_locale(headers);
            gate_document(state, locale, next, None)
        }
    }
}

/// What went wrong on the previous attempt, for the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    Wrong,
    RateLimited { minutes: i64 },
    Failed,
}

/// The unlock form as a whole document (401). Used for raw links opened in a
/// browser and as the answer to a failed form post.
pub fn gate_document(state: &AppState, locale: Locale, next: &str, error: Option<GateError>) -> Response {
    let t = |key: &str| state.messages.t(locale, "encryption", key, &[]);
    let error_text = error.map(|e| match e {
        GateError::Wrong => t("gate.wrong"),
        GateError::RateLimited { minutes } => {
            state.messages.t(locale, "encryption", "gate.rateLimited", &[("minutes", Arg::Num(minutes as f64))])
        }
        GateError::Failed => t("gate.failed"),
    });
    let html = render_gate(&GateText {
        lang: locale.as_str(),
        title: &t("gate.fileTitle"),
        prompt: &t("gate.filePrompt"),
        password_label: &t("gate.passwordLabel"),
        remember: &t("gate.remember"),
        remember_help: &t("gate.rememberHelp"),
        unlock: &t("gate.unlock"),
        protected_by: &t("gate.protectedBy"),
        app_name: &state.config.app_name,
        error: error_text.as_deref(),
        next,
    });
    let status = match error {
        Some(GateError::RateLimited { .. }) => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::UNAUTHORIZED,
    };
    let mut res = base(status, "text/html; charset=utf-8", Body::from(html));
    if let Ok(v) = HeaderValue::from_str(GATE_CSP) {
        res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v.clone());
        res.extensions_mut().insert(GateDocumentCsp(v));
    }
    res.headers_mut().insert("x-robots-tag", HeaderValue::from_static("noindex"));
    res
}

/// The words of the form.
pub struct GateText<'a> {
    pub lang: &'a str,
    pub title: &'a str,
    pub prompt: &'a str,
    pub password_label: &'a str,
    pub remember: &'a str,
    pub remember_help: &'a str,
    pub unlock: &'a str,
    pub protected_by: &'a str,
    pub app_name: &'a str,
    pub error: Option<&'a str>,
    pub next: &'a str,
}

/// HTML text and attribute escaping.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const LOCK_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"#;

/// The Graphite tokens (src/styles/graphite.css of the SPA), light and dark by
/// the system's preference: no script decides anything on this page.
const GATE_CSS: &str = r#"
:root{color-scheme:light dark;--bg:#fafafa;--fg:#171717;--card:#ffffff;--muted:#55555c;--line:#e4e4e7;--field:#ffffff;--btn:#171717;--btn-fg:#ffffff;--btn-hover:#333333;--ring:#2563eb;--danger:#dc2626;--soft:#f4f4f5;--bevel:0 0 0 1px #e4e4e7,0 1px 2px rgb(16 16 20/4%),0 4px 16px -8px rgb(16 16 20/8%)}
@media (prefers-color-scheme:dark){:root{--bg:#0a0a0a;--fg:#ededed;--card:#121212;--muted:#a1a1a1;--line:#262626;--field:#0a0a0a;--btn:#ffffff;--btn-fg:#000000;--btn-hover:#e5e5e5;--ring:#3b82f6;--danger:#f87171;--soft:#171717;--bevel:0 0 0 1px #262626,inset 0 1px 0 0 rgb(255 255 255/3%),0 8px 24px -12px rgb(0 0 0/60%)}}
@font-face{font-family:"Geist Bold";src:url("/fonts/Geist-Bold.ttf") format("truetype")}
*{box-sizing:border-box}
html,body{margin:0;min-height:100%;background:var(--bg);color:var(--fg)}
body{font-family:ui-sans-serif,system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;font-size:14px;letter-spacing:-.006em;display:flex;min-height:100vh;align-items:center;justify-content:center;padding:40px 16px}
main{width:100%;max-width:400px}
.card{background:var(--card);border-radius:14px;box-shadow:var(--bevel);overflow:hidden}
.head{display:flex;flex-direction:column;align-items:center;gap:14px;padding:36px 24px 26px;text-align:center;border-bottom:1px solid var(--line)}
.icon{display:flex;align-items:center;justify-content:center;width:56px;height:56px;border-radius:14px;background:var(--soft);box-shadow:0 0 0 1px var(--line)}
h1{margin:0;font-family:"Geist Bold",ui-sans-serif,system-ui,sans-serif;font-size:18px;font-weight:600;letter-spacing:-.02em}
.head p{margin:6px 0 0;color:var(--muted);line-height:1.5}
form{display:flex;flex-direction:column;gap:14px;padding:22px 24px 24px}
label.field{font-weight:500}
input[type=password]{width:100%;height:38px;border-radius:9px;border:1px solid var(--line);background:var(--field);color:var(--fg);padding:0 12px;font:inherit}
input[type=password]:focus{outline:2px solid var(--ring);outline-offset:1px;border-color:transparent}
.error{margin:-6px 0 0;color:var(--danger)}
.remember{display:flex;gap:10px;align-items:flex-start;padding:12px;border-radius:10px;background:var(--soft);box-shadow:0 0 0 1px var(--line)}
.remember input{margin:2px 0 0;accent-color:var(--ring)}
.remember small{display:block;color:var(--muted);margin-top:3px;line-height:1.45}
button{height:38px;border:0;border-radius:9px;background:var(--btn);color:var(--btn-fg);font:inherit;font-weight:600;cursor:pointer}
button:hover{background:var(--btn-hover)}
.foot{margin:16px 0 0;text-align:center;color:var(--muted);font-size:12px}
.foot b{font-weight:600;color:var(--fg)}
"#;

pub fn render_gate(t: &GateText<'_>) -> String {
    let error = match t.error {
        Some(e) => format!(r#"<p class="error" role="alert">{}</p>"#, esc(e)),
        None => String::new(),
    };
    format!(
        r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title><style>{css}</style></head><body><main><div class="card"><div class="head"><div class="icon">{svg}</div><div><h1>{title}</h1><p>{prompt}</p></div></div><form method="post" action="/api/encryption/unlock"><input type="hidden" name="next" value="{next}"><label class="field" for="ml-file-password">{label}</label><input id="ml-file-password" name="password" type="password" autocomplete="off" autofocus required>{error}<label class="remember"><input type="checkbox" name="remember" value="1" checked><span>{remember}<small>{remember_help}</small></span></label><button type="submit">{unlock}</button></form></div><p class="foot">{protected_by} <b>{app}</b></p></main></body></html>"#,
        lang = esc(t.lang),
        title = esc(t.title),
        css = GATE_CSS,
        svg = LOCK_SVG,
        prompt = esc(t.prompt),
        next = esc(t.next),
        label = esc(t.password_label),
        error = error,
        remember = esc(t.remember),
        remember_help = esc(t.remember_help),
        unlock = esc(t.unlock),
        protected_by = esc(t.protected_by),
        app = esc(t.app_name),
    )
}

/// A path the form may return to: same origin, absolute path, no scheme-relative
/// `//host`, no backslash tricks, no control characters.
pub fn safe_next(next: &str) -> Option<&str> {
    let ok = next.starts_with('/')
        && !next.starts_with("//")
        && !next.starts_with("/\\")
        && !next.chars().any(|c| c.is_control() || c == '\\')
        && next.len() <= 2048;
    ok.then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(*k, HeaderValue::from_static(v));
        }
        m
    }

    #[test]
    fn the_answer_follows_the_caller() {
        assert_eq!(caller_of(&h(&[("authorization", "Bearer abc")])), Caller::Api);
        assert_eq!(caller_of(&h(&[("x-api-key", "abc")])), Caller::Api);
        assert_eq!(caller_of(&h(&[("sec-fetch-dest", "document")])), Caller::Document);
        assert_eq!(caller_of(&h(&[("accept", "text/html,application/xhtml+xml")])), Caller::Document);
        assert_eq!(caller_of(&h(&[("sec-fetch-dest", "image"), ("accept", "text/html")])), Caller::Subresource);
        assert_eq!(caller_of(&h(&[("sec-fetch-dest", "video")])), Caller::Subresource);
        assert_eq!(caller_of(&h(&[("accept", "application/json")])), Caller::Api);
        assert_eq!(caller_of(&HeaderMap::new()), Caller::Subresource);
    }

    #[test]
    fn locked_json_is_423_and_names_no_file() {
        let res = locked_json("tag");
        assert_eq!(res.status().as_u16(), 423);
        assert_eq!(res.headers()[LOCKED_HEADER], "1");
        assert_eq!(res.headers()[header::CACHE_CONTROL], "private, no-store");
    }

    #[test]
    fn the_form_escapes_everything_it_prints() {
        let html = render_gate(&GateText {
            lang: "en",
            title: "T<script>",
            prompt: "p",
            password_label: "l",
            remember: "r",
            remember_help: "h",
            unlock: "u",
            protected_by: "by",
            app_name: "A&B",
            error: Some("\"bad\""),
            next: "/t/c/raw?x=\"><script>alert(1)</script>",
        });
        assert!(!html.contains("<script>"));
        assert!(html.contains("T&lt;script&gt;"));
        assert!(html.contains("A&amp;B"));
        assert!(html.contains("&quot;bad&quot;"));
        assert!(html.contains(r#"action="/api/encryption/unlock""#));
        assert!(html.contains("checked"));
    }

    #[test]
    fn only_same_origin_paths_are_returned_to() {
        assert_eq!(safe_next("/t/c/raw"), Some("/t/c/raw"));
        assert_eq!(safe_next("/t/c/raw?w=1"), Some("/t/c/raw?w=1"));
        for bad in
            ["", "t/c", "//evil.example.com/x", "/\\evil.example.com", "https://evil.example.com", "/a\nb", "/a\\b"]
        {
            assert_eq!(safe_next(bad), None, "{bad:?}");
        }
    }
}
