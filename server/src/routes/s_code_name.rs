//! Port of `app/s/[code]/[name]/route.ts`: a namespaced short link,
//! `/s/{userCode}/{code}` -> target. The first segment is the owner's tag,
//! stored as part of `short_links.code` ("alicja/test"), so two users can
//! both own "test". The folders are `[code]/[name]` because Next refuses two
//! slug names at one position: `code` is the tag and `name` the leaf.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;

use crate::error::AppResult;
use crate::http::cookies::parse_cookies_next;
use crate::naming::scoped_short_code;
use crate::shortlinks::{redirect_short_link, text_response};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/s/{code}/{name}", get(resolve))
}

async fn resolve(
    State(state): State<AppState>,
    Path((tag, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    // The static unlock page wins over this route; belt and braces in case
    // that ordering ever changes.
    if name == "unlock" {
        return Ok(text_response(StatusCode::NOT_FOUND, "Not found"));
    }
    let jar = parse_cookies_next(&headers);
    let stored = scoped_short_code(Some(&tag), &name);
    Ok(redirect_short_link(&state.db, &state.config, &stored, |cookie| jar.get(cookie).cloned()).await?)
}
