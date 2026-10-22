# Security audit: Modlight (server, accounts, authentication, API)

Scope: the Next.js 16 server + the custom `server.ts`, better-auth, drizzle/PostgreSQL. Subject of
the audit: `/path/to/modlight/upstream/Modlight`. Static analysis only
(reading the code); nothing from the repository was run.

Commit: `a59d03c` (a single commit, 2026-10-18).

## Summary

The code is unusually hardened and densely commented with security rationale.
I followed the paths from entry to effect for: registration and the admin bootstrap, sign-in,
2FA, password reset, invitations, API tokens and scope enforcement, every route handler in
`app/api/**`, the remaining `route.ts` files, the server actions in `app/actions/**`, the
desktop application authorization flow (loopback), release publishing, migrations and scripts.

I did not find:
- a backdoor, a hidden account, a master password or a magic header that bypasses sign-in,
- a user -> admin escalation path on an existing installation,
- an IDOR (every resource access is narrowed to the owner in the WHERE clause),
- a bypass of token scopes (upload/read/write/full are enforced on every call),
- data leaving the server (no code in this area sends data to foreign hosts;
  see the section "Sending data outside"),
- `eval`/`new Function`/`child_process` (other than `ffmpeg`/`ffprobe`, which have a
  protocol whitelist `file,pipe` and a forced demuxer `-f`).

The most important findings (details below):

| Severity | Location | Statement |
|---|---|---|
| Medium | `lib/upload/sharex.ts:206` (+ `server.ts`) | An unauthenticated caller can force up to ~512 MB to be written to disk per connection before the token is checked; without Content-Length the preliminary limit does not work -> disk DoS. |
| Medium | `lib/auth.ts:252-275` | Granting the "admin" role to the first account counts users without a lock (TOCTOU); it affects only an empty installation and gives no escalation on an existing one. |
| Low | `app/api/releases/app/route.ts:79-84` | The POST that publishes a release reads `req.json()` with no size limit and no Content-Type check; CSRF is mitigated by SameSite=Lax, but this is inconsistent with the rest of the API. |
| Low | `app/[userCode]/[mediaCode]/delete/[token]/route.ts:28` | The delete token is compared with a plain `===` (not in constant time); the risk is negligible given the high entropy, but it is inconsistent with `timingSafeEqual` elsewhere. |
| Informational | reverse proxy configuration | The whole rate-limit model trusts only `x-real-ip`; a proxy that does not overwrite it (or exposing the port directly) disables throttling of sign-in/key/file passwords. It is documented, but it is the biggest real foot-gun. |
| Informational | `lib/authorize/loopback.ts`, `app/(auth)/authorize` | The desktop token handoff (RFC 8252) has no PKCE; `state` is only reflected, not bound on the server side. Consistent with the standard; the residual risk requires a local listener. |

---

## Detailed findings

### [Medium] Unauthenticated write of a large body to disk before the token is checked (DoS)
- File: `lib/upload/sharex.ts` (meter: `:198-221`, preliminary check: `:126-134`), `server.ts:82-88`.
- Confidence: likely (the exact behavior depends on the filesystem and on concurrency).
- Code path: `POST /upload` and `POST /api/upload` are handled in the raw Node layer
  (before Next). Authentication from the header happens before any I/O only when the header is present
  (`bearerFromHeaders`). If the client does not send the header (legacy: the token as a form field),
  busboy streams the file to `tmpPath`, and the anonymous limit (`ANON_MAX_BYTES`, default
  **512 MB**, `LEGACY_UPLOAD_MAX_SIZE`) aborts only once it is exceeded. The preliminary
  `hasRoomFor(declared, ...)` works only for a finite `Content-Length > 0`
  (`:126-134`), so a **chunked request without Content-Length** bypasses it.
- Effect: an anonymous caller opens many chunked connections to `/upload`, and in each sends up to
  ~512 MB before the final missing token returns 400. The temporary files are cleaned up in `finally`,
  but during the transfer they occupy `UPLOAD_TMP_PATH` (the same filesystem as the storage),
  and the code itself warns elsewhere that a full disk "takes down every application on the host". There is no
  rate limit on this path for an anonymous caller.
- Input -> effect: `POST /api/upload`, `Transfer-Encoding: chunked`, no `x-api-key`, no
  `token` field, a body of more than a few hundred MB, N parallel connections -> temporary exhaustion of
  the storage's disk space.
- Proposal: require a token/session before a single byte of the body arrives (as on the tus path:
  `onUploadCreate` authenticates before writing), or hard-limit the anonymous buffer to
  a few MB and throttle tokenless `POST /upload` by `x-real-ip`. Consider rejecting
  chunked requests without `Content-Length` on this path.

### [Medium] Granting the admin role to the first account without a transactional lock (TOCTOU)
- File: `lib/auth.ts:252-275` (`databaseHooks.user.create.before`), supporting:
  `lib/registration.ts:161-164`, `lib/auth.ts:206-239`.
- Confidence: confirmed (that the count has no lock); the effect is limited.
- Code path: the hook runs `count()` over the users and, if 0, assigns the `admin` role. Two
  simultaneous "first" registrations on an empty installation can both read `n=0` and both
  get admin. Independently, the `evaluateSignUp` gate also counts (`isFirstAccount`).
- Effect: on an **empty** installation one can end up with two admins instead of one. On an
  **existing** installation `count>0`, so self-signup always gets the `user` role: there is no
  escalation. An explicitly supplied `admin` role is honored only from `/admin/create-user`
  (protected by the better-auth admin plugin), so an outsider cannot inject it.
- Input -> effect: two parallel `POST /sign-up/email` within the bootstrap window -> 2 admin accounts.
- Proposal: grant the first admin in a single transaction with a lock (e.g. `INSERT ...
  SELECT ... WHERE NOT EXISTS`, or an advisory lock), so that "being first" is decided
  atomically. Low priority because of the narrow window and no escalation on a live installation.

### [Low] The release-publishing POST has no body limit and no Content-Type check
- File: `app/api/releases/app/route.ts:75-88`.
- Confidence: confirmed.
- Code path: the handler calls `await req.json()` directly: there is no upper size limit
  (the rest of the API uses `readJsonObject`/`readCappedBody` with a limit) and no requirement for
  `application/json`, which in `app/api/shortlinks/route.ts:233-241`,
  `app/api/client-settings` and `app/api/theme` is deliberately used as a CSRF check
  for routes that accept the session cookie. The route is admin-only (`requireAdminCaller`), and CSRF
  is additionally mitigated by the default `SameSite=Lax` of the better-auth session cookie (not
  overridden in `lib/auth.ts`), so a cross-site request will not carry the cookie anyway.
- Effect: the real risk is minimal (it needs a victim admin and knowledge of the `code` of an existing
  public upload), but it diverges from the rule documented in the repository, "Content-Type is the CSRF
  check", and there is no body limit (a minor memory DoS on an admin route).
- Proposal: pass the body through `readJsonObject` (limit + the `application/json` requirement),
  like the other write routes.

### [Low] Comparison of the delete token takes data-dependent time
- File: `app/[userCode]/[mediaCode]/delete/[token]/route.ts:28`
  (`token === media.deleteToken`); likewise the matching of API tokens through an indexed
  hash lookup without constant time (`lib/api-token.ts:107-114`).
- Confidence: confirmed (the comparison is not constant-time); the practical effect is negligible.
- Code path: `deleteToken` and API tokens are high-entropy secrets (`secureToken`,
  `randomBytes`), so a network timing attack to extract a 64-character secret is
  impractical. Even so, the rest of the code (file unlock tokens, render tickets) uses
  `timingSafeEqual`, so this is an inconsistency worth aligning.
- Proposal: compare `deleteToken` with `timingSafeEqual` over buffers of equal length
  (as in `lib/password.ts:249` / `lib/media/render-ticket.ts:146`).

### [Informational] The rate-limit model depends entirely on `x-real-ip`
- File: `lib/password.ts:375-377`, `lib/auth.ts:100-104`, `docker/Caddyfile:36`,
  `docker/nginx.example.conf:25`, `docker/docker-compose.expose.yml`.
- Code path: `clientIp()` trusts only `x-real-ip`; better-auth has `ipAddressHeaders:
  ["x-real-ip"]`. That is the right choice (X-Forwarded-For can be appended to by the client). The risk
  is in the configuration: if the reverse proxy **does not overwrite** `x-real-ip` (or the application
  is exposed directly, which `docker-compose.expose.yml` warns against), every
  client sets any `x-real-ip` it likes and gets a fresh bucket on every request,
  disabling throttling of sign-in, the registration key, file password unlocking and resending
  of confirmations. It is documented extensively (`docs/deployment.md`), but it is the biggest real
  deployment foot-gun.
- Proposal: leave the code as it is; in the documentation/healthcheck consider a hard
  startup warning when `TRUST_PROXY=true` and the header does not arrive.

### [Informational] The desktop token handoff (loopback) has no PKCE
- File: `lib/authorize/loopback.ts`, `app/(auth)/authorize/page.tsx`,
  `app/actions/authorize.ts`, `components/authorize/authorize-prompt.tsx`, `next.config.ts:68-94`.
- Assessment: the implementation is correct and careful. `callback` is built solely from the
  **parsed port number** (`loopbackCallback`), never from a string supplied by the client: an open
  redirect outside loopback is impossible (only `http://127.0.0.1:<port>/`, port
  1024-65535). `name` is sanitized (control characters and bidi override are removed). The token mint is
  a Server Action (POST) that requires a session and a human click; the token's scope is `write`
  (not `full`), so the desktop cannot publish releases. The CSP for `/authorize` extends
  `connect-src` with `http://127.0.0.1:*` for that path only (the rest of the application does not).
- Residual risk: no PKCE and no server-side binding of `state` means that `state` serves
  only for the application to recognize its own reply. Intercepting the token requires a process
  listening on the victim's loopback port (local access); this is in line with RFC 8252
  and acceptable. Nothing to fix; noted because it was within the scope of the questions.

---

## Logic errors

I found no logic errors resulting in data loss, wrong ordering or wrong
conditions beyond what is listed above. Several places were checked from this angle and judged
**correct** (I note them because they were in scope):

- The order in `updateUserAccount` (`app/actions/admin.ts:331-550`): validation of everything before
  writing, the tag first (because its uniqueness is decided by the write itself), self-demotion and
  self-ban rejected: consistent, with no "half-changed account" state.
- `app/api/uploads/route.ts` `reachableBy()` (`:529-533`): the `permanent` mode deliberately sees
  rows with and without `deleted_at`; an earlier bug (permanent did not empty the trash) is described
  and fixed; `trash`/`restore` have disjoint scopes. OK.
- Rotation of the default file password (`app/actions/account.ts:464-510`): a version bump + re-encryption of
  the existing ones in one transaction; files with a manual password (`passwordVersion=null`) and without
  a password are untouched. OK.
- `isNarrowing` (`lib/api-scopes.ts:109-112`) and `narrowApiTokenScope` (`lib/api-token.ts:248-281`):
  narrowing is a subset test of capabilities (not a rank), and the old scope is pinned in the UPDATE WHERE:
  two parallel narrowings cannot interleave into a widening. OK.
- The registration gate (`lib/registration.ts:85-103`): bootstrap > rate limit > "closed" >
  key; an unreadable key = closed (fail-closed). OK.

---

## Sending data outside (my scope)

In the server/accounts/authentication/API area there is **no** code that sends data to foreign
hosts. Specifically:

- The server's only outgoing network traffic is **SMTP** through nodemailer (`lib/mail.ts`), to a
  host configured by the administrator (panel or env). Content: verification e-mails,
  password reset, invitations, "password/e-mail changed", the SMTP test. SMTP errors are redacted of
  secrets (`redactSecrets`, `smtpSecretForms`) before reaching the log/audit. The SMTP password
  never goes back to the browser (`toMailStatus`) and is stored encrypted (AES-256-GCM,
  `lib/secret-box.ts`).
- The S3 server (`lib/storage/s3.ts`) only when `STORAGE_DRIVER=s3`, to the operator's endpoint.
- `ffmpeg/ffprobe` have a protocol whitelist `file,pipe` and a forced `-f`
  (`lib/media/ffmpeg.ts`), so they will not fetch anything over the network (SSRF protection).
- Client-side fetch goes only to the application's own routes (`/api/avatar`,
  `/api/background`, `/api/uploads/check`) and to loopback (`127.0.0.1:<port>`). The only
  hard-coded external URLs are links to GitHub on the landing page and `oembed.com` (spec text).
- The account's `userEmail` (harness context) does not go into any outgoing request.

I found no exfiltration of tokens, passwords or secrets.

---

## API map (method, path, authentication / scope)

Public (no authentication):
- `GET /api/health`: a probe; it reveals only the status of the subsystems.
- `GET /api/releases/app`: the release manifest (deliberately public, cached 300 s).
- `GET /download/app`: 302 to the current release.
- `GET /avatar/{tag}`, `GET /bg/{tag}`: images (always re-encoded to WebP; `nosniff`).
- `GET /{tag}/{code}` (page), `/raw[.ext]`, `/download`, `/render`, `/timeline*`,
  `GET|POST /{tag}/{code}/delete/{token}`: the delete token **or** an API token with the scope
  `write` belonging to the owner; `/render` requires `Sec-Fetch-Dest: iframe` + a valid ticket.
- `GET /a/{tag}/{slug}` and `/a/{tag}/{slug}/zip`: only a folder that is `isPublic`; items filtered
  (published, not in the trash, without a password, not expired, within the view limit) in
  `lib/data/folders.ts:299-333`.
- `GET /u/{tag}`, `GET /s/{code}[/..]`, `GET /oembed`, `GET /embed/{v}/{card}`.
- `POST /upload`, `POST /api/upload`: an API token in the `x-api-key`/`Authorization` header or
  (legacy) the `token` field; scope **upload**.
- `/api/files*` (tus: POST/PATCH/HEAD/...): an API token (scope **upload**) **or** a session;
  subsequent PATCHes are authorized by possession of the upload URL.

API token (takes precedence) or session; required scope:
- `GET /api/me`: **identify** (any scope).
- `GET /api/folders`: read; `POST/PATCH/DELETE /api/folders`: **write**.
- `GET /api/uploads`: read; `PATCH/DELETE /api/uploads`: **write** (narrowed to the owner,
  with no admin privilege on this route).
- `POST /api/uploads/check`: **read**.
- `GET /api/duplicates`: read; `POST /api/duplicates`: **write**.
- `GET /api/shortlinks`: read; `POST /api/shortlinks`: **write** (+ rate limit).
- `GET /api/client-settings`: read; `PUT /api/client-settings`: **write**.
- `GET /api/theme`: read; `PUT /api/theme`: **write**.
- `GET /api/link-info`: **read**, but with an anonymous mode (a bad/empty token = a
  public response, not 403).

Admin (a token with scope **full** + the `admin` role, or an admin session):
- `POST /api/releases/app`: release publishing (two gates: scope `full` and the admin role).
- `GET /api/admin/disk`: the disk report (a non-admin gets a 404 byte-for-byte identical to a missing route).

Browser session only:
- `POST|DELETE /api/avatar`, `POST|DELETE /api/background`: the caller's own account.
- `ALL /api/auth/[...all]`: better-auth (sign-in, 2FA, reset, verification, the admin plugin
  protected by role).
- All server actions in `app/actions/**` (each checks `requireUser`/`requireAdmin`
  or narrows by owner in the WHERE; no dead endpoint code).
