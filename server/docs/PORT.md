# Modlight server: Rust port, working rules

This directory holds the Rust rewrite of the Modlight server
(upstream: `../../upstream/Modlight`, Next.js 16 + PostgreSQL 17, AGPL-3.0).
The upstream checkout is the **specification**. When this document and the
upstream code disagree about behaviour, the upstream code wins, except for the
security fixes listed at the end.

## 1. Goal

- **1:1 behaviour.** Same URLs, same HTTP methods, same request and response
  bodies, same status codes, same headers that matter (cache, CSP, content type,
  cookies), same validation messages, same side effects in the database and in
  storage. A client written against the TypeScript server (the web panel,
  ShareX, Modlight App) must not be able to tell the difference.
- **Same database.** The Rust server runs on the schema the drizzle migrations
  create, reads rows written by the TypeScript server and writes rows the
  TypeScript server can read. Switching an install between the two
  implementations must work in both directions (sessions, password hashes,
  two-factor secrets, API tokens, settings, uploads).
- **Same web panel.** The browser UI keeps the upstream React components and
  translations. Only the Next.js server layer goes away: pages become a
  client-side app (`web/`, Vite + React) that loads its data from the Rust
  server, and server actions become RPC calls to the Rust server.
- Runtime is one Rust binary plus static files, `ffmpeg`/`ffprobe` and libvips.
  No Node.js at runtime (Node is a build-time tool for `web/` only).

## 2. Layout

```
server/
  Cargo.toml            single package `modlight-server` (lib + bin)
  src/                  Rust, modules mirror the upstream tree (section 5)
  migrations/           copied verbatim from upstream drizzle/ (SQL + meta/)
  messages/             copied verbatim from upstream messages/ (en, pl)
  public/               copied from upstream public/ + app/icon.svg, app/apple-icon.png
  web/                  the SPA (Vite + React + TypeScript)
  docker/               Dockerfile and compose files for the local stack
  tests/                Rust integration tests
  docs/                 this file, CONTRACT.md (RPC + page data), notes
```

## 3. Environments and ports (this Mac only, never 0.0.0.0)

| What | Address |
|---|---|
| Reference: ORIGINAL TypeScript server in Docker (`_wzorzec/`) | http://127.0.0.1:8420, its Postgres 127.0.0.1:55420 |
| Rust server, `cargo run` during development | http://127.0.0.1:8421 |
| Development Postgres for the Rust server (Docker) | 127.0.0.1:55421 |
| Rust server, final local Docker stack | http://127.0.0.1:8430 |

Docker runs in Colima, profile `modlight` (`colima start --profile modlight`).
Publish ports as `127.0.0.1:HOST:CONTAINER`, never bare `HOST:CONTAINER`.
Nothing in this project is exposed to the internet.

## 4. Stack

- tokio, axum 0.8, tower-http, hyper 1.
- sqlx 0.8 (postgres, runtime-tokio, tls-rustls, chrono, uuid, json), raw SQL.
- serde, serde_json, chrono, uuid, rand, sha2, hmac, base64, hex,
  scrypt, bcrypt, argon2 only if upstream uses it, chacha20poly1305, jsonwebtoken.
- reqwest (rustls) only where upstream makes outbound calls (S3 can use
  `aws-sdk-s3` or `rust-s3`; mail uses `lettre`).
- libvips through FFI for everything upstream does with sharp (same library,
  same results); `image` crate only where libvips is not needed.
- ffmpeg/ffprobe as child processes with argument vectors, exactly the
  arguments upstream builds.
- Errors: `anyhow` inside, a typed error at the HTTP edge.
- Formatting `cargo fmt`, lints `cargo clippy -- -D warnings` on new code.

## 5. Mapping upstream files to Rust modules

Every upstream TypeScript module that runs on the server gets one Rust module
with the same path in snake_case, so a reviewer can put the two side by side:

| Upstream | Rust |
|---|---|
| `lib/config.ts` | `src/config.rs` |
| `lib/media/av-convert.ts` | `src/media/av_convert.rs` |
| `lib/data/uploads.ts` | `src/data/uploads.rs` |
| `app/api/uploads/route.ts` | `src/routes/api/uploads.rs` |
| `app/[userCode]/[mediaCode]/[action]/route.ts` | `src/routes/media_action.rs` |
| `app/actions/uploads.ts` | `src/actions/uploads.rs` |
| page data for `app/(app)/dashboard/folders/page.tsx` | `src/pages/dashboard_folders.rs` |
| `server.ts` | `src/main.rs` + `src/http.rs` |
| `proxy.ts` | `src/http/proxy.rs` |

Keep upstream function names (snake_case) and keep the upstream comments that
explain *why* (translated only if they were not English). Port the upstream unit
tests of a module next to it (`#[cfg(test)]`) or under `tests/`.

## 6. HTTP surface

- Every route handler in `app/**/route.ts`, the two raw paths of `server.ts`
  (`/upload`, `/api/upload`, `/api/files/*` tus), and every page URL.
- `proxy.ts`: shortener-host rewrite and the optimistic `/dashboard` gate.
- `next.config.ts`: every header rule (CSP, security headers, cache) applied to
  the same paths. Next's default headers that clients may rely on
  (`x-powered-by` is NOT one: do not send it) are reviewed case by case.
- Unknown paths: the SPA shell with the not-found page and status 404, exactly
  where Next would answer 404.

## 7. Server actions become RPC

The SPA keeps the upstream components unchanged. `@/app/actions/<module>` is
aliased to a generated client stub with the same exported functions; each stub
calls:

```
POST /_rpc/<module>/<exportName>
Content-Type: application/json
X-Modlight-Rpc: 1
Origin: <must equal the request's own origin, else 403>

<tagged JSON array of the arguments>
```

`<module>` is the file name under `app/actions/` without `.ts`
(`uploads`, `shortlink-unlock`). An action whose single argument is a `FormData`
is sent as `multipart/form-data` (the form fields as-is) with
`X-Modlight-Rpc: form`.

Response: always HTTP 200 with a JSON envelope (cookies via `Set-Cookie`):

| Upstream action did | Envelope |
|---|---|
| returned a value | `{"ok": <tagged JSON>}` |
| called `redirect(url)` | `{"redirect": "<url>"}` |
| called `notFound()` | `{"notFound": true}` |
| threw | `{"error": {"message": "<message>"}}` (stub rejects with `Error(message)`) |
| called `revalidatePath`/`revalidateTag` | adds `"revalidate": true` (SPA refetches current page data) |

Tagged JSON (both directions): a `Date` is `{"$date":"2026-10-18T12:00:00.000Z"}`,
`undefined` is `{"$undefined":1}`, a `bigint` is `{"$bigint":"123"}`,
`NaN`/`Infinity` are `{"$num":"NaN"}`. Everything else is plain JSON. Field names
stay exactly as upstream returns them (camelCase).

`docs/CONTRACT.md` lists every action: module, name, arguments, return type,
redirects, revalidation, auth requirement, which upstream file implements it.

## 8. Pages become SPA routes with a page-data endpoint

Each upstream page and layout computes props on the server and renders client
components. In the SPA the same client components render with the same props,
fetched from:

```
GET /_page?path=<pathname>&<the page's own query string>
```

Response:

```json
{"route": "/dashboard/folders", "params": {}, "layouts": {"root": {...}, "(app)": {...}},
 "props": {...}, "meta": {"title": "...", "...": "..."}}
```

or `{"redirect": "/login?next=%2Fdashboard", "status": 307}`, or
`{"notFound": true}`. `props` is exactly the object the upstream server
component passed to its client components (tagged JSON). `layouts` carries the
data of every layout on the path (root: locale, messages, theme; `(app)`:
session user and navigation; `(auth)`: whatever it computes).

The first request for any page URL returns `web/dist/index.html` with
- the `<head>` the upstream `metadata`/`generateMetadata` would produce
  (title, description, Open Graph, Twitter, oEmbed discovery link, icons,
  theme colour). Link-preview bots (Discord, Telegram, Slack...) read only this.
- the same theme classes/attributes on `<html>` that the upstream root layout
  renders from cookies (no theme flash),
- the page payload inlined as
  `<script id="__MODLIGHT_PAGE__" type="application/json">...</script>`
  (escaped: `<` as `<`), so the first paint needs no second request.

## 9. Authentication (Better Auth compatibility)

Upstream uses better-auth (admin + twoFactor plugins, email+password, email
verification, cookie sessions). The Rust server must implement, with the same
wire format and the same database writes:

- the `/api/auth/*` endpoints the browser calls (`twoFactor.verifyTotp`,
  `verifyBackupCode`, `enable`, `disable`, `resetPassword`,
  `requestPasswordReset`, `getSession`, `changePassword`, the e-mail
  verification link, the reset-password link callback, sign-out),
- the server-side calls upstream makes (`getSession`, `signInEmail`,
  `sendVerificationEmail`, `listSessions`, `revokeSession`,
  `revokeOtherSessions`, `revokeUserSessions`, `createUser`, `removeUser`,
  `setRole`, `banUser`, `unbanUser`) as Rust functions with the same effects,
- password hashes (Better Auth scrypt format, plus legacy bcrypt `$2a/$2b/$2y`),
- session cookie name, signing, attributes, expiry and refresh rules,
- two-factor secret and backup-code encryption format, TOTP parameters,
- verification tokens (e-mail verification JWT, reset-password rows),
- origin checks and rate limits (window/max per path, keyed like upstream).

Read the better-auth source of the exact version pinned in
`upstream/Modlight/pnpm-lock.yaml` (download the npm tarball into
`_scratch/`, read it, never execute it).

## 10. Database and migrations

- `migrations/` is a verbatim copy of upstream `drizzle/` (SQL + `meta/_journal.json`).
- The migrator reproduces drizzle's: schema `drizzle`, table
  `__drizzle_migrations (id serial, hash text, created_at bigint)`, migration
  hash = SHA-256 hex of the SQL file, `created_at` = journal `when`, statements
  split on `--> statement-breakpoint`, all pending migrations in one transaction,
  applied when `created_at` of the newest row is lower than the journal entry.
- Then the seed from `migrate.ts` (`register_enabled` from `REGISTRATION_ENABLED`,
  `ON CONFLICT DO NOTHING`).
- A database migrated by either implementation is accepted by the other.

## 11. Translations

`messages/` is shared: the SPA bundles it, the Rust server uses it where
upstream translates on the server (mail, metadata, error pages). Locale
resolution as in `i18n/request.ts` (cookie `ml_locale`, then Accept-Language).
ICU message syntax is formatted the way `next-intl` formats it.

## 12. Security fixes (the only intended differences)

From the audit (`docs/audyt/`):

1. **High**: view limits (`maxViews`, burn after reading) are bypassable with
   `Range: bytes=1-` or `Sec-Fetch-Dest: image|video`. Every response that
   serves the full content of a file with a view limit must consume or check
   the budget. `/download` included.
2. **Medium**: `POST /upload` (ShareX) writes up to ~512 MB to disk before the
   token is validated, and chunked requests skip the free-space check.
   Authenticate before reading the body; enforce quota and free space on
   chunked bodies as they stream.
3. **Medium**: first account becomes admin with a count outside a transaction.
   Decide the role inside a transaction holding an advisory lock.
4. **Low**: `POST /api/releases/app` reads JSON without size or content-type
   checks; use the same `readJsonObject` rules as the other write endpoints.
5. **Low**: the delete token is compared with `===`; compare in constant time.
6. **Low**: `/oembed` describes a published file that has a view limit; apply
   the same rule as `lib/media/embed.ts`.
7. Rate limits key on `X-Real-IP`: honour it only when `TRUST_PROXY` is on,
   otherwise use the socket peer address.

## 13. Verifying parity

- Port upstream unit tests (`tests/unit`) for logic that now lives in Rust.
- Integration tests against a real Postgres (Docker, 127.0.0.1:55421), real
  ffmpeg and libvips.
- Run the upstream Playwright suite (`tests/e2e`) against the Rust server.
- Differential checks: the same scripted requests against 8420 (reference) and
  8421 (Rust), responses compared after normalising ids and timestamps.

## 14. Rules for everyone working here

- `upstream/` is read-only. Never run anything from it on the host.
- Work in your own git worktree and branch; commit small, English commit
  messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Code, identifiers and comments in English. User-facing text only through
  `messages/` (English and Polish).
- Never print secrets (`.env`, `AUTH_SECRET`, passwords, tokens) into output.
- Never bind anything to `0.0.0.0` on the host. Never push to any remote.
- Keep `cargo fmt`, `cargo clippy`, `cargo test` green before each commit.
