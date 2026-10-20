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
on first use and never prints it. The file lives in the MAIN checkout
(`/Users/marcinasdfg/Desktop/modlight/server/docker/.env.devdb`, found through
the shared git directory): every worktree uses the same database volume, so
they must share one password. Extra variables for the server can go into
`server/.env` (gitignored, loaded like `dotenv/config` upstream; variables
already in the shell win). Logs go to stderr, filtered by `RUST_LOG`.

Database tests (`tests/db_compat.rs`) run only when a server they may create
databases on is given; each run creates and drops its own database:

```sh
set -a; source "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/server/docker/.env.devdb"; set +a
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
| `actions/{uploads,folders,duplicates,shortlinks,shortlink_unlock,pastes}.rs` | `app/actions/` same names | done (parity: `tests/parity/library_actions.py`) |
| `actions/admin_library.rs` | `app/actions/admin.ts` (loadOrphans, deleteOrphan, purgeAllOrphans, loadDeletionImpact, deleteUserAccount, recalculateEveryQuota, runStorageScan), `components/admin/confirm.ts` | done; registered as module `admin` beside the accounts port's own `actions/admin.rs` |
| `js_value.rs` | (JavaScript operators on raw arguments) | done |
| `actions/avatar.rs` | `app/actions/avatar.ts` | done (`myAvatar` is in CONTRACT 7.3 but unreachable on the reference build: no client component imports it) |
| `actions/convert.rs` | `app/actions/convert.ts` | done (parity: `tests/parity_media/convert_actions.py`) |
| `actions/unlock.rs` | `app/actions/unlock.ts` | done (`verifyMediaPassword`; parity `media_serve.py --only gates`) |
| `pages/` | page data, SPA shell, metadata | done (framework + table of every page and route handler) |
| `pages/root_layout.rs` | `app/layout.tsx` | done |
| `pages/layout_app.rs`, `pages/layout_auth.rs`, account pages | `app/(app)/layout.tsx`, `app/(auth)/layout.tsx`, `/`, `/welcome`, `/download`, auth pages, `/dashboard/{profile,tokens,admin}` | done (accounts port; the `(app)` layout reads the library's data functions) |
| `pages/dashboard*.rs`, `pages/shortlink_unlock.rs` | `/dashboard`, `/dashboard/{folders,trash,duplicates,links,paste,upload}`, `/dashboard/admin/users/[id]/files`, `/s/[code]/unlock`, `/s/[code]/[name]/unlock` | done (parity: `tests/parity/library_pages.py`) |
| `pages/media.rs`, `pages/public_profile.rs`, `pages/public_album.rs` | `app/[userCode]/[mediaCode]/page.tsx`, `app/[userCode]/page.tsx`, `app/u/[userCode]/page.tsx`, `app/a/[userCode]/[slug]/page.tsx` | done: props, `generateMetadata` (Open Graph, Twitter, oEmbed and link-info discovery), the owner's skin and wallpaper on `<html>`, the tagless action redirect (parity `media_serve.py`) |
| `i18n/` | `i18n/request.ts`, next-intl formatting | done (locale, messages, ICU subset used by the dictionaries) |
| `themes.rs`, `theme_cookies.rs` | `lib/themes.ts`, `lib/theme-cookies.ts` | done |
| `auth/cookies.rs`, `auth/session.rs` | better-auth 1.6.13 cookies + `getSession` | done |
| `auth_helpers.rs` | `lib/auth-helpers.ts` | done (functions + axum extractors) |
| `auth/api.rs`, `auth/password.rs`, `auth/two_factor.rs`, `auth/admin.rs`, `auth/email.rs` | `/api/auth/*`, better-auth plugins | layout + docs only (auth agent) |
| `api_scopes.rs`, `api_token.rs`, `api_caller.rs` | `lib/api-scopes.ts`, `lib/api-token.ts`, `lib/api-caller.ts` | done |
| `password.rs` | `lib/password.ts` | done: `client_ip`, rate limiter, file-password hashing (accounts port); unlock tokens and cookies, owner-scoped consent (library port) |
| `ids.rs`, `audit.rs` | `lib/ids.ts`, `lib/audit.ts` | done |
| `storage/` | `lib/storage/*` | done: local driver; S3 driver on `aws-sdk-s3` with ports of lib-storage `Upload` (`s3/lib_storage.rs`) and s3-request-presigner (`s3/presigner.rs`, URLs byte-identical to the JS SDK's); `is_not_found` |
| `upload/tus_server/s3_store.rs` | `@tus/s3-store` 2.0.4 | done (`S3Store::from_env` = `makeDatastore()` for S3); keys, `.info` JSON, metadata and tags as the TS store, uploads resume across the two |
| `routes/api/health.rs` | `app/api/health/route.ts` | done |
| `jobs.rs` | timers in `server.ts` | done: tus sweep (`upload/tus.rs`), trash purge (`maintenance/trash.rs`), mail settings (accounts port) |
| `upload/sharex.rs`, `upload/busboy.rs` | `lib/upload/sharex.ts` + busboy 1.6's part rules | done (security fix 2; parity `library.py --only sharex`) |
| `upload/tus.rs`, `upload/tus_server/{server,store,file_store}.rs` | `lib/upload/tus.ts`, `@tus/server` 2.4.1, `@tus/utils` 0.7.0, `@tus/file-store` 2.1.0 | done (records interchangeable with the TS store; tus-js-client 4.3.1 resumes across both; `tests/parity/tus_client.cjs`) |
| `upload/{finalize,disk,folder_target,duplicate_lookup,duplicate_check}.rs` | `lib/upload/*` | done |
| `media/{sniff,mime,urls,paths,paste_type,actions,derived_cache,convert_targets,av_targets}.rs` | `lib/media/*` same names | done (`derived_cache`: keys, purge, `mayCacheDerived`; reading and writing cached conversions belongs to the media serving port) |
| `media/serve.rs` | `lib/media/serve.ts` | done: `getMedia` (+ `lookup_media`, which tells a spent budget apart), `viewerRelation`, `consumeView`, `cacheHeadersFor`, `parseRange`, `streamMedia`; security fix 1 (the view grant, `ViewBudget`) |
| `media/{bots,render_ticket,embed,link_formats}.rs` | `lib/media/` same names | done (media serving port) |
| `media/{convert,convert_plan,convert_size,convert_verdict,convert_notes,convert_in_place,download_menu,av_convert,av_action,motion,motion_preview,timeline,timeline_preview,poster,frame_step,source_preview,card,card_art,card_render,player_i18n,playback_health}.rs`, `media/pdf_lib.rs` | `lib/media/` same names, the part of pdf-lib 1.17.1 `convert.ts` uses | done (media processing port: section 12) |
| `media/part_a.rs` | the processing half of `lib/media` as the serving half calls it | the serving port's adapter, wired to the processing modules (section 12) |
| `crypto/envelope.rs` | `lib/crypto/envelope.ts` | server part: constants, `isEncryptedUpload` (the rest runs in the browser) |
| `routes/media_action.rs` | `app/[userCode]/[mediaCode]/[action]/route.ts` | done: `raw`, `raw.<ext>`, `download`, `render`, `timeline.vtt/jpg`, `convert` (the engines through `part_a`) |
| `routes/{media_delete,album_zip,embed_card,oembed}.rs`, `archiver.rs` | `app/[userCode]/[mediaCode]/delete/[token]`, `app/a/[userCode]/[slug]/zip`, `app/embed/[version]/[card]`, `app/oembed`; archiver 8 / zip-stream 7 / compress-commons 7 | done (security fixes 5 and 6; the ZIP writer is archiver's zip layout for stream entries) |
| `http/web_response.rs` | Fetch `new Response(text)` defaults | done (`text/plain;charset=UTF-8`, headers object semantics) |
| `media/vips.rs` + `media/vips/*`, `media/{image,probe,avif_brand,blurhash,sharp_runtime,transform_cost,transform_queue,avatar,abort}.rs` | sharp 0.34.5 / libvips 8.17, `lib/media/{image,probe,avif-brand,sharp-runtime,transform-cost,transform-queue,avatar}.ts`, blurhash 2.0.5 | done (section 11) |
| `media/{ffmpeg,av_probe,av_cost,av_workspace,av_meta}.rs` | `lib/media/*` same names | done |
| `media/{docx,docx_support,zip_guard}.rs`, `media/mammoth/` | `lib/media/{docx,docx-support,zip-guard}.ts`, mammoth 1.12.1 (+ JSZip 3.10.1, @xmldom/xmldom 0.8.13, lop 0.4.2, dingbat-to-unicode 1.0.1) | done (section 13) |
| `naming.rs`, `naming_words.rs`, `folders.rs`, `uploads.rs`, `shortlinks.rs`, `quota.rs`, `format.rs`, `avatar.rs`, `avatar_store.rs`, `background.rs`, `paste_languages.rs`, `js_date.rs`, `link_info/parse.rs`, `maintenance/trash.rs` | `lib/` same names (`js_date.rs`: V8's `new Date(string)`) | done |
| `data/{uploads,upload_query,folders,duplicates,admin_files}.rs` | `lib/data/` same names | done |
| `routes/api/{uploads,uploads_check,folders,duplicates,shortlinks,link_info,avatar,background,admin_disk}.rs`, `routes/{avatar,s_code,s_code_name}.rs` | `app/api/{uploads,uploads/check,folders,duplicates,shortlinks,link-info,avatar,background,admin/disk}`, `app/avatar/[userCode]`, `app/bg/[userCode]`, `app/s/[code]`, `app/s/[code]/[name]` | done (parity: `library.py`, `library_routes.py`) |
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
14. **S3 transport**: the Rust SDK gives up connecting after 3.1 s and fails a
    transfer the server stalls (no bytes for 5 s while it is being read; a
    reader that pauses is not a stall), where Node would wait. `stat` still
    swallows every error like upstream, so `/api/health` reports `storage: ok`
    on an S3 install whose endpoint is down, as upstream did. The runtime image
    needs a CA bundle for HTTPS endpoints (rustls reads the system store).
15. **`pastes.createPaste` languages**: `LANG_EXT[lang]` upstream also finds
    what every JavaScript object inherits, so a `language` of `constructor` or
    `__proto__` (the lower-cased names among them) took a function or an
    object as the extension, wrote the bytes under `<code>.function Object()
    { [native code] }` / `<code>.[object Object]` and then threw (verified on
    a private reference). Here only the 28 listed languages map to an
    extension; any other name is `txt`, and the paste is created.
16. **`uploads.emptyTrash` with a storage driver that cannot be built** (an S3
    install missing a variable) throws even for an empty trash; upstream's
    `getStorage()` only ran once a batch had rows. Either way the install is
    broken; the answer differs only for an empty trash.
17. **ShareX (security fix 2)**: the header token is authenticated before a
    byte of the body is read; a request that names no header token may still
    carry a legacy `token` field, but its body is capped at 8 MiB
    (`LEGACY_UPLOAD_MAX_SIZE` overrides; 413 with the upstream wording); quota
    and free space are re-checked every 8 MiB while a chunked body streams
    (507 "User disk quota exceeded." / the low-disk message), instead of after
    the whole file landed on disk.
18. **tus wire details**: the status line carries a reason phrase, empty
    bodies go out with `Content-Length: 0` (Node chunked them), and there is no
    `Connection: keep-alive` header. Kept from upstream: an upload finished at
    creation (zero length, or creation-with-upload of the whole body) is
    finalized, and its URL answers 404 afterwards.
19. **`DELETE /api/uploads`** reads at most 1 MiB of body (upstream:
    `req.json()` without a limit); 500 ids need about 20 KB.
20. **libvips**: only the loaders sharp-libvips 1.2.4 ships may run (Homebrew's
    ImageMagick loader would accept BMP and the `avis` AVIF production
    refuses); HEVC pixels are refused (`HEVC_DECODE = false`) as production
    cannot decode them. Built against libvips 8.18, image metadata, thumbnails
    and avatars are byte-identical to production's 8.17.3 except CMYK input
    (mean 3/255 colour shift); on Debian 13's 8.16.1, 2 of 20 blurhashes differ
    by a digit. The runtime image should carry libvips 8.17 or newer.
21. **ffprobe**: durations come from whatever ffprobe the host has. The
    reference image (Debian 12, ffmpeg 5.1) counts an MP3's encoder padding
    (1.541224 s) where ffmpeg 9 does not (1.5 s); ship the reference's ffmpeg
    major for identical numbers.
22. **`GET /api/link-info`** sends the handler's own `Vary: Authorization,
    Cookie` (Next replaces it with its `rsc, ...` list on every route handler;
    see 11).
23. **Kept on purpose (upstream behaviour worth knowing)**: replacing or
    removing an avatar or a wallpaper never deletes the previous object
    (`UPDATE ... RETURNING` reads the NEW key, so `discard(old, key)` is a
    no-op; confirmed on the reference); on S3 a finished tus upload stays
    downloadable at its tus URL (`GET /api/files/<id>`), whatever the file's
    later privacy or view limit, until the file is deleted; `listFolders`
    always reports `count: 0` (the subquery compares `uploads` with itself).
24. **A thrown error in a route handler** answers 500 with
    `{"message":"Internal server error"}` (section 3); Next sends an empty
    500. Seen on `/api/duplicates` with an out-of-range count and on `/s/%00`.
25. **`read_json_object`** replaces lone surrogate escapes with U+FFFD before
    parsing (JSON.parse accepted them); a number beyond the f64 range
    (`1e400`) or nesting deeper than 128 levels is still refused with 400
    where upstream judged the fields.
26. **Security fix 1, view limits** (`media::serve::ViewBudget`, the exact
    rule is in its doc comment). For a file with `max_views`, and anybody but
    its owner or an administrator: a request upstream counted is counted
    (unchanged); a request upstream exempted (a `Range` not starting at 0,
    `Sec-Fetch-Dest: image|video`, `w`/`h`/`motion`, a bot) is served without
    consuming only with the VIEW GRANT of a counted view, else it consumes one.
    The grant is a signed cookie `ml_view_<uploadId>` = `<exp>.<HMAC(AUTH_SECRET,
    "view|<id>|<exp>")>` (6 hours, httpOnly, `SameSite=Lax`, `Secure` in
    production), minted by every counted view of a capped file: the preview
    page's (document and `/_page`), and `/raw`/`/download`'s. It also lets
    its holder through when that view was the LAST one (`views == max_views`),
    which repairs two upstream failures: the page's own `<img>`/`<video>` on the
    last permitted view, and an encrypted one-time file whose second ranged
    read 404ed (verified on the reference). It applies to everything that
    carries the file's content: the stored bytes (whole or ranged) and pictures
    made of them (resized stills, moving tiles, timeline sheets); refusals and
    the cue track spend nothing. Also: a preview bot on the page of a capped
    file is not counted (as upstream) and gets no content (no inlined text, no
    render ticket), the card of a capped text file carries no excerpt, and a
    capped item travels in an album ZIP only by spending a view (as a download).
    Costs: a cross-host page (owner's preferred domain different from the host
    the page was opened on) cannot carry the cookie, so its `<img>` pays a view;
    public gallery tiles of a capped file pay a view each; resuming a capped
    download with `Range` pays a view.
27. **Security fix 5**: the delete-by-link token is compared in constant time.
28. **Security fix 6**: `/oembed` describes a published file with a view cap as
    `{version, title, provider_name, provider_url, cache_age, type: "link"}`:
    no raw URL, no dimensions, no player HTML (as `embed.ts` withholds
    `og:image`/`og:video`).
29. **Album ZIP**: the reference answers every `/a/<tag>/<slug>/zip` with 500
    (`import archiver from "archiver"` against archiver 8, which has only
    named exports: `TypeError: (0, a.default) is not a function`). The port
    serves the archive the route was written for, in archiver's zip layout for
    stream entries (`src/archiver.rs`); deflate bytes come from `flate2`, not
    Node's zlib, so archives are equivalent (same entries, order, names,
    flags, attributes, CRCs and contents), not byte-identical.
30. **Metadata placement**: Next 16 streams a slow `generateMetadata` into the
    BODY for browsers (it blocks the head only for the bots it recognises);
    the port always puts it in `<head>`. Same tags, same order.
31. **Plain-object lookups**: `RENDERABLE`, `EXT_CATEGORY` and `LONG_TYPES`
    match only their listed keys (upstream found `constructor` and friends on
    the prototype, so a stored type `constructor` was offered the render toggle
    and an extension `constructor` a card URL spelled from a function).
32. **Derived responses** (thumbnails, motion tiles) carry `Content-Length`;
    Next sent them chunked. Same bytes.

33. **ffmpeg version (media processing)**: the argument vectors are upstream's
    and identical to the reference's (verified, `tests/parity_media`), but
    they were written for ffmpeg 5.1 (Debian 12, the production image). On
    ffmpeg 7+ (Homebrew's 9 on the Mac): `-fflags +shortest` is no longer a
    format flag, so audio -> MP4 fails; the mjpeg encoder refuses the
    limited-range YUV the timeline sheet hands it, so `timeline.jpg` fails; a
    clip offset past the end (`?t=9999`) fails writing the GIF trailer where
    5.1 wrote a one-frame GIF; ffprobe measures VFR files, AVIF sequences and
    MP3 padding differently, so a few plans differ (item 21). Homebrew's
    build also has no libvorbis and no libwebp (`ogg`, and ffmpeg's animated
    WebP of an AVIF sequence, fail). Run the server with the reference's
    ffmpeg major (the Docker image on Debian 12) for identical behaviour; with
    it, every status, header, argument vector and deterministic output matched.
34. **Text on the embed card**: libvips' Pango is put on its fontconfig
    backend (`PANGOCAIRO_BACKEND=fc`, set once at libvips start unless the
    operator set it). It is the only backend Linux has; on macOS Pango would
    otherwise pick CoreText, which never sees the `fontfile` and draws a
    system face. With it every card is pixel-identical to the reference.
35. **PDF output** (`media/pdf_lib.rs`): pdf-lib's file structure object for
    object; the Flate streams are compressed by miniz rather than pako (they
    inflate to the same bytes). Like pdf-lib, the info dates and the XObject
    name (`Image-` + random) change on every call, so no two PDFs are equal
    byte for byte on either server.
36. **`HEVC_DECODE = false`**, decided exactly as upstream decides it: by
    the libheif sharp-libvips 1.2.4 ships (1.20.2 with aom only, no libde265,
    checked in the reference container: `metadata()` reads a HEIC's header
    as `compression: hevc`, `toBuffer()` fails with "Support for this
    compression format has not been built in"). So HEIC gets no blurhash, no
    thumbnail of its own, no conversion target (convert-targets offers none)
    and no motion tile (`not-animated`), on both servers.
37. **Lookups by a caller's string** in `media/source_preview.rs` (paste
    language -> extension) use exact keys, as item 31 does for the card table.

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

## 11. The libvips binding (`src/media/vips.rs`)

Upstream does all image work with sharp 0.34.5, a thin C++ layer over libvips
(8.17.3 in production). Calling libvips with the same operations and
arguments gives the same pixels, so the port binds libvips directly:
hand-written `extern "C"` declarations in `vips/ffi.rs` (no bindgen, no extra
crates); `build.rs` links what `pkg-config --libs vips` names (libvips 8.15 or
newer; `brew install vips` on macOS, `libvips-dev` in the image).

Layers: `ffi.rs` (raw declarations, `GValue`, `VipsProgress`) ->
`operation.rs` (`Operation`: any libvips operation by nickname, like vips8's
`VImage::call`) -> `image.rs` (`Image`: one owned reference, `Blob`, typed
wrappers, the timeout) -> `sharp.rs` (`Sharp`: sharp's `pipeline.cc` and
`metadata()` for the options the server uses).

Rules:

1. `init()` runs `vips_init` once with sharp-runtime's settings (threads from
   `effective_cpu_count()`, cache 32 MB / 10 files / 50 operations).
2. Everything is blocking: use the async methods or `run_blocking` (tokio's
   blocking pool, then `vips_thread_shutdown`). Expensive work goes through
   `transform_queue::image_transforms()` like upstream's `imageTransforms`.
3. `Image` holds one GObject reference, `Send` but not `Sync`; metadata is
   written only through `Image::mutate` on an uncached private copy.
4. Inputs are copied into `vips_tracked_malloc` memory so the 32 MB cache
   ceiling bounds what cached operations can pin.
5. Errors take and clear the process-wide error buffer (concurrent failures
   can mix messages); VIPS warnings go to tracing at debug level.
6. Loaders: JPEG, PNG, WebP, GIF, TIFF, HEIF/AVIF and SVG only (section 9,
   item 20); loads are sequential, as in sharp 0.34.5.
7. `Image::set_timeout` is sharp's `.timeout()` (an `eval` handler that kills
   the evaluation); encoders that never report progress run to the end.
8. Argument names and types are checked against the operation's own
   declarations before anything reaches GObject.

Adding an operation needs no new FFI: `Operation::new("flatten")?.set("in",
self).set("background", background).call()?.image("out")`. Scalars are
`i32`/`f64`/`bool`/`Arg::Str`, enums and flags `Arg::Nick`; arguments only
newer libvips releases have go through `set_if_known` with a comment naming the
release; encoders are `*save_buffer` operations passed to `Image::save_buffer`.
Copy the order and arguments from sharp's `pipeline.cc` (sources in
`_scratch/srv-library/ref-src/sharp@0.34.5`). The media processing port
added `flatten`, `join: { animated }`, `composite` and text input (section 12).

## 12. Media processing API (for the media serving port)

What the serving routes (`/[userCode]/[mediaCode]/[action]`: `convert`,
`raw?motion=1`, `raw?w=` for a video, `timeline.vtt`, `timeline.jpg`; the
`render` of a .docx; `/embed/[version]/[card]`) and the media page call, by
upstream name. `MediaRow` is `db::models::Upload` (with `rowMimeType`
applied, as `get_media` returns it). Engine errors that upstream let escape
(`BusyError`, `TimeoutError`, `AbortedError` from the limiter;
`AvTimeoutError`, `AvAbortedError` from ffmpeg/ffprobe) come back as
`anyhow::Error`s to `downcast_ref` - the route's `transformRefusal`.

| Upstream | Rust | Notes |
|---|---|---|
| `avConvert(req)` (av-action.ts) | `media::av_action::av_convert(&AppState, AvConvertRequest) -> anyhow::Result<Option<Response>>` | `None` = not audio/video, take the image branch. `AvConvertRequest { media, privileged, to: Option<String>, offset_seconds (?t=), clip_seconds (?d=), signal, admit: Option<AdmitHook> (consumeView, download: true), dry_run (?check=1) }`. Builds the whole response (refusals with `X-Convert-Refused`/`limit`, 204 with `X-Convert-Note`, streamed download); runs inside the limiter itself. |
| `convertOptionFor`, `conversionBudget`, `probeImage`, `convertImageBuffer` (convert.ts) | `media::convert::{convert_option_for, conversion_budget_for(&ImageProbe, ConvertTarget), conversion_budget(animated, frame_pixels, total_pixels, to), probe_image(Bytes), convert_image_buffer(Bytes, ConvertTarget, ConvertOptions { animated, timeout_ms, signal }) -> anyhow::Result<Converted { buffer, mime, ext }>}` | the image branch of /convert; the `?check=1` row shortcut is `conversion_budget(frames > 1, w*h, w*h*frames, to)`. |
| `derivedKey`, `contentHash`, `mayCacheDerived`, `readDerived`, `writeDerived`, `readDerivedStream`, `writeDerivedFile` | `media::derived_cache::*` (`read_derived(&Storage, key) -> Option<Bytes>`, `write_derived(&Storage, key, Bytes)`, ...) | best effort, as upstream. |
| `motionPreview(media, opts, signal)` (motion.ts) | `media::motion::motion_preview(&AppState, &Upload, MotionOptions { width, height, fit: MotionFit }, AbortSignal) -> anyhow::Result<Result<(Bytes, u32 frames), MotionRefusal>>` | run it inside `image_transforms().run(.., MOTION_WEIGHT, MOTION_TIMEOUT_MS)`; `X-Motion-Frames` is the `u32`; refusal tokens `MotionRefusal::as_str()`. Query parsing: `motion_preview::{wants_motion, MOTION_DEFAULT_SIZE, clamp_motion_edge}`. |
| `timelineTrack`, `timelineSheet` (timeline.ts) | `media::timeline::{timeline_track, timeline_sheet}(&AppState, &Upload, AbortSignal) -> anyhow::Result<Result<String / Bytes, TimelineRefusal>>` | inside the limiter with `TIMELINE_TRACK_*` / `TIMELINE_*`; actions `timeline_preview::{TIMELINE_TRACK_ACTION, TIMELINE_SHEET_ACTION}`. |
| `videoPoster(media, signal)` (poster.ts) | `media::poster::video_poster(&AppState, &Upload, AbortSignal) -> Option<Bytes>` | `None` = no poster (the route answers 415, never the video). `open_video_source` is shared with the timeline. |
| `docxToHtml(buf)`, `MAX_DOCX_BYTES`, `docxSupport` (docx.ts) | `media::docx::docx_to_html(Bytes) -> DocxResult`, `media::docx_support::{MAX_DOCX_BYTES, docx_support}` | section 13; run it inside the limiter at thumbnail weight with `DOCX_TIMEOUT_MS`, as the route does. |
| `renderCard(choice)`, `parseCardFile`, `cardChoiceFor`, `cardUrl`, `EMBED_CARD_VERSION` (card-render.ts, card.ts) | `media::card_render::render_card(&CardChoice) -> Result<Bytes, String>`, `media::card::{parse_card_file, card_choice_for, card_path, card_url, EMBED_CARD_VERSION, CARD_WIDTH, CARD_HEIGHT}` | `render_card` caches every pair it drew for the life of the process (shared futures, failures not cached). |
| `diagnosePlayback(media)` (playback-health.ts) | `media::playback_health::diagnose_playback(&PlaybackSource) -> Option<PlaybackDiagnosis>` (`PlaybackSource::from(&Upload)`) | serialises like upstream's object (`playback` prop of the media page). |
| `showsSource`, `inlineTextPlan`, `sourceReadings`, `codeLanguageLabel`, `MAX_INLINE_TEXT` (source-preview.ts) | `media::source_preview::*` | |
| `downloadOptionsFor`, `downloadHintKey`, `downloadLabelKey`, `sectionHeadingKey` (download-menu.ts) | `media::download_menu::*` (`DownloadOption` serialises as upstream's literals) | the media page's menu (`canEncode` = owner or admin). |
| `estimateTargetSize`, `formatEstimate`, ... (convert-size.ts) | `media::convert_size::*` | |
| `playerLang`, `buildPlayerDictionary` (player-i18n.ts) | `media::player_i18n::*` | |
| `readVerdict`/`rememberVerdict`/`verdictKey` (convert-verdict.ts), `noteKey`/`REFUSAL_KEYS` (convert-notes.ts) | `media::convert_verdict::*`, `media::convert_notes::*` | process-wide memo, as upstream. |
| `REASON_TEXT`, `refusal`, `verdictResponse` (av-action.ts) | `media::av_action::{REASON_TEXT, refusal, verdict_response, base_name, source_tag, source_kind_of}` | `base_name` is the download name rule (`filename.replace(/\.[^.]+$/, "") \|\| code`). |

Also ported and used inside: `av_convert` (`planAv`, `runAvPlan`, `avVariant`,
`frameOffset`, `animationFilters`, `webScaleFilter`, `audioCoverPath`),
`convert_plan` and `convert_in_place` (`convertUploadInPlace` behind the
`convert` RPC module), `frame_step` (the player's arithmetic, server twin),
`pdf_lib` (section 9, item 35).

Verification (re-runnable, see each script's header):

```sh
S=_scratch/srv-media-a      # scratch reference on 8446, Rust server on 8426
python3 server/tests/parity_media/corpus.py $S/corpus
python3 server/tests/parity_media/media_parity.py ref              # 595 cases on the reference
python3 server/tests/parity_media/media_parity.py rust --ffmpeg ref # the reference image's ffmpeg 5.1
python3 server/tests/parity_media/media_parity.py compare --ffmpeg ref
python3 server/tests/parity_media/media_parity.py cards            # all 155 embed cards
python3 server/tests/parity_media/convert_actions.py               # the convert RPC module, 20 checks
```

Through the routes: part B's `tests/parity/media_serve.py` on the merged
binary, 1113/1113 (with the reference image's ffmpeg; encoder-dependent
bytes compared by decoded content).

Measured on 2026-10-20 (libvips 8.18.7 here, 8.17.3 in the reference):
statuses, meaningful headers and ffmpeg/ffprobe argument vectors identical
in all 595 cases; outputs byte-identical for every JPEG, WebP (still and
animated, including every motion tile), GIF, MP4/MOV remux and encode, MP3,
WAV, poster and timeline sheet and track; PNG and AVIF differ in their bytes
only (encoder builds; same pixels), Matroska and Ogg differ by their random
segment/serial ids (same streams), PDFs as item 35; CMYK input shifts colour
by about 3/255 (section 11). All 155 embed cards are pixel-identical. With
Homebrew's ffmpeg 9 instead, see item 33.

## 13. Document preview (docx)

`media::docx::docx_to_html(Bytes) -> DocxResult` is `docxToHtml`
(`Result<DocxPreview { html, images, images_dropped }, DocxRefusal>`,
`DocxRefusal::as_str()` gives upstream's token: `not-a-zip`, `malformed`,
`too-big`, `too-many-entries`, `zip64`, `unsupported-method`, `too-large`,
`legacy-doc`, `not-a-document`); `media::zip_guard::inspect_zip` and
`media::docx_support::{docx_support, MAX_DOCX_BYTES}` are the two helpers.
The route (`/{tag}/{code}/render`, gating, status codes) is not part of it.

`media/mammoth/` is mammoth 1.12.1 ported file by file (`docx/body_reader.rs`
= `lib/docx/body-reader.js`, ...), with what it runs of JSZip
(`jszip.rs`: central directory, local names, size check, no CRC check),
@xmldom/xmldom (`xml/xmldom.rs`: sax.js and the DOM rules, every
warning/error is a failure because mammoth throws on them), lop
(`lop.rs`, the style-map grammar's combinators) and dingbat-to-unicode.
`mammoth/js.rs` holds the JavaScript semantics it needs (`parseInt`,
`ToNumber`, `{}` as a map with `Object.prototype` showing through, UTF-16
strings). The conversion runs on a thread of its own (256 MB stack, the
readers recurse); pictures go through the libvips binding one at a time in
document order (`mapSeries` upstream), so the 3 MB budget drops the same ones.
New dependency: `flate2` on `zlib-rs` (zlib's inflate semantics, as node:zlib
and pako).

Verified by `tests/parity_media/`: `docx_corpus.py` builds 229 documents
(hand-built OOXML per mammoth feature, embedded style maps, pictures, hostile
archives, xmldom edge cases, `Object.prototype` names, macOS textutil output,
mammoth's own Word/LibreOffice test documents); `docx_parity.py` runs each
through upstream's own `docx.ts`, `zip-guard.ts`, `docx-support.ts`,
`sharp-runtime.ts` and `limits.ts` inside the reference container (node 24
strips the types; imports rewritten to the container's packages) and through
`docx_to_html` (`tests/media_docx.rs`, ignored test `parity_dump`), and
compares the JSON. On 2026-10-20: 222 identical byte for byte, pictures
included (libvips 8.18 here and 8.17.3 in the reference wrote the same
WebP); 3 known differences (items 3 and 4 below); 4 that never finish
upstream (item 1). `docx_route_e2e.py` fetches a sample through the
reference's real `/render` route with minted tickets: same bytes as the
harness.

To repeat (from the main checkout; the reference app runs on 8446, see
`_scratch/srv-media-a/`):

```sh
python3 server/tests/parity_media/docx_corpus.py _scratch/srv-media-a/docx/corpus
python3 server/tests/parity_media/docx_parity.py --corpus _scratch/srv-media-a/docx/corpus \
  --work _scratch/srv-media-a/docx/run --container modlight-srvmedia-a-ref-app-1
python3 server/tests/parity_media/docx_route_e2e.py --corpus _scratch/srv-media-a/docx/corpus \
  --work _scratch/srv-media-a/docx/run f-basic.docx i-budget.docx h-bomb-lying.docx
```

Differences, all deliberate or unavoidable:

1. **Hangs.** Upstream never finishes on some inputs the guard lets
   through; the port answers. JSZip loops forever on ZIP64 "extensible data"
   whose length walks the reader backwards (`not-a-document` here); xmldom's
   error recovery is quadratic (the port stops at the first report, which is
   the same answer); some error-free scans are quadratic (many unterminated
   `<?` or CDATA sections, many distinct element names: memoised here, same
   tree).
2. **Entries the guard never inflated** (a central record beyond the
   declared count, or the directory JSZip finds after its prepended-data
   shift) are expanded with a ceiling of 40 MiB + 1 (JSZip: none) and fail
   past it.
3. **Prototype pollution not reproduced.** A style with `w:type="__proto__"`
   writes itself onto `Object.prototype` in upstream (every object in the
   Node process gains a property, for the rest of its life: e.g. a style id
   `w:type` removes every `<w:br/>`). Here such a style is ignored. An object
   assigned to a `__proto__` key of mammoth's maps does not become the
   prototype of the other keys (inherited `Object.prototype` members are
   reproduced: `constructor`, `toString`, ... as ids, element names,
   relationship types).
4. **Nesting depth.** Upstream runs out of V8 stack at a depth that depends
   on what nests and on JIT warm-up (fresh process: 1898 nested elements in a
   paragraph, 909 nested `w:sdt`, 306 nested tables; warm: 2499, 1249, 312).
   The port fails an XML part nested deeper than 1901 elements (the fresh
   limit for plain nesting); deeper `w:sdt`/table nesting below that renders
   here and fails upstream.
5. **Lone UTF-16 surrogates** (from character references) are carried as
   U+10F800..U+10FFFF inside the port and resolved at the end exactly as the
   JavaScript string would be encoded; a document containing those private-use
   code points literally would see them treated as surrogates.
