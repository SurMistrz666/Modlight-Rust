# Server skeleton: how this crate is put together

Read `PORT.md` first; this file is the map of what already exists, how to add
to it, and where the port deliberately differs from upstream.

## 1. Run, test, lint

```sh
server/scripts/dev.sh            # start the dev DB (127.0.0.1:55421), migrate, serve on 127.0.0.1:8421
server/scripts/dev.sh migrate    # only migrate + seed
cd server
cargo test                       # unit + integration tests (no database needed)
cargo clippy --all-targets -- -D warnings
cargo fmt --check                # rustfmt.toml: max_width 120
```

`dev.sh` generates `server/docker/.env.devdb` (database password, gitignored)
on first use and never prints it. Extra variables for the server can go into
`server/.env` (gitignored, loaded like `dotenv/config` upstream; variables
already in the shell win). Logs go to stderr, filtered by `RUST_LOG`.

Database tests (`tests/db_compat.rs`) run only when a server they may create
databases on is given; each run creates and drops its own database:

```sh
set -a; source server/docker/.env.devdb; set +a
MODLIGHT_TEST_DATABASE_URL="postgresql://modlight:${POSTGRES_PASSWORD}@127.0.0.1:55421/modlight" \
  cargo test --test db_compat
```

CLI: `modlight-server [serve]` (migrate, then serve; the upstream container ran
`migrate.js` before `server.js`) and `modlight-server migrate` (`migrate.ts`:
raw `DATABASE_URL`, PG* variables when unset, no production checks).

Environment: everything `lib/config.ts` reads (see `src/config.rs`, `ENV_VARS`),
plus `BIND_ADDR` (default `127.0.0.1`; the Docker image sets `0.0.0.0` inside
the container), `MODLIGHT_ROOT` (where `migrations/`, `messages/`, `public/`
and `web/dist/` live; default: the working directory if it contains
`migrations/`, else the crate directory), `TZ` (the `timeZone` handed to the
SPA, default `UTC` like the upstream container) and `RUST_LOG`. `NODE_ENV=production`
still means production (refusals in `config.rs`, production CSP, sanitised
action errors), so upstream `.env` files keep their meaning.

## 2. Module map

| Rust | Upstream | Status |
|---|---|---|
| `main.rs` | `server.ts`, `migrate.ts` | done: CLI, runtime dirs, migrate-then-serve, jobs, graceful shutdown |
| `http.rs` | `server.ts` request handling | done: raw upload bypass, path normalisation, router assembly, hyper loop without timeouts |
| `http/proxy.rs` | `proxy.ts` | done |
| `http/headers.rs` | `next.config.ts` `headers()` | done (CSP computed at startup) |
| `http/cookies.rs`, `http/set_cookie.rs` | Next / better-call / better-auth cookie parsing and serialisation | done |
| `http/static_files.rs` | `public/`, `app/icon.svg`, `app/apple-icon.png`, SPA `assets/` | done |
| `config.rs` | `lib/config.ts` | done |
| `paths.rs` | (working directory conventions) | done |
| `error.rs` | (typed error at the edge) | done |
| `api_json.rs` | `lib/api-json.ts` + the per-route `json()` helpers | done |
| `db/mod.rs`, `db/migrate.rs` | `lib/db/index.ts`, `migrate.ts` + drizzle 0.45.2 migrator | done |
| `db/models.rs`, `db/types.rs` | `lib/db/schema/*.ts` + `drizzle/*.sql` | done (every table and column, checked against `information_schema`) |
| `rpc/` | Next server actions | done (framework) |
| `actions/tokens.rs` | `app/actions/tokens.ts` | done |
| `actions/*` (others) | `app/actions/*.ts` | to do |
| `pages/` | page data, SPA shell, metadata | done (framework + table of every page and route handler) |
| `pages/root_layout.rs` | `app/layout.tsx` | done |
| `(app)`, `(auth)` layouts and every page loader | `app/**/layout.tsx`, `app/**/page.tsx` | to do (501 until registered) |
| `i18n/` | `i18n/request.ts`, next-intl formatting | done (locale, messages, ICU subset used by the dictionaries) |
| `themes.rs`, `theme_cookies.rs` | `lib/themes.ts`, `lib/theme-cookies.ts` | done |
| `auth/cookies.rs`, `auth/session.rs` | better-auth 1.6.13 cookies + `getSession` | done |
| `auth_helpers.rs` | `lib/auth-helpers.ts` | done (functions + axum extractors) |
| `auth/api.rs`, `auth/password.rs`, `auth/two_factor.rs`, `auth/admin.rs`, `auth/email.rs` | `/api/auth/*`, better-auth plugins | layout + docs only (auth agent) |
| `api_scopes.rs`, `api_token.rs`, `api_caller.rs` | `lib/api-scopes.ts`, `lib/api-token.ts`, `lib/api-caller.ts` | done |
| `password.rs` | `lib/password.ts` | `client_ip` only |
| `ids.rs`, `audit.rs` | `lib/ids.ts`, `lib/audit.ts` | done |
| `storage/` | `lib/storage/*` | local driver done, S3 configuration only |
| `routes/api/health.rs` | `app/api/health/route.ts` | done |
| `jobs.rs` | timers in `server.ts` | done; the work functions are stubs in `upload/tus.rs`, `maintenance/trash.rs`, `mail_settings.rs` |
| `runtime/limits.rs` | `lib/runtime/limits.ts` | done |

Reference sources (read-only, never executed) of the exact library versions
upstream pins are unpacked in `_scratch/srv-skeleton/src/`: better-auth 1.6.13,
@better-auth/core 1.6.13, better-call 1.3.5, @better-auth/utils 0.4.1,
drizzle-orm 0.45.2 (integrity checked against `pnpm-lock.yaml`).

## 3. Shared vocabulary

- **Models** (`db/models.rs`): one struct per table, field names = column
  names, serde camelCase like drizzle's `$inferSelect`. Each implements
  `Columns` (`TABLE`, `COLUMNS` in database order), so queries read
  `format!("SELECT {} FROM {} WHERE ...", User::COLUMNS, User::TABLE)`.
- **Timestamps** are `JsDate` everywhere: decodes `timestamp` and
  `timestamptz` as UTC (drizzle stores UTC wall time in the Better Auth
  `timestamp` columns), binds as `timestamptz` (every connection runs with
  `TimeZone=UTC`), serialises as `toISOString()` (`...000Z`) in JSON and as
  `{"$date"}` in RPC/page payloads. `JsDate::now()` has JavaScript's
  millisecond precision.
- **Tagged JSON** (`rpc/tagged.rs`): build payloads with typed structs and
  `rpc::ok(&value)` / `PageData::new(&props, meta)`, or with
  `modlight_server::tagged_json!({...})`. Plain `serde_json::json!` serialises
  its interpolated values immediately, so a `JsDate` inside it becomes a plain
  string instead of `{"$date"}`.
- **Errors**: route handlers return `AppResult<Response>`; `AppError::message`
  gives `{"message": ...}` with a status, `AppError::Internal` (any
  `anyhow`/`sqlx` error via `?`) logs and answers 500
  `{"message":"Internal server error"}` like `server.ts` `fail()`.
- **Sessions**: `RequestSession` (one lookup per request) with the refresh
  policy of the context the upstream code ran in; see section 8.

## 4. Adding a route handler (`app/**/route.ts`)

1. Create `src/routes/<path>.rs` named after the upstream path
   (`app/api/uploads/route.ts` -> `src/routes/api/uploads.rs`,
   `app/[userCode]/[mediaCode]/[action]/route.ts` -> `src/routes/media_action.rs`).
2. Expose `pub fn routes() -> Router<AppState>` registering the upstream path
   in axum syntax with the **upstream folder names as parameter names**:
   `/{userCode}/{mediaCode}/{action}`, `/api/auth/{*all}` (axum keeps one
   name per position, and the page table already uses these names).
3. Add one line to `http::router()` in `src/http.rs` (the central list).

Until step 3, the URL answers 501 naming the upstream file (the placeholder in
`pages::fallback`), so unported handlers are visible. What the edge already
does for every handler: the `next.config.ts` headers (config values win over
the same header set by a handler, exactly like Next), `OPTIONS` -> 204 with
`Allow`, an unsupported method -> 405 without a body, HEAD from GET, panics ->
500, cookies pushed into `ResponseCookies` written as `Set-Cookie`.

Tools for the handler body:

```rust
use crate::api_json::{json_private, read_json_object, supplied, JSON_BODY_MAX};
use crate::api_caller::resolve_api_owner;
use crate::auth_helpers::{MaybeUser, RequireUser, Sessions};

async fn patch(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> AppResult<Response> {
    let (parts, body) = req.into_parts();
    let (owner, scope) = resolve_api_owner(&state, &session, &parts.headers, TokenCapability::Write)
        .await?
        .or_respond(Some(PRIVATE_NO_STORE))?;          // 401/403 refusals, upstream wording
    let input = read_json_object(&parts.headers, body, JSON_BODY_MAX).await?; // 415/413/400 like upstream
    ...
    Ok(json_private(StatusCode::OK, &result))
}
```

Session extractors (`auth_helpers.rs`) use the route-handler policy (refresh
the row AND re-issue the cookie): `Sessions` (the lazy `RequestSession`),
`MaybeUser` (`await getSession()`), `RequireUser` / `RequireAdmin` (307 to
`/login` / `/dashboard`, as `redirect()` in a route handler). Every surface that
accepts an API token names its capability (`api_token::authenticate_api_token`,
`api_caller::resolve_api_caller/resolve_api_owner`); keep the inventory of
`tests/unit/token-scope-coverage.test.ts` in mind.

The raw upload paths of `server.ts` (`/api/files*`, `POST /upload`,
`POST /api/upload`) bypass all of the above: replace the placeholder in
`http::raw_upload_router` with the tus and ShareX routers. They see the request
before normalisation, with no body limit and no timeouts.

## 5. Adding a server action (`app/actions/<module>.ts`)

1. Create `src/actions/<module>.rs` (`shortlink-unlock.ts` -> `shortlink_unlock.rs`).
2. Write each export as `async fn name(ctx: ActionCtx) -> ActionResult` and
   register it under the upstream names:
   `reg.register("shortlink-unlock", "unlockShortlink", unlock_shortlink);`
3. Add `<module>::register(reg);` to `actions::register_all`.

`ActionCtx`: `args.get::<T>(i)` (missing/`undefined` read as `null`, so
`Option<T>` is `None`; `args.raw(i)` and `args.is_undefined(i)` for exact
checks; `args.num(i)` for NaN/Infinity), `form` (for `X-Modlight-Rpc: form`
calls), `require_user()` / `require_admin()` / `get_session()`,
`set_cookie(SetCookie)` / `delete_cookie(name)` / `request_cookies()`,
`revalidate_path(..)`, `client_ip()`, `locale()`, `state`.
Return `rpc::ok(&value)`, `rpc::void()` (resolves to `undefined`),
`Err(ActionError::Redirect(url))`, `Err(ActionError::NotFound)` or
`Err(ActionError::thrown("message"))`; `?` on `anyhow`/`sqlx` errors is a
throw. Envelopes, origin check, body limit and cookies are the framework's
job (`rpc/mod.rs`); `tests/rpc_framework.rs` has a complete example module.
The upstream unit tests of an action port best against a small `*Deps` trait
the way `actions/tokens.rs` does it.

## 6. Adding a page loader (`app/**/page.tsx`, `layout.tsx`)

1. Create `src/pages/<name>.rs` (`app/(app)/dashboard/folders/page.tsx` ->
   `src/pages/dashboard_folders.rs`).
2. Write `pub async fn load(ctx: PageCtx) -> PageResult` returning
   `PageData::new(&props, meta)`: `props` is exactly the object the upstream
   server component passed to its client component(s), `meta` what its
   `metadata`/`generateMetadata` returned (`pages/meta.rs`: `PageMeta::titled`,
   `Title::Absolute`, `Field::Clear` for `null`, Open Graph, Twitter,
   alternates, `.noindex()`, raw `extra` tags).
3. Register it in `pages::register_all` by its app path from
   `pages/table.rs`: `reg.page("/dashboard/folders", dashboard_folders::load);`
   Layout loaders the same way: `reg.layout(Layout::App, layout_app::load);`

`PageCtx`: `param("userCode")`, `search.get("next")` / `search.to_value()`
(Next's `searchParams` object), `require_user()` / `require_admin()` /
`get_session()`, `cookies()`, `locale()`, `t("namespace").t("key")` /
`t_with("key", &[("count", 3.into())])` (ICU like next-intl), `client_ip()`,
`mode` (`Document` or `Navigation`), `state`. End with
`Err(PageError::redirect(url))`, `Err(PageError::NotFound)` or `?` on any error.

Layouts run first, outside in (`root`, then `(app)` or `(auth)`); a layout that
redirects stops the page. A page or layout without a loader answers 501 with
the upstream file name. The response shapes are in `pages/mod.rs` (module
docs). A page URL loaded directly returns the SPA shell (`web/dist/index.html`,
or a placeholder until the SPA is built) with the payload inlined; the SPA
fetches `GET /_page?path=<encodeURIComponent(location.pathname)>&<the page's
own query>` on client-side navigation and may send
`X-Modlight-Messages: <locale>:<messagesHash>` to skip the dictionaries.

## 7. Adding a background job

Write the work in the module that owns it (upstream path), then add one entry
to `jobs::jobs()` with its first-run delay and interval. The three upstream
timers are already scheduled: tus sweep (boot, then hourly), trash purge (60 s,
then every 6 hours), mail settings warm-up (once at boot); their work
functions are stubs to fill in: `upload::tus::sweep_tus_uploads`,
`maintenance::trash::purge_expired_trash`, `mail_settings::warm_mail_settings`.

## 8. Sessions (Better Auth compatibility)

`auth/session.rs` is Better Auth 1.6.13's `getSession` with lib/auth.ts's
options; verified against the reference server (section 10). Cookie
`better-auth.session_token` (`__Secure-` prefix when APP_URL is https), value
`encodeURIComponent(token + "." + base64(HMAC-SHA256(AUTH_SECRET, token)))`,
7-day expiry, refreshed when older than a day. Where a refresh may happen
depends on the Next context upstream ran in, so each caller picks a
`RefreshPolicy`:

| Context | Policy | Upstream reason |
|---|---|---|
| route handler, server action, `/api/auth/get-session` | `DbAndCookie` | `nextCookies()` copies the cookie into the response |
| page loaded as a document | `DbOnly` | a Server Component cannot set cookies (the write is swallowed) |
| `/_page` (client navigation) | `Skip` | `RSC: 1` requests skip the refresh |
| raw upload paths | `DbOnly` | no request scope for `cookies()` |

A session for a banned account is still returned (Better Auth deletes the
sessions when banning and refuses to create new ones:
`check_ban_before_session_create`); `resolve_api_owner` refuses banned owners
itself, as upstream. `AuthSession::output()` serialises exactly like
`auth.api.getSession()` (same fields, same order). Accounts created through
Better Auth get `theme = "modern"` written explicitly (its `additionalFields`
default), not the column default.

## 9. Deviations from upstream and from PORT.md

Deliberate, each for a reason:

1. **Malformed percent-encoding** in a path answers 400 (upstream: 500
   `text/plain`), per "malformed requests answered 400".
2. **Thrown action errors**: in production the envelope carries React's generic
   production message plus a `digest`, because that is what the upstream
   client saw (Next strips the message); with `NODE_ENV` unset the real message
   is sent. PORT.md's `{"error":{"message":"<message>"}}` shape is kept.
3. **`/_page` envelopes added** beyond PORT.md: `{"reload": true}` (the URL is a
   route handler or a static file: load it as a document), 501
   `{"notImplemented": true, "route", "file"}`, 500 `{"error": ...}`; a
   `notFound` envelope also carries `layouts.root` and `meta`; the root layout
   includes `messagesHash`, and `messages` is `null` when the SPA already has
   them (`X-Modlight-Messages`).
4. **Shortener host pass-through** also covers `/_page`, `/_rpc` and
   `/assets/` (the SPA's surface, like `/_next` upstream). A short code named
   exactly `assets/...` on the shortener host is the one thing shadowed.
5. **RPC origin check** compares the `Origin` host with `Host`
   (`X-Forwarded-Host` only when TRUST_PROXY is on), like Next's server-action
   check, which survives TLS termination; `X-Modlight-Rpc` must be `1` or
   `form`; unknown action 404; bodies over 1 MB (Next's default server-action
   limit, never raised upstream) 413.
6. **Session reads are memoised per request** (upstream re-queried; same
   answer, fewer queries).
7. **Cookie serialisation**: cookies set in route handlers and actions use
   Next's format (`Path`, `Expires`, `Max-Age`, ..., `SameSite=lax`); Better
   Auth's own endpoints should use `SetCookie::to_better_call_string`. Same
   semantics; attribute order differs between the two exactly as upstream.
8. **Security fix 12.7** is in `password::client_ip`: `X-Real-IP` only with
   TRUST_PROXY on, else the socket peer address.
9. **Head tags**: icons are linked without Next's `?icon.<hash>` cache-buster;
   the not-found `robots` tag comes after the title; Next's framework tags
   (`next-size-adjust`, preload links) are gone. Link-preview relevant tags
   (title, description, Open Graph, Twitter, canonical, oEmbed alternates) are
   rendered in Next's order; diff them against the reference once the media
   pages are ported.
10. **Static files** are served without `ETag` (with `Last-Modified`), with the
    upstream `Cache-Control` values.
11. **HTTP**: HTTP/1.1 only (as Node), no `X-Powered-By`, no Next `Vary: rsc`,
    no `Keep-Alive: timeout=5` header; no response compression yet (Next
    gzipped HTML/JSON).
12. **Jobs** never overlap a slow previous run (upstream `setInterval` could).
13. **`serve` validates the configuration before migrating** (the upstream
    container migrated first and refused to start afterwards).
14. **S3**: until the driver is ported, `/api/health` reports `storage: fail` on
    an S3 install (upstream's `stat` swallowed every error).

## 10. Verified on 2026-10-18

- Fresh dev database -> `modlight-server migrate` -> `pg_dump --schema-only`
  identical to the reference database (only pg_dump's random `\restrict` key
  differs); `drizzle.__drizzle_migrations` rows (id, hash, created_at) and the
  `settings` seed identical.
- Reference dump restored into the dev server -> the migrator applies nothing.
- A session created by signing in on the reference server (8420) is accepted
  by the Rust reader on a copy of that database (same AUTH_SECRET), its
  `getSession` output is identical field by field and in order, the cookie
  re-issued on refresh is byte-identical to the one the reference issued, and
  the three refresh policies write what the table in section 8 says. A cookie
  with a tampered signature is refused.
- `tests/db_compat.rs` checks every model against `information_schema`
  column by column.

To repeat the session check: sign in on 8420 (QA credentials in
`_wzorzec/qa-accounts.env`), `pg_dump -Fc` the reference database into a new
database on 55421, run the server against it with the reference AUTH_SECRET,
and send the cookie to `/_page?path=/login` (the root layout's `theme` comes
from the session user) or to `POST /_rpc/tokens/myTokens`.
