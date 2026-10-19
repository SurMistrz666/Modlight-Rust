//! Route handlers: one module per upstream `app/**/route.ts`, named after its
//! path (PORT.md section 5): `app/api/uploads/route.ts` ->
//! `src/routes/api/uploads.rs`, `app/[userCode]/[mediaCode]/[action]/route.ts`
//! -> `src/routes/media_action.rs`.
//!
//! Each module exposes `pub fn routes() -> Router<AppState>` registering its
//! upstream path with axum syntax and the upstream folder names as parameter
//! names (`/{userCode}/{mediaCode}/{action}`, `/api/auth/{*all}`); the names
//! must match [`crate::pages::table`] because axum keeps one parameter name
//! per path position. Each module is listed with one line in
//! [`crate::http::router`]; until then its URLs answer the 501 placeholder
//! naming the upstream file.

pub mod api;
