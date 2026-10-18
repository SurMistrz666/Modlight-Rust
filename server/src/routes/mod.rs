//! Route handlers: one module per upstream `app/**/route.ts`, named after its
//! path (PORT.md section 5): `app/api/uploads/route.ts` ->
//! `src/routes/api/uploads.rs`, `app/[userCode]/[mediaCode]/[action]/route.ts`
//! -> `src/routes/media_action.rs`.
//!
//! Each module exposes `pub fn routes() -> Router<AppState>` registering its
//! upstream path with axum syntax and the upstream folder names as parameter
//! names (`/{userCode}/{mediaCode}/{action}`, `/api/auth/{*all}`); the names
//! must match [`crate::pages::table`] because axum keeps one parameter name
//! per path position. Until a module is listed here, its URLs answer the 501
//! placeholder naming the upstream file.

pub mod api;

use axum::Router;

use crate::state::AppState;

/// Every route handler module. ONE LINE PER MODULE.
pub fn all() -> Router<AppState> {
    Router::new()
        .merge(api::health::routes()) // app/api/health/route.ts
        .merge(crate::auth::api::routes()) // app/api/auth/[...all]/route.ts
}
