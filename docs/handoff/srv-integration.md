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
| 2 | Production image `server/docker/Dockerfile` | written; first build running (`logs/build1.log`) |
| 3 | Local stack `server/docker/docker-compose.yml` + `setup-local.sh` | written, not yet up |
| 7a | S3: finished upload unreachable at `GET /api/files/<id>` | todo |
| 7b | Delete the previous avatar / wallpaper object | todo |
| 7c | PORT.md 12 fixes 1-7 re-checked, each with a test | todo |
| 4 | Parity suites against the Docker image | todo |
| 5 | Upstream e2e (Playwright) on both servers | todo |
| 6 | Compression, redirect bodies, 500 bodies, /dashboard, JSON.parse | todo |
| 1 | `server/docs/COVERAGE.md` | todo |
| 8 | Final verification, `server/README.md` (Polish) | todo |

## Image (task 2)

- Context `server/`, `docker build -f server/docker/Dockerfile -t modlight-rust:local server`.
- libvips: `server/docker/sharp-libvips/` holds sharp-libvips v1.2.4's
  `build/posix.sh`, `versions.properties`, `platforms/linux-*/meson.ini` and
  `Toolchain.cmake` (identical for arm64 and x64) unmodified (tag commit
  20b5e899954907a3039d6e3d4c200aaa0ec52c4c, Apache-2.0). The `libvips` stage
  runs posix.sh as the sharp-libvips build image does (PLATFORM/FLAGS per
  arch, `PKG_CONFIG="pkg-config --static"`, the meson cross file) on Debian 12
  with gcc 12 (sharp-libvips: Amazon Linux 2, gcc 10) and stable Rust 1.99
  (sharp-libvips: nightly; the two `-Z` size flags are dropped). The result
  is `libvips-cpp.so.8.17.3` (libvips + every dependency static inside, GLib
  and GObject exported): the binary links it through a generated `vips.pc`.
- ffmpeg: Debian 12's package (5.1), as the reference image.
- Health check: `modlight-server health` (new subcommand, GET /api/health on
  127.0.0.1:$PORT).
- Entrypoint `server/docker/entrypoint.sh`: cgroup memory/CPU report,
  `VIPS_CONCURRENCY` / `TOKIO_WORKER_THREADS` from `cpu.max`, `exec
  modlight-server serve` (validates config, migrates, serves).

## Decisions taken (task 6)

(none yet)

## Fixes (task 7)

(none yet)
