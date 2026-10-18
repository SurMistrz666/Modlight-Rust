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
//! | Module | Status | Upstream |
//! |---|---|---|
//! | [`cookies`] | done | cookie names, attributes, signed values (`cookies/index.mjs`, better-call `crypto.mjs`) |
//! | [`session`] | done | `getSession` incl. refresh and expiry (`api/routes/session.mjs`, `db/internal-adapter.mjs`) |
//! | [`api`] | layout only | the `/api/auth/*` endpoints (`app/api/auth/[...all]/route.ts`) |
//! | [`password`] | layout only | scrypt hashes + legacy bcrypt (`lib/auth.ts` `verifyUserPassword`) |
//! | [`two_factor`] | layout only | TOTP, backup codes, trusted devices (`plugins/two-factor/`) |
//! | [`admin`] | layout only | admin plugin endpoints and server calls (`plugins/admin/`) |
//! | [`email`] | layout only | e-mail verification JWT, reset-password tokens |
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

pub mod admin;
pub mod api;
pub mod cookies;
pub mod crypto;
pub mod email;
pub mod error;
pub mod password;
pub mod session;
pub mod two_factor;

pub use session::{AuthSession, RefreshPolicy, SessionLookup, SessionOutput};
