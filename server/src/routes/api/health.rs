//! Port of `app/api/health/route.ts`: liveness/readiness probe for the
//! container healthcheck and for host-side alerting.
//!
//! Checks the two dependencies that can fail independently of the process
//! still being alive: a server that answers HTTP while its database is
//! unreachable is exactly the "silently broken" state a healthcheck exists to
//! catch. Answers 503 so Docker and any uptime monitor treat it as down.
//!
//! Reveals nothing useful to an anonymous caller: no versions, no paths, no
//! connection strings, just which subsystem is unhappy.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use serde_json::json;

use crate::api_json::json_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/health", get(health))
}

async fn health(State(state): State<AppState>) -> Response {
    let database = sqlx::query("select 1").execute(&state.db).await.is_ok();

    // Cheap, read-only: proves the storage driver answers and its root is
    // reachable, without touching user data. A missing key is a healthy
    // answer; only an error (unmounted volume, dead S3 endpoint, a driver that
    // cannot be built) means trouble.
    let storage = match state.storage.get() {
        Ok(driver) => driver.stat("__healthcheck__").await.is_ok(),
        Err(_) => false,
    };

    let ok = |b: bool| if b { "ok" } else { "fail" };
    let healthy = database && storage;
    json_response(
        if healthy { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        &json!({
            "status": if healthy { "ok" } else { "degraded" },
            "checks": { "database": ok(database), "storage": ok(storage) },
        }),
        Some("no-store"),
    )
}
