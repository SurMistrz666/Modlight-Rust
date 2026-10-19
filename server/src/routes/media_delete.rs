//! Port of `app/[userCode]/[mediaCode]/delete/[token]/route.ts`: the
//! delete-by-link page (`GET` asks, `POST` deletes for good).
//!
//! Two different credentials open this page, and only one of them is a link.
//! The per-media `deleteToken` is the one baked into `.sxcu`'s DeletionURL and
//! handed to other people; it deletes exactly the file it names. The second
//! branch accepts an ACCOUNT API token in the same slot, which deletes any file
//! the account owns, so it needs the `write` capability. 404 rather than 403
//! on a refusal: answering "right file, wrong key" would turn the URL into an
//! oracle for which codes exist.
//!
//! Security fix 5 (PORT.md 12.5): the delete token is compared in constant
//! time (upstream: `===`).

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;

use crate::api_token::{TokenAuth, TokenCapability, authenticate_api_token};
use crate::auth::crypto::constant_time_eq;
use crate::db::models::Upload;
use crate::error::AppResult;
use crate::media::actions::{DeletableMedia, delete_media_hard};
use crate::media::serve::{get_media, plain_text, put_headers};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/{userCode}/{mediaCode}/delete/{token}", get(ask).post(delete))
}

/// `authorized(media, token)`
async fn authorized(state: &AppState, media: &Upload, token: &str) -> anyhow::Result<bool> {
    // Security fix 5: no early exit on the first differing byte.
    if !token.is_empty()
        && !media.delete_token.is_empty()
        && constant_time_eq(token.as_bytes(), media.delete_token.as_bytes())
    {
        return Ok(true);
    }
    Ok(match authenticate_api_token(&state.db, Some(token), TokenCapability::Write).await? {
        TokenAuth::Ok { user, .. } => media.owner_id.as_deref() == Some(user.id.as_str()),
        _ => false,
    })
}

/// `esc(value)`: this page is hand-built HTML rather than React, so nothing
/// escapes for us. `filename` comes straight off the multipart upload; a name
/// like `<img src=x onerror=...>` would otherwise execute on this origin, in the
/// session of whoever opened the deletion link.
pub fn esc(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// `page(title, body)`: `body` is trusted markup assembled below; every value
/// coming from the database or the request passes through [`esc`] first.
fn page(app_name: &str, title: &str, body: &str) -> Response {
    let app = esc(app_name);
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} - {app}</title></head>\n\
<body style=\"margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0b0c;color:#fafafa;font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif\">\n\
<div style=\"max-width:420px;width:90%;background:#151517;border:1px solid #26262a;border-radius:16px;padding:28px;text-align:center\">\n\
<div style=\"font-weight:700;margin-bottom:12px\">{app}</div>{body}</div></body></html>",
        esc(title)
    );
    let mut res = Response::new(Body::from(html));
    put_headers(
        &mut res,
        [
            ("Content-Type", "text/html; charset=utf-8".to_string()),
            // Belt and braces: even if something unescaped slips in later, this
            // page has no legitimate need to run script or be framed. (The
            // next.config rule that covers every path replaces this policy and
            // the Referrer-Policy on the wire, exactly as upstream.)
            (
                "Content-Security-Policy",
                "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'".to_string(),
            ),
            ("X-Content-Type-Options", "nosniff".to_string()),
            ("Referrer-Policy", "no-referrer".to_string()),
            ("Cache-Control", "no-store".to_string()),
        ],
    );
    res
}

async fn ask(
    State(state): State<AppState>,
    Path((user_code, media_code, token)): Path<(String, String, String)>,
) -> AppResult<Response> {
    let Some(found) = get_media(&state.db, &user_code, &media_code).await? else {
        return Ok(plain_text(StatusCode::NOT_FOUND, "Not found"));
    };
    if !authorized(&state, &found.media, &token).await? {
        return Ok(plain_text(StatusCode::NOT_FOUND, "Not found"));
    }
    let body = format!(
        "\n    <p style=\"color:#a1a1aa;font-size:14px;line-height:1.6\">Delete <b style=\"color:#fafafa\">{}</b> permanently? This cannot be undone.</p>\n    <form method=\"post\" style=\"margin-top:18px\">\n      <button type=\"submit\" style=\"background:#ef4444;color:#fff;border:0;border-radius:10px;padding:11px 22px;font-weight:600;cursor:pointer\">Delete permanently</button>\n    </form>",
        esc(&found.media.filename)
    );
    Ok(page(&state.config.app_name, "Delete file", &body))
}

async fn delete(
    State(state): State<AppState>,
    Path((user_code, media_code, token)): Path<(String, String, String)>,
) -> AppResult<Response> {
    let Some(found) = get_media(&state.db, &user_code, &media_code).await? else {
        return Ok(plain_text(StatusCode::NOT_FOUND, "Not found"));
    };
    if !authorized(&state, &found.media, &token).await? {
        return Ok(plain_text(StatusCode::NOT_FOUND, "Not found"));
    }
    let storage = state.storage.get()?;
    delete_media_hard(&state.db, storage.as_ref(), &DeletableMedia::from(&found.media)).await?;
    Ok(page(
        &state.config.app_name,
        "Deleted",
        "<p style=\"color:#a1a1aa;font-size:14px\">The file has been deleted.</p>",
    ))
}

#[cfg(test)]
mod tests {
    //! tests/unit/delete-page-escaping.test.ts
    use super::*;

    #[test]
    fn neutralises_an_image_onerror_payload() {
        let out = esc("<img src=x onerror=\"fetch('/api/auth/update-user')\">");
        assert!(!out.contains("<img") && !out.contains('>') && out.contains("&lt;img"));
    }

    #[test]
    fn neutralises_a_script_tag() {
        assert_eq!(esc("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;/script&gt;");
    }

    #[test]
    fn escapes_quotes() {
        assert_eq!(esc("\" onmouseover=\"alert(1)"), "&quot; onmouseover=&quot;alert(1)");
        assert_eq!(esc("' onfocus='alert(1)"), "&#39; onfocus=&#39;alert(1)");
    }

    #[test]
    fn escapes_ampersands_first() {
        assert_eq!(esc("&lt;script&gt;"), "&amp;lt;script&amp;gt;");
    }

    #[test]
    fn leaves_ordinary_filenames_readable() {
        assert_eq!(esc("Źródła wiedzy.docx"), "Źródła wiedzy.docx");
        assert_eq!(esc("raport-2026.pdf"), "raport-2026.pdf");
    }

    #[test]
    fn the_page_carries_only_escaped_values() {
        let res = page("App <b>", "Delete file", "");
        let ct = res.headers()["content-type"].to_str().unwrap().to_string();
        assert_eq!(ct, "text/html; charset=utf-8");
    }
}
