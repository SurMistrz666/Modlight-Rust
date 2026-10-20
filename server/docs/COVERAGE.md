# Coverage: every upstream server-side piece and its Rust counterpart

Upstream: `upstream/Modlight` (Next.js 16), read 2026-10-20. Rust: `server/src`.
"Parity" names the differential suites (`tests/parity/*.py`,
`tests/parity_media/*.py`, run against the reference image; results in
`docs/handoff/srv-integration.md`), "e2e" upstream's Playwright specs
(`tests/e2e`), "test" the Rust tests. Deliberate differences are SKELETON.md
section 9 (item numbers given as 9.N).

Completeness is also a test: `tests/coverage.rs` fails when any of the 27
pages or 3 layouts has no loader, or when any of the 25 route handlers answers
the 501 "not ported" placeholder. No page and no handler answers 501.

## 1. Route handlers (`app/**/route.ts`, 25)

| Upstream | Rust | Parity / tests |
|---|---|---|
| `app/api/auth/[...all]/route.ts` (Better Auth 1.6.13) | `auth/api.rs`, `auth/routes/*`, `auth/two_factor.rs`, `auth/admin.rs` | `auth.py`, `crosscompat.py` (both directions on one database), e2e |
| `app/api/health/route.ts` | `routes/api/health.rs` | `api_misc.py`, compose healthcheck (`modlight-server health`) |
| `app/api/me/route.ts` | `routes/api/me.rs` | `api_misc.py` |
| `app/api/theme/route.ts` | `routes/api/theme.rs` | `api_misc.py` |
| `app/api/client-settings/route.ts` | `routes/api/client_settings.rs` | `api_misc.py` |
| `app/api/releases/app/route.ts` | `routes/api/releases_app.rs` | `releases.py`, `tests/releases.rs` (fix 4) |
| `app/download/app/route.ts` | `routes/download_app.rs` | `releases.py` |
| `app/api/uploads/route.ts` | `routes/api/uploads.rs` | `library.py` (uploads_api), `tests/library_routes_db.rs` |
| `app/api/uploads/check/route.ts` | `routes/api/uploads_check.rs` | `library_routes.py` |
| `app/api/folders/route.ts` | `routes/api/folders.rs` | `library_routes.py` |
| `app/api/duplicates/route.ts` | `routes/api/duplicates.rs` | `library_routes.py` |
| `app/api/shortlinks/route.ts` | `routes/api/shortlinks.rs` | `library_routes.py` |
| `app/api/link-info/route.ts` | `routes/api/link_info.rs` | `library.py` (link_info) |
| `app/api/avatar/route.ts` | `routes/api/avatar.rs` + `avatar_store.rs` | `library.py` (avatars), `tests/library_routes_db.rs` (9.39) |
| `app/api/background/route.ts` | `routes/api/background.rs` | `library.py` (avatars), `tests/library_routes_db.rs` (9.39) |
| `app/api/admin/disk/route.ts` | `routes/api/admin_disk.rs` | `library_routes.py` (admin_disk) |
| `app/avatar/[userCode]/route.ts`, `app/bg/[userCode]/route.ts` | `routes/avatar.rs` | `library.py` (avatars) |
| `app/s/[code]/route.ts`, `app/s/[code]/[name]/route.ts` | `routes/s_code.rs`, `routes/s_code_name.rs` | `library_routes.py` (redirects) |
| `app/[userCode]/[mediaCode]/[action]/route.ts` (raw, raw.ext, download, render, timeline, convert) | `routes/media_action.rs` + `media/*` | `media_serve.py`, `media_parity.py`, `docx_parity.py`, `tests/media_serving_db.rs` (fix 1) |
| `app/[userCode]/[mediaCode]/delete/[token]/route.ts` | `routes/media_delete.rs` | `media_serve.py` (delete, fix 5), e2e (ShareX contract) |
| `app/a/[userCode]/[slug]/zip/route.ts` | `routes/album_zip.rs` + `archiver.rs` | `media_serve.py` (zip; 9.29: the reference always answers 500) |
| `app/embed/[version]/[card]/route.ts` | `routes/embed_card.rs` + `media/card_render.rs` | `media_serve.py` (embed), `media_parity.py cards` (155/155 byte-identical) |
| `app/oembed/route.ts` | `routes/oembed.rs` | `media_serve.py` (oembed, fix 6) |

Metadata routes `app/icon.svg`, `app/apple-icon.png`: static files
(`http/static_files.rs`, `public/`), the upstream cache headers.

## 2. `server.ts`, `proxy.ts`, `next.config.ts`, `migrate.ts`

| Upstream | Rust | Parity / tests |
|---|---|---|
| `server.ts`: raw paths `/api/files*` (tus 2.4.1, file and S3 stores) | `upload/tus.rs`, `upload/tus_server/*` | `library.py` (tus), `tus_client.cjs` via `_scratch/.../tus_parity.py`, `tests/tus_s3_store.rs`, `tests/library_s3_e2e.rs`, unit test of 9.38 |
| `server.ts`: `POST /upload`, `POST /api/upload` (ShareX) | `upload/sharex.rs`, `upload/busboy.rs` | `library.py` (sharex), `tests/security_fixes_db.rs` (fix 2), e2e |
| `server.ts`: `fail()`, socket error handling, request timeouts off | `http.rs`, `error.rs` | `tests/http_edge.rs` |
| `server.ts`: timers | `jobs.rs` (section 6 below) | unit tests in `upload/tus.rs`, `maintenance/trash.rs` |
| `server.ts`: `reportRuntimeLimits` | `main.rs` + `docker/entrypoint.sh` | image start log |
| `proxy.ts` (shortener host, `/dashboard` gate) | `http/proxy.rs` | `tests/http_edge.rs`, e2e (smoke: dashboard redirects) |
| `next.config.ts` `headers()` (CSP, security headers, cache) | `http/headers.rs` | every parity suite compares the headers that matter |
| Next's `compress` | `http/compression.rs` (9.42) | `tests/page_framework.rs`, unit tests |
| `migrate.ts` + drizzle migrator 0.45.2 | `db/migrate.rs`, `main.rs migrate` | `tests/db_compat.rs`, `crosscompat.py` (schema dump unchanged by the Rust migrator) |

## 3. Pages (`page.tsx`, 27), layouts and boundaries

| Upstream | Rust loader | Parity |
|---|---|---|
| `app/page.tsx` | `pages/root_page.rs` | `auth_pages.py`, e2e |
| `app/welcome/page.tsx` | `pages/welcome.rs` | `auth_pages.py` |
| `app/download/page.tsx` | `pages/download.rs` | `releases.py` |
| `app/(auth)/login/page.tsx` | `pages/login.rs` | `auth_pages.py`, e2e |
| `app/(auth)/register/page.tsx` | `pages/register.rs` | `auth_pages.py`, e2e |
| `app/(auth)/forgot/page.tsx` | `pages/forgot.rs` | `auth_pages.py` |
| `app/(auth)/reset/page.tsx` | `pages/reset.rs` | `auth_pages.py` |
| `app/(auth)/2fa/page.tsx` | `pages/two_factor.rs` | `auth_pages.py` |
| `app/(auth)/invite/[token]/page.tsx` | `pages/invite.rs` | `auth_actions.py`, `admin_users.py` |
| `app/(auth)/authorize/page.tsx` | `pages/authorize.rs` | `authorize.py` |
| `app/(app)/dashboard/page.tsx` | `pages/dashboard.rs` | `library_pages.py`, `panel.py`, e2e |
| `app/(app)/dashboard/folders/page.tsx` | `pages/dashboard_folders.rs` | `library_pages.py` |
| `app/(app)/dashboard/trash/page.tsx` | `pages/dashboard_trash.rs` | `library_pages.py` |
| `app/(app)/dashboard/duplicates/page.tsx` | `pages/dashboard_duplicates.rs` | `library_pages.py` |
| `app/(app)/dashboard/links/page.tsx` | `pages/dashboard_links.rs` | `library_pages.py` |
| `app/(app)/dashboard/paste/page.tsx` | `pages/dashboard_paste.rs` | `library_pages.py` |
| `app/(app)/dashboard/upload/page.tsx` | `pages/dashboard_upload.rs` | `library_pages.py`, e2e (Uppy upload) |
| `app/(app)/dashboard/profile/page.tsx` | `pages/dashboard_profile.rs` | `account.py` |
| `app/(app)/dashboard/tokens/page.tsx` | `pages/dashboard_tokens.rs` | `auth_pages.py`, e2e (token UI) |
| `app/(app)/dashboard/admin/page.tsx` | `pages/dashboard_admin.rs` | `panel.py`, `admin_settings.py`, e2e |
| `app/(app)/dashboard/admin/users/[id]/files/page.tsx` | `pages/dashboard_admin_user_files.rs` | `library_pages.py` |
| `app/[userCode]/[mediaCode]/page.tsx` | `pages/media.rs` (`load`) | `media_serve.py` (pages, bots, gates, fix1), e2e (viewer) |
| `app/[userCode]/page.tsx` | `pages/media.rs` (`load_tagless`) | `media_serve.py` (tagless) |
| `app/u/[userCode]/page.tsx` | `pages/public_profile.rs` | `media_serve.py` (public), e2e (extras) |
| `app/a/[userCode]/[slug]/page.tsx` | `pages/public_album.rs` | `media_serve.py` (public) |
| `app/s/[code]/unlock/page.tsx`, `app/s/[code]/[name]/unlock/page.tsx` | `pages/shortlink_unlock.rs` | `library_pages.py` |
| `app/layout.tsx` | `pages/root_layout.rs` (+ `pages/shell.rs`, `pages/meta.rs`) | every page check, e2e (`lang`) |
| `app/(app)/layout.tsx` | `pages/layout_app.rs` | `panel.py` |
| `app/(auth)/layout.tsx` | `pages/layout_auth.rs` | `auth_pages.py` |
| `app/(app)/dashboard/loading.tsx` (streaming boundary) | `pages/mod.rs` `dashboard_boundary` (9.41) | `panel.py`, `tests/page_framework.rs` |
| `app/not-found.tsx`, `app/error.tsx`, `app/(app)/dashboard/error.tsx` | not-found / error documents (`pages/mod.rs`), the SPA's boundaries | `media_serve.py`, `library_pages.py` |

## 4. Server actions (`app/actions/*.ts`, 94 exports) -> `POST /_rpc/<module>/<export>`

All 94 upstream exports are registered (checked against the source and the
SPA's `web/src/rpc/actions.manifest.json`); the RPC framework is `rpc/`.

| Module (exports) | Rust | Parity |
|---|---|---|
| `account` (20) | `actions/account.rs` | `account.py` |
| `admin` (15) | `actions/admin.rs` (8, accounts), `actions/admin_library.rs` (7, library) | `admin_users.py`, `library_actions.py`, `panel.py` |
| `authorize` (1) | `actions/authorize.rs` | `authorize.py` |
| `avatar` (3) | `actions/avatar.rs` | `library_actions.py`, `library.py` |
| `convert` (4) | `actions/convert.rs` | `convert_actions.py` |
| `duplicates` (4) | `actions/duplicates.rs` | `library_actions.py` |
| `folders` (9) | `actions/folders.rs` | `library_actions.py` |
| `invite` (3) | `actions/invite.rs` | `auth_actions.py`, `admin_users.py` |
| `login-identifier` (1) | `actions/login_identifier.rs` | `auth_actions.py`, e2e (sign in) |
| `mail` (6) | `actions/mail.rs` | `admin_settings.py` |
| `pastes` (1) | `actions/pastes.rs` | `library_actions.py` |
| `registration` (4) | `actions/registration.rs` | `admin_settings.py`, e2e (admin toggle, adapted) |
| `releases` (3) | `actions/releases.rs` | `releases.py` |
| `shortlink-unlock` (1) | `actions/shortlink_unlock.rs` | `library_actions.py` |
| `shortlinks` (4) | `actions/shortlinks.rs` | `library_actions.py` |
| `tokens` (4) | `actions/tokens.rs` | `auth_actions.py`, e2e (token UI) |
| `unlock` (1) | `actions/unlock.rs` | `media_serve.py` (gates) |
| `uploads` (10) | `actions/uploads.rs` | `library_actions.py` |

## 5. `lib/` modules (120 files)

109 have a Rust module of the same path (`lib/media/av-convert.ts` ->
`src/media/av_convert.rs`); the rest are browser-only or folded into a module
of another name. Modules that are partly client code (`"use client"` helpers
that also run on the server) exist on both sides.

| Upstream | Rust |
|---|---|
| `lib/admin-users.ts` | `admin_users.rs` |
| `lib/api-caller.ts` | `api_caller.rs` |
| `lib/api-json.ts` | `api_json.rs` |
| `lib/api-scopes.ts` | `api_scopes.rs` (also in the SPA: client component code) |
| `lib/api-token.ts` | `api_token.rs` |
| `lib/audit.ts` | `audit.rs` |
| `lib/auth-client.ts` | browser only (SPA: web/lib/auth-client.ts) |
| `lib/auth-helpers.ts` | `auth_helpers.rs` |
| `lib/auth.ts` | `auth/mod.rs` |
| `lib/authorize/loopback.ts` | `authorize/loopback.rs` |
| `lib/avatar-store.ts` | `avatar_store.rs` |
| `lib/avatar.ts` | `avatar.rs` |
| `lib/background.ts` | `background.rs` |
| `lib/client-settings.ts` | `client_settings.rs` |
| `lib/config.ts` | `config.rs` |
| `lib/credentials.ts` | `credentials.rs` |
| `lib/crypto/envelope.ts` | `crypto/envelope.rs` |
| `lib/crypto/link.ts` | browser only (SPA; the link key never reaches the server) |
| `lib/data/admin-disk.ts` | `data/admin_disk.rs` |
| `lib/data/admin-files.ts` | `data/admin_files.rs` |
| `lib/data/admin.ts` | `data/admin.rs` |
| `lib/data/duplicates.ts` | `data/duplicates.rs` |
| `lib/data/folders.ts` | `data/folders.rs` |
| `lib/data/sessions.ts` | `data/sessions.rs` |
| `lib/data/upload-query.ts` | `data/upload_query.rs` |
| `lib/data/uploads.ts` | `data/uploads.rs` |
| `lib/data/user-prefs.ts` | `data/user_prefs.rs` |
| `lib/db/index.ts` | `db/mod.rs` |
| `lib/db/schema/app-schema.ts` | `db/models.rs`, `db/types.rs` (+ `tests/db_compat.rs` against `information_schema`) |
| `lib/db/schema/auth-schema.ts` | `db/models.rs` |
| `lib/db/schema/duplicates-schema.ts` | `db/models.rs` |
| `lib/db/schema/index.ts` | `db/models.rs` |
| `lib/folders.ts` | `folders.rs` |
| `lib/format.ts` | `format.rs` |
| `lib/ids.ts` | `ids.rs` |
| `lib/invites.ts` | `invites.rs` |
| `lib/link-info/parse.ts` | `link_info/parse.rs` |
| `lib/mail-settings.ts` | `mail_settings.rs` |
| `lib/mail-templates.ts` | `mail_templates.rs` |
| `lib/mail.ts` | `mail.rs` |
| `lib/maintenance/trash.ts` | `maintenance/trash.rs` |
| `lib/media/actions.ts` | `media/actions.rs` |
| `lib/media/av-action.ts` | `media/av_action.rs` |
| `lib/media/av-convert.ts` | `media/av_convert.rs` |
| `lib/media/av-cost.ts` | `media/av_cost.rs` |
| `lib/media/av-meta.ts` | `media/av_meta.rs` |
| `lib/media/av-probe.ts` | `media/av_probe.rs` |
| `lib/media/av-targets.ts` | `media/av_targets.rs` |
| `lib/media/av-workspace.ts` | `media/av_workspace.rs` |
| `lib/media/avatar.ts` | `media/avatar.rs` |
| `lib/media/avif-brand.ts` | `media/avif_brand.rs` |
| `lib/media/bots.ts` | `media/bots.rs` |
| `lib/media/card-art.ts` | `media/card_art.rs` |
| `lib/media/card-render.ts` | `media/card_render.rs` |
| `lib/media/card.ts` | `media/card.rs` |
| `lib/media/convert-in-place.ts` | `media/convert_in_place.rs` |
| `lib/media/convert-notes.ts` | `media/convert_notes.rs` |
| `lib/media/convert-plan.ts` | `media/convert_plan.rs` |
| `lib/media/convert-size.ts` | `media/convert_size.rs` |
| `lib/media/convert-targets.ts` | `media/convert_targets.rs` |
| `lib/media/convert-verdict.ts` | `media/convert_verdict.rs` |
| `lib/media/convert.ts` | `media/convert.rs` |
| `lib/media/derived-cache.ts` | `media/derived_cache.rs` |
| `lib/media/docx-support.ts` | `media/docx_support.rs` |
| `lib/media/docx.ts` | `media/docx.rs` |
| `lib/media/download-menu.ts` | `media/download_menu.rs` |
| `lib/media/embed.ts` | `media/embed.rs` |
| `lib/media/ffmpeg.ts` | `media/ffmpeg.rs` |
| `lib/media/frame-step.ts` | `media/frame_step.rs` |
| `lib/media/image.ts` | `media/image.rs` |
| `lib/media/link-formats.ts` | `media/link_formats.rs` |
| `lib/media/mime.ts` | `media/mime.rs` |
| `lib/media/motion-preview.ts` | `media/motion_preview.rs` |
| `lib/media/motion.ts` | `media/motion.rs` |
| `lib/media/paste-type.ts` | `media/paste_type.rs` |
| `lib/media/paths.ts` | `media/paths.rs` |
| `lib/media/playback-health.ts` | `media/playback_health.rs` |
| `lib/media/player-i18n.ts` | `media/player_i18n.rs` |
| `lib/media/poster.ts` | `media/poster.rs` |
| `lib/media/probe.ts` | `media/probe.rs` |
| `lib/media/render-ticket.ts` | `media/render_ticket.rs` |
| `lib/media/serve.ts` | `media/serve.rs` |
| `lib/media/sharp-runtime.ts` | `media/sharp_runtime.rs` |
| `lib/media/sniff.ts` | `media/sniff.rs` |
| `lib/media/source-preview.ts` | `media/source_preview.rs` |
| `lib/media/timeline-preview.ts` | `media/timeline_preview.rs` |
| `lib/media/timeline.ts` | `media/timeline.rs` |
| `lib/media/transform-cost.ts` | `media/transform_cost.rs` |
| `lib/media/transform-queue.ts` | `media/transform_queue.rs` |
| `lib/media/urls.ts` | `media/urls.rs` |
| `lib/media/zip-guard.ts` | `media/zip_guard.rs` |
| `lib/naming-words.ts` | `naming_words.rs` |
| `lib/naming.ts` | `naming.rs` |
| `lib/password.ts` | `password.rs` |
| `lib/paste-languages.ts` | `paste_languages.rs` |
| `lib/quota.ts` | `quota.rs` |
| `lib/registration.ts` | `registration.rs` |
| `lib/releases-core.ts` | `releases_core.rs` |
| `lib/releases.ts` | `releases.rs` |
| `lib/runtime/limits.ts` | `runtime/limits.rs` |
| `lib/secret-box.ts` | `secret_box.rs` |
| `lib/settings.ts` | `settings.rs` |
| `lib/shortlinks.ts` | `shortlinks.rs` |
| `lib/storage/index.ts` | `storage/mod.rs` |
| `lib/storage/local.ts` | `storage/local.rs` |
| `lib/storage/s3.ts` | `storage/s3.rs` |
| `lib/storage/types.ts` | `storage/mod.rs` (`StorageDriver`) |
| `lib/theme-api.ts` | `theme_api.rs` |
| `lib/theme-cookies.ts` | `theme_cookies.rs` |
| `lib/themes.ts` | `themes.rs` |
| `lib/transfer-rate.ts` | browser only (SPA) |
| `lib/upload/disk.ts` | `upload/disk.rs` |
| `lib/upload/duplicate-check.ts` | `upload/duplicate_check.rs` |
| `lib/upload/duplicate-lookup.ts` | `upload/duplicate_lookup.rs` |
| `lib/upload/finalize.ts` | `upload/finalize.rs` |
| `lib/upload/folder-target.ts` | `upload/folder_target.rs` |
| `lib/upload/sharex.ts` | `upload/sharex.rs` |
| `lib/upload/tus.ts` | `upload/tus.rs` |
| `lib/uploads.ts` | `uploads.rs` |
| `lib/utils.ts` | browser only (SPA) |

Vendored library behaviour ported where upstream depends on it: better-auth
1.6.13 / better-call 1.3.5 / zod 4 (`auth/`), drizzle 0.45.2's migrator
(`db/migrate.rs`), @tus/server 2.4.1 + file-store 2.1.0 + s3-store 2.0.4
(`upload/tus_server/`), busboy 1.6 (`upload/busboy.rs`), sharp 0.34.5
(`media/vips/sharp.rs`, on libvips 8.17.3 built as sharp-libvips 1.2.4 builds
it, `docker/sharp-libvips/`), mammoth 1.12.1 + JSZip + xmldom (`media/mammoth/`),
pdf-lib 1.17.1 (`media/pdf_lib.rs`), archiver 8 (`archiver.rs`), blurhash 2.0.5,
nodemailer 8 (`mail/`), V8's JSON.parse/stringify (`js_json.rs`), V8's Date
parser (`js_date.rs`), next-intl ICU formatting (`i18n/`), AWS SDK v3
lib-storage and presigner (`storage/s3/`).

## 6. Background jobs (`server.ts` timers)

| Upstream | Rust | Schedule |
|---|---|---|
| `sweepTusUploads` | `jobs.rs` `tus-sweep` -> `upload::tus::sweep_tus_uploads` | at boot, then hourly |
| `purgeExpiredTrash` | `jobs.rs` `trash-purge` -> `maintenance::trash::purge_expired_trash` | 60 s after boot, then every 6 h |
| `warmMailSettings` | `jobs.rs` -> `mail_settings::warm_mail_settings` | once at boot |

## 7. Environment variables

Every variable upstream reads (`process.env.*` in `lib/`, `app/`, `server.ts`,
`migrate.ts`, `proxy.ts`, `next.config.ts`):

| Variable | Rust |
|---|---|
| `NODE_ENV`, `NEXT_PHASE`, `SKIP_ENV_VALIDATION`, `AUTH_SECRET`, `DATABASE_URL`, `APP_URL`, `APP_DOMAINS`, `APP_NAME`, `EMBED_BRAND`, `SHORTLINK_URL`, `PORT`, `TRUST_PROXY`, `REGISTRATION_ENABLED`, `STORAGE_DRIVER`, `STORAGE_LOCAL_PATH`, `MAX_UPLOAD_SIZE`, `MIN_FREE_DISK_BYTES`, `TUS_CHUNK_SIZE`, `TUS_TMP_PATH`, `UPLOAD_TMP_PATH`, `DEFAULT_USER_QUOTA`, `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASSWORD`, `SMTP_SECURE`, `SMTP_SECURITY`, `MAIL_FROM`, `MAIL_FROM_NAME` | `config.rs` (`ENV_VARS`, same parsing and production refusals) |
| `S3_ENDPOINT`, `S3_REGION`, `S3_BUCKET`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`, `S3_FORCE_PATH_STYLE`, `S3_PREFIX` | `storage/s3.rs`, `upload/tus_server/s3_store.rs` (read from the environment, as upstream) |
| `SETTINGS_ENCRYPTION_KEY` | `secret_box.rs` |
| `FFMPEG_PATH`, `FFPROBE_PATH`, `PATH` | `media/ffmpeg.rs` |
| `AUDIO_COVER_PATH` | `media/av_convert.rs` |
| `LEGACY_UPLOAD_MAX_SIZE` | `upload/sharex.rs` (default 8 MiB with security fix 2) |
| `UV_THREADPOOL_SIZE` (Node's libuv pool, set by the upstream entrypoint) | no libuv; `docker/entrypoint.sh` sets `TOKIO_WORKER_THREADS` and `VIPS_CONCURRENCY` from the cgroup instead |
| `NEXT_PUBLIC_TUS_CHUNK_SIZE` (browser build) | the SPA (`web/components/upload/uppy-client.ts`, unset by default as upstream) |
| `E2E_BASE_URL` (tests) | `tests/e2e/playwright.config.ts` |

Added by the port: `BIND_ADDR`, `MODLIGHT_ROOT`, `TZ`, `RUST_LOG`,
`PANGOCAIRO_BACKEND` (SKELETON.md section 1, 9.34), `TOKIO_WORKER_THREADS`,
`VIPS_CONCURRENCY` (entrypoint).
