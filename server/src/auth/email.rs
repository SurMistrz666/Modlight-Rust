//! E-mail verification and password reset tokens. NOT IMPLEMENTED YET: the
//! auth agent owns this.
//!
//! Upstream: better-auth 1.6.13 `api/routes/email-verification.mjs` (a JWT
//! signed with AUTH_SECRET, `crypto/jwt.mjs`) and `api/routes/password.mjs`
//! (a `verification` row `reset-password:<token>`). Mail bodies come from
//! `lib/mail-templates.ts` and go out through `lib/mail.ts`, gated by
//! `lib/mail-settings.ts` (`mailReady`, `verificationRequiredSync`).
//! `autoSignInAfterVerification` creates a session when a link is followed.
