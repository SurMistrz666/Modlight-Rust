//! Port of `app/s/[code]/route.ts`: a root-level short link, `/s/{code}` ->
//! target. Kept for links created before per-user namespacing and for the
//! (privileged) root namespace; namespaced links live one segment deeper
//! ([`crate::routes::s_code_name`]). On a dedicated shortener host, proxy.ts
//! rewrites the bare path to this route.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;

use crate::error::AppResult;
use crate::http::cookies::parse_cookies_next;
use crate::shortlinks::redirect_short_link;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/s/{code}", get(resolve))
}

async fn resolve(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap) -> AppResult<Response> {
    let jar = parse_cookies_next(&headers);
    Ok(redirect_short_link(&state.db, &state.config, &code, |name| jar.get(name).cloned()).await?)
}
