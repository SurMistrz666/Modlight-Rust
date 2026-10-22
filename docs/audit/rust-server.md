# Security audit: the Modlight server in Rust (the port)

Scope: `/path/to/modlight/server` (axum + sqlx + Postgres, ~90 thousand lines),
branch `main` (at the time of the audit HEAD `cfe7411`, the server code unchanged since merge `0933c6d`).
The TypeScript original (`upstream/Modlight`) is treated as the specification; the audits of the original are in
`docs/audit/original-server-accounts-api.md` and `docs/audit/original-server-files-exfiltration.md`, the fixes to implement are in
`server/docs/PORT.md` section 12 and `server/docs/SKELETON.md` section 9 (items 26-40).

Method: static analysis of the whole `src/` tree (auth and sessions, tokens/scopes, the RPC framework and the
authorization of every action, page loaders, ShareX/tus uploads, storage drivers, media
serving and gates, ffmpeg/libvips transformations, mail, admin, docker) plus dynamic tests on a
**disposable** Docker stack (`modlight-secreview`, 127.0.0.1:8449, its own `.env`, its own database and
volumes, removed with `down -v` after the work). The owner's stacks (8430, 8420, the dev database 55421) were
not touched. Secrets were not printed.

Date: 2026-10-20.

## Short verdict

**No Critical or High vulnerability introduced in the translation was found.** The port is faithful and
unusually carefully hardened; authorization, narrowing by owner, token scopes and the view
gates work as designed and were confirmed dynamically. All 7 fixes
from the audit of the original are present and **effective** (6 of 7 reproduced on the live stack; fix 3
confirmed statically and by a test). The code **does not send data outside** other than to the services that the
administrator configures (SMTP, S3, Postgres). The findings are solely Low/Informational matters,
for the most part a faithful reproduction of the original's behavior (parity), not a regression of the port.

---

## Does the code send data outside

A full review of outgoing connections (grep: `reqwest`, `TcpStream`, `UdpSocket`, `tokio::net`,
`std::net::Tcp`, `hyper*client`, `aws_sdk`, `lettre`, `rustls`, `to_socket_addrs`, `lookup_host`,
`curl`, `fetch`). Dependencies: `Cargo.toml` does not contain `reqwest`; the only network libraries are
`aws-sdk-s3`, `lettre`, `sqlx` (since 2026-10-21 also the `hyper-util` + `hyper-rustls` client, used
solely by `turnstile.rs`).

| Location | Destination | What data | Does the administrator decide? |
|---|---|---|---|
| `mail/smtp.rs`, `mail.rs` (lettre, rustls) | the SMTP server from the configuration | verification/reset/invitation e-mails to users | YES (host from env/panel; no host = no sending) |
| `storage/s3.rs`, `storage/s3/lib_storage.rs`, `upload/tus_server/s3_store.rs` (aws-sdk-s3) | `S3_ENDPOINT` from the configuration | users' files | YES (only when `STORAGE_DRIVER=s3`) |
| `db/*`, `main.rs` (sqlx) | `DATABASE_URL` | application data | YES |
| `turnstile.rs` (hyper-util + hyper-rustls, ring, Mozilla roots), added 2026-10-21, after the audit | `https://challenges.cloudflare.com/turnstile/v0/siteverify` (address fixed in the code) | the token from the widget in the form, the Turnstile secret key and the visitor's IP address (`remoteip`, the same rule as the limits: `X-Real-IP` only with `TRUST_PROXY`); on "Sprawdź" (Check) and a save that enables it in the panel: the secret key with a Cloudflare test token | YES (only when the administrator enables Turnstile in the panel or sets both `TURNSTILE_*`; otherwise zero connections) |
| `main.rs::health()` | 127.0.0.1:PORT (loopback) | one `GET /api/health` request for the container healthcheck | n/a (local) |

- **No SSRF.** `routes/api/link_info.rs` + `link_info/parse.rs` answer solely from a database
  row (`get_media`/`SELECT`), never fetch the address being described, and reject hosts outside
  the deployment (`LinkHosts::is_known_host`); confirmed live (a foreign host -> `ForeignHost`).
- `ffmpeg/ffprobe`: `-protocol_whitelist file,pipe`, a forced `-f`, `-use_absolute_path 0` for
  mov, `env_clear()` (except `PATH`), `shell:false`, its own process group with a kill after the deadline
  (`media/ffmpeg.ts` -> `media/ffmpeg.rs`). No argument injection and no fetching over the network.
- `libvips`: loaders from the sharp-libvips allow-list (SVG is loaded, but served as `text/plain`;
  no network loader).
- Frontend (`server/web`): `fetch` only to the own origin (`/_page`, `/_rpc`, `/api/*`) and to
  `http://127.0.0.1:<port>` for the desktop handoff (RFC 8252, a separate CSP). The only external
  URLs are static links to GitHub on the landing page. No telemetry/beacons/analytics.
- **Cloudflare Turnstile (since 2026-10-21, optional)**: when the administrator enables it, the server on
  sign-in, registration, a password reset request and accepting an invitation sends Cloudflare the
  visitor's token and IP address (10 s limit, no response = refusal), and the browser loads
  `https://challenges.cloudflare.com/turnstile/v0/api.js` (the CSP allows this origin in `script-src`
  and `frame-src` only then). The secret key and the token never reach the logs; the secret key lies
  in the database encrypted (secret box, purpose `turnstile`). Disabled = no script, no connections, CSP
  unchanged. Details: `server/docs/SKELETON.md` section 9, item 43.
  `index.html`: the inline script is the theme bootstrap (`localStorage`), with no network.

**Verdict: the server does not exfiltrate data. No backdoor, no hidden account, no master password, no telemetry
and no "phone home". The only network outputs are SMTP/S3/Postgres configured by the administrator
and a local healthcheck.** Since 2026-10-21 a third one, also enabled only by the administrator, is
Cloudflare Turnstile (above).

---

## Fixes from the audit of the original (PORT.md section 12)

| # | Fix | Status | Evidence |
|---|---|---|---|
| 1 | The view limit (maxViews) cannot be bypassed with `Range`/`Sec-Fetch-Dest`/`w,h,motion` | **Effective (reproduced)** | see below |
| 2 | ShareX: authorization before the body, quota/space on chunked bodies | **Effective (reproduced)** | see below |
| 3 | The first admin in a transaction with a lock | **Effective (statically + test)** | `auth/adapter.rs:251` |
| 4 | `POST /api/releases/app`: body limit + Content-Type | **Effective (statically)** | `routes/api/releases_app.rs:165` |
| 5 | The delete token compared in constant time | **Effective (statically)** | `routes/media_delete.rs:38` |
| 6 | `/oembed` of a file with a limit without bytes | **Effective (reproduced)** | see below |
| 7 | `X-Real-IP` only with `TRUST_PROXY` | **Effective (statically + test + live)** | `auth/ip.rs:134`, `password.rs:11` |

### Fix 1 (view limit / "burn after viewing") - `media/serve.rs`, `routes/media_action.rs`, `pages/media.rs`
- Core: `lookup_media` distinguishes `Live`/`Spent`/`Missing`; `view_budget()` (`media/serve.rs:336`)
  enforces that for a file with `max_views` every request carrying paid content is a view, unless it
  carries a signed pass `ml_view_<id>` (HMAC-SHA256 from `AUTH_SECRET`, httpOnly, SameSite=Lax,
  `constant_time_eq` on verification) minted by an earlier counted view. `consume_view`
  counts atomically (`UPDATE ... WHERE views < max_views RETURNING`).
- **Reproduced** on the 8449 stack: a file with `maxViews=1`, the attack `GET /raw` with `Range: bytes=1-` ->
  the first request 206 and **consumed the view** (`views` 0->1), subsequent `Range: bytes=1-` and
  `Sec-Fetch-Dest: image` -> 404. A plain `GET /raw` of another file: the first 200, the second 404.
  In the original the same sequence gave endless 206 without counting (A1 of the audit).

### Fix 2 (ShareX disk DoS / anonymous write) - `upload/sharex.rs`
- The header token (`x-api-key`/`Authorization`) is authenticated **before** the body is read
  (`:185`); without a token the stream is cut after `ANON_MAX_BYTES` (8 MiB, overridable by
  `LEGACY_UPLOAD_MAX_SIZE`); the quota and free space are checked every 8 MiB while streaming
  (`:451-479`). `Content-Length > 0` is checked up front anyway (`:196`).
- **Reproduced**: an anonymous upload of 10 MB -> `413` "File too large for an unauthenticated upload";
  anonymous without a token -> `400 Token not specified`. In the original up to ~512 MB landed on disk before
  rejection, and a chunked body bypassed the preliminary check.

### Fix 3 (first admin TOCTOU) - `auth/adapter.rs`
- `create_user` opens a transaction, takes `SELECT pg_advisory_xact_lock($1)`
  (`USER_CREATE_LOCK_KEY`), counts `count(*)` and decides the role in the same transaction (`:250-264`).
  Injecting `role:"admin"` in `/sign-up/email` is rejected (`FIELD_NOT_ALLOWED`); **confirmed
  live** (`HTTP 400 "role is not allowed to be set"`). Test: `security_fixes_db.rs`
  `fix3_concurrent_first_sign_ups_make_exactly_one_admin`.

### Fix 4 (release publishing) - `routes/api/releases_app.rs`
- `post_release` reads the body through `read_json_object(..., JSON_BODY_MAX=16 KiB)` with the requirement of
  `application/json` (415) and a limit (413), after the `require_admin_caller` gate (token scope `admin`
  AND the account's `admin` role: two gates, `:131-170`).

### Fix 5 (delete token in constant time) - `routes/media_delete.rs:38`
- `constant_time_eq(token, media.delete_token)` (no early exit), a fallback to an API token
  narrowed to the owner. A wrong token -> `404` (no oracle); confirmed live.

### Fix 6 (`/oembed` of a file with a limit) - `routes/oembed.rs:96`
- A file with `max_views` returns `{version,title,provider_name,provider_url,cache_age,type:"link"}`
  without `url`/dimensions/HTML. **Reproduced**: the same file without a limit -> with `url`; after setting
  `maxViews` -> `type:"link"` without `url`.

### Fix 7 (`X-Real-IP` only behind a trusted proxy) - `auth/ip.rs`, `password.rs`
- `get_ip`/`client_ip` trust `X-Real-IP` only when `TRUST_PROXY=true`, otherwise they take the socket address
  (`to_canonical()`). `setup-local.sh` sets `TRUST_PROXY=false` for the loopback stack.
  Tests: `password::tests::honours_x_real_ip_only_behind_a_trusted_proxy`,
  `auth::ip::tests::real_ip_only_behind_a_trusted_proxy`.

### Intended deviations (SKELETON.md 9) - they open nothing
Items 26-42 and 15 (prototype pollution in `pastes`), 20 (libvips loaders),
31/37 (lookups by literal keys instead of prototype ones), 38 (tus finished on S3/local),
39 (deleting the old avatar/background only for own keys) were checked. Fixes 38 and 39 close real holes
of the original:
- **38** (confirmed live): after a tus upload finishes, `GET`/`HEAD` -> `404`, and `DELETE` does not
  remove the media row (the file is still `200`). In the original on S3, `GET` streamed the object
  to anyone with the tus URL regardless of privacy, and `DELETE` removed the object from under the row.
- **39**: `discard()` (`avatar_store.rs:140`) deletes only the keys `avatars/<userId>-...` of that account,
  because `user.image` can also be set through the Better Auth `update-user`, hence the hard prefix
  gate that protects against deleting someone else's object.

---

## Logic errors

No logic errors resulting in data loss, an authorization bypass or wrong ordering were found
beyond the minor items listed below. Checked and correct (noted because they were in scope):

- `/api/uploads` PATCH/DELETE narrowed to the owner also for an admin (no admin privilege on
  this route, parity). **Reproduced**: an admin's `full` token on user2's file -> `affected:0`.
- Token scopes (`upload`/`read`/`write`/`full`) are enforced on every call
  (`api_scopes.rs`, `api_token.rs`). **Reproduced**: a `read` token -> upload `403`,
  `upload` -> `GET /api/uploads` `403 insufficient_scope`, `upload` -> `/api/me` OK.
- Admin: self-demotion of the role and self-ban rejected (`actions/admin.rs:568-575`), deleting
  one's own account rejected (`admin_library.rs`). Narrowing a token is a subset test of capabilities with
  a WHERE on the old scope (`is_narrowing`, `set_scope_if`).
- The RPC framework: origin = the request host (`rpc/mod.rs:318`), `X-Modlight-Rpc` in {1,form}, an unknown
  action 404, a body > 1 MiB -> 413. **Reproduced**: a missing/wrong Origin -> 403, a missing header -> 403,
  a 1.2 MB body -> 413.
- The JSON parser (`js_json.rs`) is iterative: no stack overflow on deep nesting;
  pagination with a cap (`MAX_PAGE`), `parse_range` on `f64` with saturation (no overflow
  in ranges/quotas).

### [Low] Queries built by concatenation in a handler (admin list and filters) - `auth/admin.rs:381-414, 426-490`
- `list_users` builds `WHERE`/`ORDER BY` with `format!`. Confidence: **checked, no injection**.
  Columns go through `user_field_column()` (an allow-list of names -> constant identifiers),
  values go as parameters (`$n::text::ty` / `LIKE $n`), the sort direction is the literal
  `ASC/DESC`, `LIMIT/OFFSET` are cast to `i64`. The route is protected by the admin plugin. The other
  `format!` SQL is safe in the same way: `uploads.rs`, `data/uploads.rs`, `data/folders.rs`,
  `folders.rs` (identifiers from constants, values as parameters). Left as an observation:
  the pattern "a column from an allow-list + a value as a parameter" requires that every future column
  still go through `user_field_column`/a constant.

### [Low/parity] The in-memory Better Auth rate counter without sweeping - `auth/rate_limit.rs`
- `memory()` (a HashMap) removes an entry only on the next `get` of the same key; there is no periodic
  cleanup. With `TRUST_PROXY=true` and a proxy that lets any `X-Real-IP` through, the number of
  keys could grow. This is a **faithful reproduction** of the better-auth 1.6.13 `memory` store (also
  without sweeping). In the loopback deployment (`TRUST_PROXY=false`) the key = the socket address, so it is
  bounded. By contrast, the own file counter (`password.rs`) DOES have sweeping (soft/hard limit).
  Proposal: consider periodic cleanup of `memory` (without changing client-visible behavior).

### [Informational/parity] The TOTP code comparison is not constant-time - `auth/crypto.rs:243`
- `verify_totp` uses `hotp(...) == code` (String). Identical to the original
  (`@better-auth/utils` `otp.mjs`: `otp === generatedOTP`). A 6-digit code, a +-1 window, short-lived,
  the route is throttled (`/two-factor/*` 3/10 s). Backup codes and tokens (unlock/render/view-grant,
  file passwords, account passwords) use `constant_time_eq`. Nothing to fix; noted for completeness.

**Status after the review (2026-10-20):** fixed. `verify_totp` computes all three steps and compares the code with `constant_time_eq` (a deliberate departure from the original, described in the function's comment).

### [Informational] The desktop token handoff without PKCE - `authorize/loopback.rs`, `actions/authorize.rs`
- `callback` is built solely from the **parsed port number** (`loopback_callback`, 1024-65535),
  never from a client string; `name` is sanitized; the token is minted in an action requiring a session, with scope
  `write` (the desktop will not publish a release). Consistent with RFC 8252, like the original. Unchanged.

---

## Things checked and fine

- **Sessions / Better Auth**: the cookie `better-auth.session_token` (prefix `__Secure-` on https),
  the value `token.base64(HMAC)`, signature verification, refresh policies (DbAndCookie/DbOnly/Skip)
  as in the Next contexts; a banned account is rejected by `resolve_api_owner`/sign-in, not by
  `getSession` itself (parity). A forged signature is rejected (a skeleton test).
- **Passwords**: scrypt in the Better Auth format (N=2^14), NFKC before hashing, legacy bcrypt
  `$2a/$2b/$2y`, `constant_time_eq`. `burn_hash` evens out the timing on "no such user" paths.
  Hashing on `spawn_blocking`.
- **2FA**: the secret and backup codes encrypted with XChaCha20-Poly1305 (key = SHA-256 of AUTH_SECRET),
  TOTP HMAC-SHA1, trust-device as an HMAC + a `verification` row, the backup-code update is
  atomic (`UPDATE ... WHERE backup_codes = $old RETURNING`).
- **API authorization**: the token before the session; a token present but invalid = refusal (no fallback to
  the session); confirmed live; the scope -> capability mapping consistent with `token-scope-coverage`.
- **Path traversal / arbitrary read-write**: `LocalDriver::full()` enforces the root prefix
  (`resolve_lexically` + `starts_with`); `userCode/code.ext` keys from a CSPRNG; the derived cache from an
  allow-list of variants; the tus `file_id` is decoded and rejects a server path segment; names in
  the album ZIP are flattened (`sanitize_entry_name`, `../` -> `_..`). Test: `..%2f..%2fetc%2fpasswd`
  live -> 404.
- **Stored XSS**: `serve_decision` is an allow-list: HTML/SVG/XML/JSON go out as
  `text/plain; charset=utf-8` + `nosniff`; real rendering only through `/render` with an HMAC ticket,
  the requirement `Sec-Fetch-Dest: iframe` and the CSP `sandbox allow-scripts; default-src 'none'`. The deletion
  page: `esc()` + `default-src 'none'`. The SPA payload: `json_for_html` escapes `<`/`>`/`&`/
  U+2028/9; the `<html>` attributes via `escape_attr`.
- **Decompression bombs**: `media/zip_guard.rs` actually inflates every entry with
  `max_output_length`, rejects ZIP64/unknown methods before mammoth; images are limited by
  `MAX_TRANSFORM_BYTES` (32 MB) and `MAX_THUMBNAIL_PIXELS` (64 Mpx) checked from metadata before
  decoding; `limit_input_pixels` for the background; everything through a shared limiter with a weight and a timeout.
- **Unsafe / FFI (libvips)**: `unsafe` is concentrated in `media/vips/*`; `Image`/`Blob` with a correct
  GObject reference count (`g_object_ref`/`unref`, `vips_area_unref`), input copied into
  `vips_tracked_malloc` and freed via `free_tracked`, the timeout callback with a `Box` freed once
  (`drop_timeout_state`). Outside vips: `libc::statvfs` (zeroed + an rc check), `kill(-pid)` on the
  ffmpeg process group, the FFI `vips_foreign_is_a_buffer`. `set_var("PANGOCAIRO_BACKEND")` once in a
  `OnceLock` before libvips starts (safe).
- **Panic from a request**: `App::handle` catches with `catch_unwind` -> 500; a review of `.unwrap()` in
  `routes/`, `actions/`, `pages/`, `rpc/`, `upload/`, `auth/routes/`: all hits other than
  `lock().unwrap()` are in `#[cfg(test)]`. `parse_metadata` `tokens[0]` is safe (split yields >=1).
- **Headers / CSP**: HSTS, nosniff, `X-Frame-Options: SAMEORIGIN`, `Permissions-Policy`,
  `frame-ancestors 'self'`, no `X-Powered-By`. Separate restrictive CSPs for `/raw`, `/render`,
  `/authorize`. `'unsafe-inline'` on the app CSP is a deliberate decision of the original (user content in
  separate policies).
- **Secrets in logs**: SMTP errors via `redact_secrets`, the `S3Config` Debug hides
  `secret_access_key`, dev-mail logs content only outside production (parity). `config.rs` refuses a
  production start with a missing/placeholder/too short `AUTH_SECRET` and default database credentials.
- **Docker/compose**: publishing only on `127.0.0.1`, Postgres without publishing, `user 1000:1000`,
  `cap_drop: ALL`, `no-new-privileges`, mem/pids/cpu limits, ffmpeg from the Debian repo (not from the network),
  the entrypoint only computes cgroup limits and starts migrations+server. `setup-local.sh`: secrets via
  `openssl rand`, `chmod 600`, it downloads and sends nothing, and preserves existing secrets.
- **No debug endpoints, backdoors, hard-coded credentials** (grep: TODO/HACK/bypass/master_key
  and external URLs: all hits in tests or comments).

---

## Dynamic test methodology (repeatable)

The disposable stack: a copy of `setup-local.sh` in `_scratch/sec-review/stack/.env` (APP_PORT=8449,
REGISTRATION_ENABLED=true), compose with `name: modlight-secreview` and port 8449, `up -d --pull
never`, `down -v` after the work. Confirmed: RPC CSRF/origin, role injection, token scopes,
IDOR between accounts, fix 1 (Range/Sec-Fetch-Dest), fix 2 (anonymous 10 MB), fix 6
(oembed), fix 38 (tus finished), body limits, path traversal, hiding of the admin route. The owner's
stacks were not touched; the disposable stack was removed along with its volumes.
