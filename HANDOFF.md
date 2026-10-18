# srv-auth handoff (2026-10-18)

Branch `srv-auth`, worktree `_wt/srv-auth`. Task: port accounts, authentication
and administration (Better Auth compatibility layer, lib modules, `/api/auth/*`,
routes, actions, page loaders, mail warm-up job, tests, cross-compatibility proof).
Work was stopped early on the owner's request; this file is the exact state.

## Done (committed, builds, `cargo fmt` / `clippy -D warnings` / 227 lib tests green)

| Rust module | Upstream | Notes |
|---|---|---|
| `src/auth/crypto.rs` | better-auth `crypto/*`, `@better-auth/utils` random/otp/hmac, jose `jwtVerify` | `generate_id` (a-zA-Z0-9), `generate_random_string` (a-z0-9A-Z-_), `symmetric_encrypt/decrypt` (key = SHA-256(secret), XChaCha20-Poly1305, 24-byte nonce prepended, lower-case hex: the format of `two_factor.secret` and `backup_codes`), `sign_jwt`/`verify_jwt` (header exactly `{"alg":"HS256"}`, payload keys then `iat`, `exp`; Expired vs Invalid), TOTP (HMAC-SHA1 keyed by the secret STRING's UTF-8 bytes, 6 digits, 30 s, window ±1), `totp_uri` (`otpauth://totp/<enc issuer>:<enc email>?secret=<base32>&issuer=..&digits=6&period=30`). RFC 4226 vectors tested. `BETTER_AUTH_SECRETS` envelope not supported (Modlight never sets it). |
| `src/auth/password.rs` | `@better-auth/utils` password.node + lib/auth.ts `verifyUserPassword` | `salt(hex 16B):key(hex 64B)`, scrypt N=16384 r=16 p=1, salt = the hex STRING, NFKC; legacy `$2a/$2b/$2y` bcrypt; blocking pool. |
| `src/auth/error.rs` | better-call `APIError`, `BASE_ERROR_CODES`, two-factor and admin codes | Status + reason phrase = status NAME (`400 BAD_REQUEST`) or the number for validation errors (`400 400`); body `{"message","code"}`; redirect = 302 `FOUND` no body. Not used by an endpoint yet. |
| `src/secret_box.rs` | lib/secret-box.ts | AES-256-GCM, HKDF-SHA256 (salt `modlight/secret-box/2026`, info `modlight:<purpose>:v1`), `v1.<iv>.<tag>.<ct>` base64url; `SETTINGS_ENCRYPTION_KEY` override; digest, redaction. Upstream tests ported. |
| `src/settings.rs` | lib/settings.ts | get/set/get_settings/seed, `is_on`, `is_registration_enabled`. |
| `src/registration.rs` | lib/registration.ts | `decide_sign_up` (+ all upstream gate tests), key set/read/state, `registration_page_state`, `evaluate_sign_up`. |
| `src/mail_settings.rs` | lib/mail-settings.ts | block precedence, snapshot (30 s), `verification_required_sync`, save/clear, `warm_mail_settings` (the jobs.rs boot hook now really warms). Upstream tests ported. |
| `src/mail_templates.rs` | lib/mail-templates.ts | identical HTML/text from `messages/*/mail.json`; logo by CID. Upstream tests ported. |
| `src/mail.rs` | lib/mail.ts | lettre (rustls, Mozilla roots): `tls`=Wrapper, `starttls`=Required, `none`=Opportunistic; transport cached by fingerprint; dev logs instead of sending; errors redacted; `SmtpFailure` classified in nodemailer's vocabulary for `sendTestMail`. Not exercised against a server yet. |
| `src/password.rs` | lib/password.ts | added `hash_password`/`verify_password` (scrypt$ F format) and `check_rate_limit`/`reset_rate_limit` (fixed window, insertion-order eviction). srv-library owns the rest of this file and may have its own copy: keep either, they are faithful ports. |

New crates in `Cargo.toml`: `aes-gcm 0.10`, `sha1 0.10`, `hkdf 0.12`,
`unicode-normalization 0.1`, `lettre 0.11` (default-features off; smtp-transport,
builder, hostname, tokio1-rustls-tls).

## Not started / remaining (in the order I planned)

1. `auth/validate.rs`: zod-like body/query validation with the exact messages
   (verified on the reference, see "Wire facts" below).
2. `auth/adapter.rs` (internal adapter): find/create user (admin plugin default
   role + lib/auth.ts user.create hook; FIX 3 = `BEGIN; SELECT
   pg_advisory_xact_lock(..); count users; insert user; COMMIT`), accounts,
   sessions (createSession with the admin ban check, ip/user-agent), verification
   values (find deletes all expired rows first; consume), update_user (sets
   `updated_at`). Bind the sign-up additional fields as text + casts so
   `hideUploads:"zz"` fails in Postgres (422 FAILED_TO_CREATE_USER) as upstream.
3. `auth/api.rs` router for `/api/auth/{*all}` (GET+POST only; HEAD answers 404;
   other methods 405, OPTIONS 204 via the edge): better-call path rules,
   rate limiter (memory; default rules sign-in/sign-up/change-password/change-email
   10s/3, request-password-reset + send-verification-email 60s/3, plugin
   `/two-factor/*` 10s/3, custom sign-in/email + sign-up/email 60s/30, else 60s/120;
   429 `{"message":"Too many requests. Please try again later."}` + `X-Retry-After`;
   counted in onResponse, checked before routing, key `ip|path`; FIX 7: ip from
   X-Real-IP only with TRUST_PROXY, else the socket address), getBody media types
   (415 messages), origin middleware (cookie => Origin/Referer must be APP_URL's
   origin; callbackURL/redirectTo/errorCallbackURL/newUserCallbackURL checked,
   relative paths regex; a truthy NON-string value crashes upstream = 500 empty),
   formCsrf for sign-in/sign-up, before hook = registration gate (runs BEFORE
   validation), after hooks = 2FA on sign-in, admin filter on list-sessions.
   Response: better-call cookie format (`SetCookie::to_better_call_string`),
   written directly on the response (not via ResponseCookies, which is Next format).
4. Endpoints: get-session, sign-in/email, sign-up/email, sign-out,
   request-password-reset, reset-password/:token, reset-password, verify-email,
   send-verification-email, change-password, update-user, list-sessions,
   revoke-session(s), revoke-other-sessions, verify-password, change-email
   (disabled: 400), delete-user (disabled: 404 no body), update-session, list-accounts,
   unlink-account, ok, error (production: 302 `/?error=..`), two-factor/* (enable,
   disable, get-totp-uri, verify-totp, verify-backup-code, generate-backup-codes,
   send-otp = OTP_NOT_CONFIGURED, verify-otp), admin/* (all 15). Social/OAuth
   endpoints planned as documented deviations.
5. Server-side calls for actions (same effects, cookies into ResponseCookies):
   getSession (exists in `auth/session.rs`), signInEmail (asResponse true/false),
   sendVerificationEmail, listSessions, revokeSession, revokeOtherSessions,
   revokeUserSessions, createUser, removeUser, setRole, banUser, unbanUser.
6. Lib ports not started: invites, credentials, admin-users, audit helper taking
   the request, releases-core/releases, authorize/loopback, data/sessions,
   data/admin, data/user-prefs, theme-api, client-settings.
7. Routes: /api/me, /api/releases/app (FIX 4: `read_json_object`), /download/app,
   /api/theme, /api/client-settings.
8. Actions: account, admin (loadUserDetail, updateUserAccount, setUserRole,
   setUserBanned, createAccount, resendInvite, setUserEmailVerified,
   sendVerificationLink), authorize, invite, login-identifier, mail, registration,
   releases; verify tokens.rs. RPC framework: add `"revalidate": true` when the
   response sets/deletes any cookie (CONTRACT 7.1) - not in the skeleton yet.
9. Page loaders: (auth) layout + login, register, 2fa, forgot, reset,
   invite/[token], authorize; (app) layout (needs folder tree, kind counts, user
   stats, background, avatar, duplicate reminder from srv-library modules);
   dashboard/profile, tokens, admin; `/` (redirect), welcome, download.
   Layout/props shapes: `_wt/web-spa/server/web/src/contract/layouts.ts` and the
   `...Props` types in `_wt/web-spa/server/web/src/pages/**`.
10. Tests still to port: authorize-loopback, client-settings, default-password,
    release-*, sessions, theme-api, theme-route, token-scope-actions,
    panel-reachability, i18n-parity (relevant parts).
11. Parity scripts under `server/tests/` and the cross-compatibility proof.

## Wire facts already verified on the reference (keep for the router)

- `GET /api/auth/get-session` without cookie: `200`, body `null`,
  `content-type: application/json`. `POST` to it: `405 METHOD_NOT_ALLOWED`
  `{"message":"POST method requires deferSessionRefresh to be enabled in session config","code":"METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED"}`.
- Unknown `/api/auth/x`: `404` empty, no content type. `HEAD`: 404. `PUT`: Next 405.
  `/api/auth` alone: the Next not-found page. `/api/auth/ok`: `{"ok":true}`.
  `/api/auth/error`: `302 location: /?error=UNKNOWN` (production).
- POST without content type: `415 415` `{"message":"Content-Type is required. Allowed types: application/x-www-form-urlencoded, application/json","code":"UNSUPPORTED_MEDIA_TYPE"}` (sign-in/up list both types; other endpoints only application/json). Invalid JSON: `500` empty body.
- Validation: `400 400` `{"message":"[body.email] Invalid input: expected string, received undefined; [body.password] ...","code":"VALIDATION_ERROR"}`; `[body] Invalid input: expected object, received array`; sign-up (object AND record) adds `; [body] Invalid input: expected record, received array`; `z.email()` -> `Invalid email address` (regex: no leading dot, no `..`, local `[A-Za-z0-9_'+\-.]*` ending `[A-Za-z0-9_+-]`, labels `[A-Za-z0-9][A-Za-z0-9-]*`, TLD `[A-Za-z]{2,}`); nonempty on `""` -> `Too small: expected string to have >=1 characters`, on `[]` additionally `Too small: expected array to have >=1 items`.
- Sign-in bad credentials: `401 UNAUTHORIZED` `{"message":"Invalid email or password","code":"INVALID_EMAIL_OR_PASSWORD"}`; bad email format: `400 BAD_REQUEST` INVALID_EMAIL. Callback `//evil.com`: `403 FORBIDDEN` INVALID_CALLBACK_URL.
- Sign-up: `role`/`userCode` in body: `400` `{"message":"role is not allowed to be set","code":"FIELD_NOT_ALLOWED"}`; short/long password PASSWORD_TOO_SHORT/LONG; success body field order `name,email,emailVerified,image,createdAt,updatedAt,role,banned,banReason,banExpires,twoFactorEnabled,userCode,usedQuota,maxQuota,hideUploads,copyRaw,theme,locale,id`, cookie `better-auth.session_token=<signed>; Max-Age=604800; Path=/; HttpOnly; SameSite=Lax`. Row: `theme='modern'` written explicitly; `session.ip_address = ''` when no X-Real-IP (production), user agent stored.
- Production reference with no X-Real-IP: Better Auth skips rate limiting entirely. For parity runs send `X-Real-IP` and run Rust with `TRUST_PROXY=true`.
- getSessionFromCtx (session middleware) refreshes the DB row but its cookies never reach the response (better-auth overwrites `ctx.context.responseHeaders`); only `/get-session` itself re-issues the cookie.

## How to build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd _wt/srv-auth/server
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## Things created outside the repo (names only, never values)

- **Database** `modlight_srv_auth` on the shared dev Postgres (127.0.0.1:55421),
  created empty (not migrated yet). Drop it when no longer needed.
- **Scratch reference stack** `_scratch/srv-auth/ref/docker-compose.yml`
  (compose project `modlight-srvauth-ref`): the original image
  `modlight-wzorzec-app:latest` on its own Postgres, app on 127.0.0.1:8424,
  Postgres on 127.0.0.1:55424, `APP_URL=http://localhost:8424`,
  `NODE_ENV=production`. Its own generated secrets are in
  `_scratch/srv-auth/ref/.env` (POSTGRES_PASSWORD, AUTH_SECRET; mode 600).
  Containers were removed with `docker compose down` at hand-off; the volumes
  (`modlight-srvauth-ref_pgdata`, `_appdata`) are kept. Restart with
  `docker compose -f _scratch/srv-auth/ref/docker-compose.yml up -d`.
  State inside it: `settings.register_enabled` set to `on` (by SQL, for probing),
  one admin `admin@srvauth.test` created by sign-up, password in
  `_scratch/srv-auth/qa.env` (also holds a not-yet-used `user@srvauth.test` entry).
- Nothing was created on the shared reference (8420): no accounts, no tokens,
  no settings changes.
- The worktree's `server/target` was seeded by cloning `_wt/srv-skeleton/server/target`.

## Gotchas

- The reference's APP_URL is `http://localhost:<port>`: send `Origin:
  http://localhost:<port>` or Better Auth answers INVALID_ORIGIN once a cookie is
  present.
- zsh: `echo ==` is a command substitution; quote it in probe scripts.
- `KeyInit` (aes-gcm / chacha) and `hmac::Mac` both define `new_from_slice`:
  call `<Hmac<_> as Mac>::new_from_slice`.
- `scrypt::Params::new` needs `len` in 10..=64, but `scrypt()` derives
  `output.len()` bytes regardless.
- Front-door cookies (`ResponseCookies`) are serialised in Next's format; Better
  Auth's own endpoints must write better-call's format on the response instead.
