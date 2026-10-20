# srv-integration handoff (living document, 2026-10-20)

Branch `srv-integration`, worktree `_wt/srv-integration` (from main 78eee7c).
Task: prove the Rust server is a 1:1 replacement, close the gaps, ship it as
a local Docker stack on 127.0.0.1:8430. No helper agents (owner's limits).
Scratch: `_scratch/srv-integration/` (`env.sh`, `check.sh` [QUICK=1],
`commit.sh` [DOCS=1 / QUICK=1], `logs/`, `sharp-libvips/` = the upstream
build files as downloaded).

## Status by task

| # | Task | State |
|---|---|---|
| 2 | Production image `server/docker/Dockerfile` | done (AL2/gcc10 libvips stage); 208 MB compressed, 776 MB unpacked |
| 3 | Local stack `server/docker/docker-compose.yml` + `setup-local.sh` | UP on 127.0.0.1:8430 (`server/docker/.env` generated, no accounts) |
| 7a | S3: finished upload unreachable at `GET /api/files/<id>` | done feb3b4b (SKELETON 9.38) |
| 7b | Delete the previous avatar / wallpaper object | done ca4c342 (SKELETON 9.39) |
| 7c | PORT.md 12 fixes 1-7 re-checked, each with a test | done c014150 (SKELETON 9.40, tests/security_fixes_db.rs) |
| 4 | Parity suites against the Docker image | done, all green (Parity results below) |
| 5 | Upstream e2e (Playwright) on both servers | 12/15 on both, the same 3 stale specs fail on the reference too; adapted copies 3/3 on both |
| 6 | Compression, redirect bodies, 500 bodies, /dashboard, JSON.parse | done 45060e2 (SKELETON 9.11, 9.24, 9.25, 9.41, 9.42) |
| 1 | `server/docs/COVERAGE.md` | done 0005e6f (+ tests/coverage.rs: no page or handler answers 501) |
| 8 | Final verification, `server/README.md` (Polish) | done: 1224 tests, every suite, e2e, smoke 12/12; stack on 8430 rebuilt from HEAD |

## Image (task 2)

- Context `server/`, `docker build -f server/docker/Dockerfile -t modlight-rust:local server`.
  Size: 208 MB compressed (`docker image inspect` Size 207886568), 776 MB
  unpacked (`docker images`); the reference image is 2.14 GB.
- libvips: `server/docker/sharp-libvips/` holds sharp-libvips v1.2.4's
  `build/posix.sh`, `versions.properties`, `platforms/linux-*/meson.ini` and
  `Toolchain.cmake` (identical for arm64 and x64) unmodified (tag commit
  20b5e899954907a3039d6e3d4c200aaa0ec52c4c, Apache-2.0). The `libvips` stage
  is sharp-libvips' own build image (Amazon Linux 2, gcc 10; pinned
  amazonlinux:2.0.20260930.0, stable Rust 1.99 instead of nightly, cargo-c
  0.10.25 with vendored OpenSSL, meson 1.9.1, ninja 1.11.1.4) running posix.sh.
  A first try on Debian 12/gcc 12 worked too, but libaom's AVIF output then
  differed in bytes from the reference (FP contraction differs between gcc
  10 and 12; output also depends on the thread count): with gcc 10 every
  AVIF is byte-identical. The binary links `libvips-cpp.so.8.17.3` (libvips
  + every dependency static, GLib/GObject exported) through a generated
  `vips.pc`.
- ffmpeg: Debian 12's package, 5.1.9-0+deb12u1 = the reference image's.
- Health check: `modlight-server health` (new subcommand).
- Entrypoint `server/docker/entrypoint.sh`: cgroup memory/CPU report,
  `VIPS_CONCURRENCY` / `TOKIO_WORKER_THREADS` from `cpu.max`, `exec
  modlight-server serve` (validates config, migrates, serves).

## Parity (task 4)

Scratch stacks in `_scratch/srv-integration/parity/` (all brought down with
`down -v` at the end; `docker compose up -d` in each directory recreates
them empty): `ref/` (the ORIGINAL image `modlight-wzorzec-app:latest`, 8448,
pg 55448), `rust/` (`modlight-rust:local`, 8449, pg 55449, storage
bind-mounted at `rust/data`), `x/` (`modlight-rust:local` on the REFERENCE
database, 8450, for crosscompat; start it after `ref/`), `shim/`
(ffmpeg/ffprobe argument loggers, both sides), `probe/Dockerfile`
(media_probe + media_docx test binary built in the image's `server` stage:
`docker build --target server -t modlight-rust-build:local -f
server/docker/Dockerfile server`, then `docker build -t
modlight-rust-probe:local -f _scratch/srv-integration/parity/probe/Dockerfile
server/tests`). Both AUTH_SECRETs are the same. `run-all.sh` runs everything
and writes `logs/summary.txt`; `run-library.sh`, `run-auth.sh` (SUITES="..."),
`run-media-parity.sh` (SKIP_REF=1 reuses the reference collection),
`tus_parity.py`. Changes made to the scripts for the Docker image: `--ref-app`
for library.py, `--ffmpeg docker` and env-configurable stacks for
media_parity.py (the probe runs with the reference's `--cpus 2`: x264 and
libaom output depends on the thread count), `DOCX_RUST_IMAGE` for
docx_parity.py, `--ref-env/--rust-env` for convert_actions.py,
`PARITY_RUST_NETNS` (SMTP sinks in the Rust container's network namespace),
panel.py's expectations (ported /dashboard, RPC admin data, default quota).

Results on the final image (2026-10-20, same order as `run-all.sh`):

| Suite | Result |
|---|---|
| library.py | 369/369 |
| library_pages.py | 185/185 |
| library_routes.py | 333/333 |
| library_actions.py | 417/417 on fresh stacks (instance-wide admin checks need both databases in the same state; after the other suites ran first it reports state drift, not a port difference) |
| media_serve.py | 1109/1109; the one encoder-dependent body that still differs in bytes is the PDF (random XObject name and date, both sides) |
| auth.py | 180/180 |
| auth_actions.py | 89/89 |
| auth_pages.py | 104/104 |
| account.py | 559/559 |
| releases.py | 171/171 |
| api_misc.py | 399/399 |
| authorize.py | 155/155 |
| admin_settings.py | 436/436 |
| panel.py | 83/83 |
| admin_users.py | 618/618 |
| tus-js-client 4.3.1 (tus_client.cjs) | traces identical |
| convert_actions.py | 20/20 |
| media_parity.py (`--ffmpeg docker`) | 640 cases: status, headers, ffmpeg/ffprobe argument vectors 640/640; outputs byte-identical 154, the other 27 are 15 PDFs (random per call), 8 Matroska (random segment/track UIDs) and 4 Ogg (random serial numbers; only the serial and CRC bytes differ): every deterministic output is byte-identical, AVIF and PNG included |
| media_parity.py cards | 155/155 byte-identical |
| docx_parity.py | 222 identical, 3 expected, 4 upstream hangs (SKELETON 13) |
| crosscompat.py (Rust image on the reference's database) | 269/269 |

## Decisions taken (task 6), all with the reference's answers

- Compression: Next compresses documents, RSC payloads and static files
  (gzip, `Vary: Accept-Encoding`, static from 1 KiB), never route handlers
  (`/api/*`, media, oembed, embed, icon routes). Same here
  (`http/compression.rs`; `/_page` and `/_rpc` stand for the RSC payloads),
  never `206`. SKELETON 9.42.
- Document redirects: page/layout `redirect()` = 307 + Location + a whole
  HTML body (Next's error shell; here the SPA shell with the redirect
  payload). Proxy gate and trailing-slash bodies were already as upstream.
- `/dashboard` boundary: a page's own `redirect()`/`notFound()` under
  `app/(app)/dashboard/` answers 200 with the page's title (+ noindex, +
  refresh tag inside `<noscript>`). Layout redirects stay 307. SKELETON 9.41,
  CONTRACT 12.4 decided.
- 500 bodies: a thrown route handler = empty 500, no content type (Next);
  raw upload paths keep server.ts `fail()` JSON. SKELETON 9.24.
- `read_json_object`: V8 `JSON.parse` (`js_json`), `1e400` reaches the field
  validators as Infinity (tagged `{"$num":"Infinity"}` in the serde values),
  any nesting accepted. SKELETON 9.25.
- Not changed: malformed percent-escapes stay 400 (9.1; the reference's 500
  is Next's own decodeURIComponent failure).

## e2e (task 5)

Specs: `server/tests/e2e/*.spec.ts` byte-identical to upstream's
`tests/e2e`, `playwright.config.ts` = upstream's with `testDir: "."`.
Runner: `_scratch/srv-integration/e2e/` (`@playwright/test` 1.60.0, its
Chromium in `browsers/`; `run.sh <base> <label>`, `prepare.py <base> <pg
container>` = registration open + admin@local.test/ToqOse, the specs' own
seed; `run-adapted.sh`). Rust ran on a throwaway stack made from the
production compose file (`e2e/stack`, project `modlight-rust-e2e`, 8451).

Results (both servers identical): 12 passed, 3 failed. The 3 fail on the
reference too because the specs are older than upstream's UI: the login
field is `#identifier` since upstream added username sign-in (specs fill
`#email`), the library heading reads "All files" (specs expect "Your
files"), and the admin page opens on the Overview tab (spec: "The Settings
tab is the default"). `run-adapted.sh` fixes only those selectors in a
scratch copy: 3/3 on both. Note: the admin spec toggles registration and
leaves it closed when it fails half-way; `prepare.py` reopens it.

## Fixes (task 7)

- 7a: tus server options `serve_finished_uploads=false`,
  `disable_termination_for_finished_uploads=true`; S3 `delete_record` drops
  `.info`/`.part` after finalize. Unit test in `upload/tus_server/server.rs`,
  `tests/library_s3_e2e.rs` (run `_scratch/srv-integration/s3-e2e.sh -q --test
  library_s3_e2e` with MinIO from `_scratch/srv-library/s3/minio/up.sh`).
- 7b: `avatar_store::set_avatar_key`, `routes/api/background::set_background_key`
  (previous key under the row lock), discard limited to the account's own keys.
  Test `tests/library_routes_db.rs` `replacing_avatar_or_wallpaper_deletes_the_previous_object`.
  Parity scripts that count stored objects after a replacement must expect
  the reference to keep the old object.
- 7c: SKELETON 9.40 maps each PORT.md 12 fix to its test.

## Final verification (task 8)

- `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test`
  (DB tests on the dev Postgres): 1224 passed. S3 tests against MinIO
  (`_scratch/srv-integration/s3-e2e.sh -q --test library_s3_e2e --test
  tus_s3_store --test storage`): 16 passed. MinIO stopped again.
- Smoke on a THROWAWAY copy of the production compose file
  (`_scratch/srv-integration/smoke/`, port 8452, `REGISTRATION_ENABLED=false`,
  removed afterwards; script `smoke/smoke.mjs`, run from `e2e/`): 12/12 —
  first account registered in Chromium lands on /dashboard and is the admin,
  token through the panel's RPC, ShareX text and PNG, raw bytes, tus with
  resume, public page with og tags, embed card from og:image and from its
  route, PNG -> WebP conversion, health.
- The stack on 127.0.0.1:8430 (project `modlight-rust`) runs the image built
  from the final commit, healthy, no account in it (`user` table empty),
  `server/docker/.env` generated by `setup-local.sh` (mode 600).

## Still different from upstream, on purpose

SKELETON.md section 9 is the list. New at integration: 9.24 (empty 500, now
as upstream), 9.25 (V8 JSON bodies, now as upstream), 9.38 (tus URL of a
finished upload), 9.39 (old avatar/wallpaper deleted), 9.40 (fix tests),
9.41 (document redirects, dashboard boundary), 9.42 (compression).
Remaining notable ones: malformed `%` escapes 400 (upstream 500), album ZIP
works (upstream always 500), HTTP/1.1 wire details (no `Vary: rsc`, Content-
Length where Node chunked), libvips warnings and Pango backend notes.
