//! `/api/auth/*`: port of `app/api/auth/[...all]/route.ts`
//! (`toNextJsHandler(auth)`). NOT IMPLEMENTED YET: the auth agent owns this.
//!
//! Until [`routes`] registers handlers, every `/api/auth/*` request is
//! answered by the route-table placeholder (501 naming this upstream file).
//!
//! What the browser and the server call (better-auth 1.6.13 paths, all under
//! `basePath = /api/auth`):
//!
//! | Path | Method | Upstream source |
//! |---|---|---|
//! | `/get-session` | GET | `api/routes/session.mjs` (reuse [`super::session::get_session`] with `RefreshPolicy::DbAndCookie`; responses use the better-call cookie format) |
//! | `/sign-in/email` | POST | `api/routes/sign-in.mjs` (username login is resolved to an e-mail by `app/actions/login-identifier.ts`) |
//! | `/sign-up/email` | POST | `api/routes/sign-up.mjs` + the `hooks.before` registration gate in lib/auth.ts |
//! | `/sign-out` | POST | `api/routes/sign-out.mjs` |
//! | `/request-password-reset`, `/reset-password`, `/reset-password/:token` | POST/GET | `api/routes/password.mjs` |
//! | `/change-password` | POST | `api/routes/update-user.mjs` |
//! | `/send-verification-email`, `/verify-email` | POST/GET | `api/routes/email-verification.mjs` |
//! | `/two-factor/enable`, `/disable`, `/verify-totp`, `/verify-backup-code`, `/get-totp-uri`, `/generate-backup-codes` | POST | `plugins/two-factor/` |
//! | `/admin/*` | POST/GET | `plugins/admin/routes.mjs` |
//! | `/list-sessions`, `/revoke-session`, `/revoke-sessions`, `/revoke-other-sessions` | GET/POST | `api/routes/session.mjs` |
//!
//! Cross-cutting rules every endpoint goes through (`api/index.mjs`,
//! `api/middlewares/origin-check.mjs`, `api/rate-limiter/index.mjs`):
//! - the origin check on state-changing requests with cookies (trusted origin
//!   = APP_URL's origin);
//! - the in-memory rate limiter (window/max per path, see `super` docs);
//! - errors as `{ "code": ..., "message": ... }` with the APIError status.

use axum::Router;

use crate::state::AppState;

/// Handlers for `/api/auth/{*all}`. Empty until the auth port lands.
pub fn routes() -> Router<AppState> {
    Router::new()
}
