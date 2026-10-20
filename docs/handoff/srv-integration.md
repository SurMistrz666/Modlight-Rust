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
| 4 | Parity suites against the Docker image | all green on the Debian build; final rerun on the AL2 build pending |
| 5 | Upstream e2e (Playwright) on both servers | todo |
| 6 | Compression, redirect bodies, 500 bodies, /dashboard, JSON.parse | todo |
| 1 | `server/docs/COVERAGE.md` | todo |
| 8 | Final verification, `server/README.md` (Polish) | todo |

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

## Parity (task 4), scratch stacks

`_scratch/srv-integration/parity/`: `ref/` (ORIGINAL image, 8448, pg 55448),
`rust/` (modlight-rust:local, 8449, pg 55449, storage bind-mounted at
`rust/data`), `x/` (modlight-rust:local on the REFERENCE database, 8450, for
crosscompat), `shim/` (ffmpeg/ffprobe argument loggers), `probe/Dockerfile`
(media_probe + media_docx test binary built in the image's server stage).
Both AUTH_SECRETs are the same. Runners: `run-library.sh`, `run-auth.sh`
(SUITES="..." to pick), `run-media-parity.sh` (SKIP_REF=1 reuses the
reference collection), `tus_parity.py`. Logs in `parity/logs/`.

## Decisions taken (task 6)

(none yet)

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
