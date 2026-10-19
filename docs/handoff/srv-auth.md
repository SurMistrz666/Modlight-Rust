# srv-auth handoff (2026-10-19)

Branch `srv-auth`, worktree `_wt/srv-auth`. Task: port accounts,
authentication and administration to Rust with 1:1 behaviour on the same
schema: the Better Auth compatibility layer, the lib modules, `/api/auth/*`,
the routes, the actions, the page loaders, the tests, the parity scripts and the
cross-compatibility proof. Helpers worked on branches `srv-auth-{r,t,m,a,p,d,x}`
(worktrees `_wt/srv-auth-*`) and were merged here. Their briefs are in
`_scratch/srv-auth/briefs/`.

Build and verify:

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd _wt/srv-auth/server
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## What is ported (upstream -> Rust)

### Better Auth (better-auth 1.6.13, better-call 1.3.5, zod 4.4.3)

| Rust | Upstream | Notes |
|---|---|---|
| `src/auth/api.rs` | `toNextJsHandler(auth)`, better-call router | `/api/auth/{*all}` (GET+POST), 32 MiB body cap, `to_response` with better-call cookie format and reason phrase = status name |
| `src/auth/endpoint.rs` | `createAuthEndpoint`, `toAuthEndpoints` | `EndpointCtx`, `Endpoint`, middlewares (session, sensitive, fresh, admin, formCsrf, originCheck), `call()` = before hook (registration gate on the RAW body), validation, handler, after hooks |
| `src/auth/routes/*.rs` | `api/routes/*.mjs` | get-session, sign-in/email, sign-up/email, sign-out, request-password-reset, reset-password(/:token), verify-email, send-verification-email, change-password, verify-password, update-user, change-email (disabled), delete-user (disabled), list/revoke sessions, update-session, list-accounts, unlink-account, ok, error |
| `src/auth/two_factor.rs` | `plugins/two-factor` | enable, disable, get-totp-uri, verify-totp, generate/verify backup codes, send-otp / verify-otp (OTP not configured), sign-in interception (after hook), trust-device cookie |
| `src/auth/admin.rs` | `plugins/admin` | all 15 admin endpoints, `has_permission`, list-sessions filter after hook, unban on expiry |
| `src/auth/adapter.rs` | internal adapter + drizzle adapter | users, accounts, sessions, verification values; FIX 3: first-account admin inside a transaction holding `pg_advisory_xact_lock` |
| `src/auth/validate.rs` | zod 4 schemas used by the endpoints | exact messages (`[body.email] Invalid input: ...`), `z.email()` regex, coercions |
| `src/auth/origin.rs` | `originCheckMiddleware`, form CSRF | callbackURL / redirectTo / errorCallbackURL / newUserCallbackURL checks |
| `src/auth/rate_limit.rs`, `src/auth/ip.rs` | BA rate limiter, `getIp` | onRequest check / onResponse count, key `ip|path`, default and custom rules; FIX 7: X-Real-IP only with TRUST_PROXY, else the socket address |
| `src/auth/session.rs`, `cookies.rs` | `getSession`, `getSessionFromCtx`, cookie utils | signed cookie `value.base64(HMAC-SHA256)`, dont_remember, refresh policies (DbAndCookie for /get-session, DbOnly for the session middleware: the refreshed cookie is lost upstream) |
| `src/auth/email.rs` | `emailVerification` / `sendResetPassword` callbacks of lib/auth.ts | mail errors swallowed (`runInBackgroundOrAwait`) |
| `src/auth/server.rs` | `auth.api.*` called from server code | `call_endpoint` runs the full pipeline (no rate limit, no origin check: there is no Request); `sign_in_email`, `send_verification_email`, `create_user`, `set_role`, `ban_user`, `unban_user`, `revoke_user_sessions`, `remove_user`, `list_sessions`, `revoke_session`, `revoke_other_sessions`. `ServerCall::apply_cookies(ctx)` is the `nextCookies()` plugin |
| `src/auth/crypto.rs`, `password.rs`, `error.rs` | crypto, scrypt hash, APIError | `two_factor.secret` / `backup_codes` = XChaCha20-Poly1305 with key SHA-256(secret), hex; TOTP HMAC-SHA1 over the secret STRING; scrypt `salt:key` N=16384 r=16 p=1 |

### Lib modules

`invites.rs`, `credentials.rs`, `admin_users.rs`, `audit.rs` (rest),
`releases_core.rs` + `releases.rs`, `authorize/{mod,loopback}.rs`,
`data/sessions.rs`, `data/admin.rs` + `data/admin_disk.rs`,
`data/user_prefs.rs`, `theme_api.rs`, `client_settings.rs`, `js_json.rs` (V8
JSON.parse/stringify semantics), `mail.rs` + `mail/smtp.rs` +
`mail/address.rs` (nodemailer 8.0.10 behaviour), `mail_settings.rs`,
`mail_templates.rs`, `registration.rs`, `secret_box.rs`, `settings.rs`,
`password.rs` (rate limiter, client_ip, file-password hash).

### Routes

`/api/auth/*`, `/api/me`, `/api/theme`, `/api/client-settings`,
`/api/releases/app` (FIX 4: `read_json_object`), `/download/app`.

### Actions (`/_rpc/<module>/<export>`)

`account` (all 20), `authorize` (approveDesktopClient), `invite`
(previewInvite, acceptInvitation, resendVerification), `login-identifier`
(signInWithIdentifier), `mail` (6), `registration` (4), `releases`, `tokens`,
and from `admin.ts` (helper D, branch `srv-auth-d`): loadUserDetail,
updateUserAccount, setUserRole, setUserBanned, createAccount, resendInvite,
setUserEmailVerified, sendVerificationLink. The other admin.ts exports belong to
srv-library and go into the same `src/actions/admin.rs` at integration. The RPC
envelope carries `"revalidate": true` whenever the action set or deleted a
cookie.

### Page loaders

Root layout (`layouts.root.config`), `(auth)` layout, `(app)` layout (folder
tree, kind counts, user stats, background, avatar, duplicate reminder),
`/`, `/welcome`, `/download`, `/login`, `/register`, `/forgot`, `/reset`,
`/2fa`, `/invite/[token]` (token in the page spelling), `/authorize`,
`/dashboard/profile`, `/dashboard/tokens`, `/dashboard/admin`.

## Parity (against the scratch reference, same input on both sides)

Stacks: reference `modlight-srvauth-ref` (app 127.0.0.1:8424, Postgres
55424, secrets in `_scratch/srv-auth/ref/.env`), Rust on 8423
(`_scratch/srv-auth/restart-rust.sh`, database `modlight_srv_auth` on the dev
Postgres, AUTH_SECRET in `_scratch/srv-auth/rust/rust.env`). From the
worktree root, with `S=/Users/marcinasdfg/Desktop/modlight/_scratch/srv-auth`,
`R="--ref http://localhost:8424 --rust http://localhost:8423"` and
`E="--ref-env $S/ref/.env --rust-env $S/rust/rust.env"`:

| Script | Result on the merged branch |
|---|---|
| `server/tests/parity/auth.py $R $E` (DESTRUCTIVE: truncates the auth tables of both databases) | 180/180 |
| `server/tests/parity/auth_actions.py $R $E --scratch $S` | 89/89 |
| `server/tests/parity/auth_pages.py $R $E` | 104/104 |
| `server/tests/parity/account.py $R --scratch $S/acct --ref-app modlight-srvauth-ref-app-1 --ref-pg modlight-srvauth-ref-postgres-1 --rust-db modlight_srv_auth` | 559/559 (profile page with the real (app) layout) |
| `server/tests/parity/releases.py $R --ref-app modlight-srvauth-ref-app-1 --ref-pg modlight-srvauth-ref-postgres-1 --rust-db modlight_srv_auth $E` | 171/171 |
| `server/tests/parity/api_misc.py $R --ref-container modlight-srvauth-ref-postgres-1 --rust-db modlight_srv_auth $E` | 399/399 |
| `server/tests/parity/authorize.py $R --ref-app modlight-srvauth-ref-app-1 --ref-db-container modlight-srvauth-ref-postgres-1 --rust-db modlight_srv_auth $E` | 155/155 |
| `server/tests/parity/admin_settings.py` (same flags as authorize.py) | 436/436 |
| `server/tests/parity/panel.py $R --ref-app-container modlight-srvauth-ref-app-1 --ref-db-container modlight-srvauth-ref-postgres-1 --rust-db modlight_srv_auth $E --rust-storage $S/rust/data/storage` | 79/83: the 4 failures are its "scenario's state on the reference" self-checks, which expect DEFAULT_USER_QUOTA=20 GB (helper A's stack); this reference runs with -1. Every reference-vs-Rust comparison passes |
| `admin_users.py` (helper D), `crosscompat.py` (helper X) | see the final report of srv-auth |

## Deliberate differences

- `ml_skin` cookie (or a stored `user.theme`) naming an `Object.prototype`
  member (`constructor`, `toString`, `__proto__`, ...): upstream's
  `resolveTheme` returns that member (`v in THEME_ALIASES`) and every page
  answers 500 (verified on the reference); Rust renders the default theme.
  `/api/theme` reproduces upstream's JSON quirk for such a value (helper T).
- A server advertising only XOAUTH2: upstream throws an uncaught TypeError;
  Rust reports kind "unknown" (helper M).
- Lone surrogates produced by `slice()` in the middle of an emoji (session
  user agents, client names): Rust strings cannot hold them, U+FFFD instead.
- TLS error wording from rustls instead of OpenSSL (kind and code match);
  DNS through getaddrinfo only.
- A lying Content-Length on /api/client-settings: Rust answers 413 at once,
  the reference waits for the body.
- `read_json_object` (shared `api_json.rs`) still parses with serde_json: a
  body with a lone surrogate or nested more than 128 levels gets 400 where
  upstream accepts it (helper T; the fix belongs to whoever owns api_json.rs).
- Document redirects (307) have no body; Next sends its HTML error shell.

## Wire facts verified on the reference (keep)

- `GET /api/auth/get-session` without cookie: `200`, body `null`. `POST`: `405
  METHOD_NOT_ALLOWED` `METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED`.
- Unknown `/api/auth/x`: `404` empty. `HEAD`: 404. `/api/auth/error`: `302
  location: /?error=UNKNOWN` (production).
- POST without content type: `415 415` UNSUPPORTED_MEDIA_TYPE (sign-in/up list
  both media types). Invalid JSON: `500` empty body.
- Validation: `400 400` `{"message":"[body.email] Invalid input: expected string, received undefined; ...","code":"VALIDATION_ERROR"}`.
- Sign-up success field order `name,email,emailVerified,image,createdAt,updatedAt,role,banned,banReason,banExpires,twoFactorEnabled,userCode,usedQuota,maxQuota,hideUploads,copyRaw,theme,locale,id`; `theme='modern'` written explicitly.
- Production reference without X-Real-IP: Better Auth skips rate limiting.
  Parity: send X-Real-IP and run Rust with TRUST_PROXY=true. lib/password's
  `clientIp` always reads X-Real-IP upstream (FIX 7 in Rust).
- The session middleware refreshes the DB row but its cookie never reaches the
  response; only `/get-session` re-issues the cookie.
- Server actions: React's decodeReply drops object keys whose value is
  undefined; a result that points at an `E` row (directly or deeper) rejects
  the whole call (parity.py handles both).
- `nextCookies()`: cookies of a server-side `auth.api.*` call become the
  action's cookies in Next's format (one per name, last value wins), which
  also marks the action as revalidating.

## Things created outside the repo (names only, never values)

- Databases on the dev Postgres (127.0.0.1:55421): `modlight_srv_auth` (mine);
  helpers' databases `modlight_srv_auth_*` (see the final report for which
  were dropped).
- Scratch reference stack `_scratch/srv-auth/ref` (project
  `modlight-srvauth-ref`); helpers' stacks under `_scratch/srv-auth-*/`
  (each brought down with `docker compose down -v` by its helper).
- `_scratch/srv-auth/acct/` holds symlinks to the two env files for account.py.

## Gotchas

- Stop servers only by PID or port (`lsof -tiTCP:<port> -sTCP:LISTEN`); other
  agents run the same binary.
- The reference's APP_URL is `http://localhost:<port>`: send that Origin or
  Better Auth answers INVALID_ORIGIN once a cookie is present.
- The reference's root layout passes every message to the client, so a
  message text is always somewhere in the RSC payload: look for it as a
  rendered node (`"children":"<text>"`) or for a component's props.
- `KeyInit` (aes-gcm / chacha) and `hmac::Mac` both define `new_from_slice`.
- Front-door cookies (`ResponseCookies`) are Next's format; Better Auth's own
  endpoints write better-call's format.
