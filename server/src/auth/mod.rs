//! Better Auth compatibility (PORT.md section 9). Port of `lib/auth.ts` and of
//! the parts of better-auth 1.6.13 (+ better-call 1.3.5, @better-auth/utils
//! 0.4.1) that Modlight's configuration exercises.
//!
//! Reference sources (read-only, never executed) are unpacked under
//! `_scratch/srv-skeleton/src/` next to this repository: `better-auth-1.6.13`,
//! `better-auth-core-1.6.13`, `better-call-1.3.5`, `better-auth-utils-0.4.1`.
//!
//! Module map:
//!
//! | Module | Upstream |
//! |---|---|
//! | [`api`] | `app/api/auth/[...all]/route.ts`, better-call `router.mjs`, better-auth `api/index.mjs` |
//! | [`endpoint`] | `api/to-auth-endpoints.mjs`, better-call `endpoint.mjs` / `context.mjs` (the call pipeline, hooks) |
//! | [`routes`] | `api/routes/*.mjs` (core endpoints) |
//! | [`two_factor`] | `plugins/two-factor/` |
//! | [`admin`] | `plugins/admin/` |
//! | [`server`] | the `auth.api.*` calls server code makes |
//! | [`adapter`] | `db/internal-adapter.mjs` + the drizzle adapter + database hooks |
//! | [`validate`] | better-call `runValidation` over zod 4 schemas |
//! | [`origin`] | `api/middlewares/origin-check.mjs` |
//! | [`rate_limit`] | `api/rate-limiter/index.mjs` |
//! | [`ip`] | `utils/get-request-ip.mjs`, core `utils/ip.mjs` |
//! | [`cookies`] | cookie names, attributes, signed values |
//! | [`session`] | `getSession` incl. refresh and expiry |
//! | [`email`] | verification tokens and lib/auth.ts's mail callbacks |
//! | [`password`] | scrypt hashes + legacy bcrypt (`lib/auth.ts` `verifyUserPassword`) |
//! | [`crypto`] | ids, symmetric encryption, JWT, TOTP |
//! | [`error`] | `APIError` and the error-code tables |
//!
//! `lib/auth-helpers.ts` (`getSession`, `requireUser`, `requireAdmin`) lives in
//! [`crate::auth_helpers`]; API tokens are not Better Auth and live in
//! [`crate::api_token`].
//!
//! The options lib/auth.ts passes, which every module here must honour:
//! - `baseURL = APP_URL`, `secret = AUTH_SECRET` (cookies `__Secure-` on https);
//! - `emailAndPassword.requireEmailVerification` is a live getter over the mail
//!   settings snapshot (`lib/mail-settings.ts` `verificationRequiredSync`);
//! - `emailVerification`: `sendOnSignUp`, `sendOnSignIn`,
//!   `autoSignInAfterVerification`;
//! - rate limit: enabled, window 60 s / max 120, `/sign-up/email` and
//!   `/sign-in/email` 60 s / 30, keyed on `x-real-ip` (PORT.md 12.7: honour it
//!   only when TRUST_PROXY is on, see [`crate::password::client_ip`]);
//! - `user.additionalFields`: userCode, usedQuota, maxQuota, hideUploads,
//!   copyRaw, theme, locale (see [`session::UserOut`] for the output order);
//! - `hooks.before` on `/sign-up/email`: the registration gate
//!   (`lib/registration.ts` `evaluateSignUp`), with its four APIError codes;
//! - `databaseHooks.user.create.before`: first account becomes admin (decide it
//!   in a transaction under an advisory lock, PORT.md 12.3), unique userCode,
//!   `maxQuota = DEFAULT_USER_QUOTA`;
//! - plugins: `admin()`, `twoFactor({ issuer: APP_NAME })`, `nextCookies()`.

// Endpoints answer with `APIError`s that carry a status, a JSON body and
// headers; returning them by value keeps the ports readable, and none of this
// is on a hot path.
#![allow(clippy::result_large_err)]

pub mod adapter;
pub mod admin;
pub mod api;
pub mod cookies;
pub mod crypto;
pub mod email;
pub mod endpoint;
pub mod error;
pub mod ip;
pub mod origin;
pub mod password;
pub mod rate_limit;
pub mod routes;
pub mod server;
pub mod session;
pub mod two_factor;
pub mod validate;

pub use session::{AuthSession, RefreshPolicy, SessionLookup, SessionOutput};
