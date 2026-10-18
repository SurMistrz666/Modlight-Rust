//! The twoFactor plugin (`twoFactor({ issuer: APP_NAME })`). NOT IMPLEMENTED
//! YET: the auth agent owns this.
//!
//! Upstream: better-auth 1.6.13 `plugins/two-factor/` (`index.mjs`,
//! `totp/`, `backup-codes/`, `verify-two-factor.mjs`).
//!
//! To reproduce, byte for byte with the rows the TypeScript server wrote:
//! - `two_factor.secret` and `two_factor.backup_codes` are encrypted with the
//!   Better Auth symmetric format (`symmetricEncrypt` over AUTH_SECRET);
//! - TOTP parameters (digits, period, algorithm) and the `otpauth://` URI with
//!   the issuer;
//! - the pending-2FA cookie set by `/sign-in/email` when the account has 2FA,
//!   and the trust-device cookie;
//! - `user.two_factor_enabled` flips on `enable` + first successful verify.
