//! Port of `app/download/app/route.ts`: the stable address of the current
//! Modlight Downloader build.
//!
//! A redirect rather than a proxy, on purpose: the target is an ordinary upload,
//! so following it gets Range requests, resumable downloads, the ETag and the
//! storage driver's own streaming for free. Re-serving the bytes through here
//! would reimplement all four, worse.
//!
//! Its whole value is that it does not change. `/{tag}/{code}` names one build
//! forever, which is right for a file and wrong for "the latest version": a
//! link put in a README or an installer would freeze on the day it was written.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::error::AppResult;
use crate::releases::current_release;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/download/app", get(download_app))
}

async fn download_app(State(state): State<AppState>) -> AppResult<Response> {
    let Some(release) = current_release(&state.db, &state.config).await? else {
        let mut res = (StatusCode::NOT_FOUND, "No release published yet.").into_response();
        let headers = res.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return Ok(res);
    };

    let mut res = StatusCode::FOUND.into_response();
    let headers = res.headers_mut();
    // The stored values reach the header as they are (a URL built from config
    // and the owner's tag; the version as published).
    if let Ok(location) = HeaderValue::from_bytes(release.url.as_bytes()) {
        headers.insert(header::LOCATION, location);
    }
    // Short, and never `immutable`: this URL means "whatever is current", so
    // a cached redirect is precisely how somebody keeps downloading a build
    // that was replaced weeks ago.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=60"));
    // Handy for a script that wants to know what it is about to fetch without
    // reading the manifest first.
    if let Ok(version) = HeaderValue::from_bytes(release.version.as_bytes()) {
        headers.insert("x-release-version", version);
    }
    if let Some(sha256) = release.sha256.as_deref().filter(|s| !s.is_empty())
        && let Ok(value) = HeaderValue::from_bytes(sha256.as_bytes())
    {
        headers.insert("x-release-sha256", value);
    }
    Ok(res)
}
