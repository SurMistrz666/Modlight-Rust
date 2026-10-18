//! Account passwords (Better Auth `emailAndPassword.password`). NOT
//! IMPLEMENTED YET: the auth agent owns this.
//!
//! Format to reproduce (`@better-auth/utils` 0.4.1 `password.node.mjs`):
//! - hash: `${salt}:${key}` where `salt` = 16 random bytes as hex (the HEX
//!   STRING is the scrypt salt, not its bytes) and `key` = scrypt(password
//!   NFKC-normalised, salt, N=16384, r=16, p=1, dkLen=64) as hex;
//! - verify: split on `:`, recompute, compare the hex strings; a hash without
//!   both parts is an error ("Invalid password hash").
//!
//! Legacy path (`lib/auth.ts` `verifyUserPassword`): hashes matching
//! `^\$2[aby]\$` are bcrypt (PHP `password_hash`), verified with bcrypt and
//! only the first 72 bytes of the password count.
//!
//! The hashes live in `account.password` (provider_id `credential`).
//! Per-file and default upload passwords are a different scheme
//! (`lib/password.ts`, see [`crate::password`]).
