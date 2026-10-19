# srv-media-a handoff (2026-10-20)

Branch `srv-media-a`, worktree `_wt/srv-media-a` (from main 1be08bc). Task:
port the media PROCESSING half of the Modlight server to Rust (part A); part B
(`srv-media-b`) ports media serving and the public pages and calls these
functions. The docx preview was done by a helper on `srv-media-a-docx`
(worktree `_wt/srv-media-a-docx`) and merged here.

Build and verify:

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd _wt/srv-media-a/server
_scratch/srv-media-a/check.sh        # fmt + clippy -D warnings + cargo test (DB tests included)
```

`_scratch/srv-media-a/commit.sh "subject" ["body"]` commits only when the check
passes. libvips (Homebrew 8.18) and ffmpeg are needed to build and test.

## What is ported (upstream -> Rust)

| Upstream | Rust |
|---|---|
| `lib/media/convert.ts` | `media/convert.rs` (+ `media/pdf_lib.rs`: the part of pdf-lib 1.17.1 `imageToPdf` uses) |
| `convert-plan.ts`, `convert-size.ts`, `convert-verdict.ts`, `convert-notes.ts`, `convert-in-place.ts` | `media/` same names |
| `download-menu.ts` (needed by convert-plan) | `media/download_menu.rs` |
| `av-convert.ts`, `av-action.ts` | `media/av_convert.rs`, `media/av_action.rs` |
| `motion.ts`, `motion-preview.ts` (the rest) | `media/motion.rs`, `media/motion_preview.rs` |
| `timeline.ts`, `timeline-preview.ts`, `frame-step.ts`, `poster.ts`, `source-preview.ts` | `media/` same names |
| `card.ts`, `card-art.ts`, `card-render.ts`, `player-i18n.ts` | `media/` same names |
| `playback-health.ts` (`diagnosePlayback`) | `media/playback_health.rs` |
| `derived-cache.ts` (read/write half) | `media/derived_cache.rs` |
| `docx.ts`, `docx-support.ts`, `zip-guard.ts`, mammoth 1.12.1 | `media/docx.rs`, `media/docx_support.rs`, `media/zip_guard.rs`, `media/mammoth/` (helper branch) |
| `app/actions/convert.ts` | `actions/convert.rs` (RPC module `convert`) |
| libvips: `flatten`, `join { animated }`, `composite`, text input | `media/vips/{image,sharp}.rs` |
| `server.ts` `reportRuntimeLimits` (libvips threads, budget warning) | `main.rs` |

Tests: the upstream unit tests of every module above next to it; integration
tests `tests/media_convert.rs`, `media_derived_cache.rs`, `media_motion.rs`,
`media_web_playable.rs` (+ video-playback), `media_docx.rs` (helper). Not
ported here: `motion-route`, `timeline-route`, `player-ssr`,
`player-contract` (route and React tests: part B / the SPA).

The API part B calls is SKELETON section 12; deviations are SKELETON section 9,
items 33-37 (+ the docx section).

## Parity (against the scratch reference)

Reference: `modlight-srvmedia-a-ref` (`_scratch/srv-media-a/ref/docker-compose.yml`,
app 127.0.0.1:8446, Postgres 55446, secrets in `ref/.env`), the ORIGINAL image
with ffmpeg/ffprobe behind a logging shim (`ref/shim`, `FFMPEG_PATH`).
Rust server: `_scratch/srv-media-a/serve.sh` (127.0.0.1:8426, database
`modlight_srv_media_a`, storage `_scratch/srv-media-a/data/storage`), stopped
by `stop.sh` (by port). `modlight-srvmedia-a-ff`: a helper container of the
reference image with the work tree mounted at the same path, so the Rust code
can run the reference's ffmpeg 5.1 (`--ffmpeg ref`).

```sh
_scratch/srv-media-a/run-media-parity.sh      # ref + rust (ref ffmpeg) + compare + rust (host ffmpeg) + compare + cards
python3 server/tests/parity_media/convert_actions.py   # Rust server started with FFMPEG_PATH=$S/parity/shim-ref/ffmpeg
```

Results on 2026-10-20: see SKELETON section 12 (statuses, headers and
ffmpeg/ffprobe argument vectors identical in every case; byte identity where
the encoders are deterministic across the libvips/ffmpeg builds; every embed
card pixel-identical; the convert RPC module 20/20) and item 33 for what
Homebrew's ffmpeg 9 changes.

## Open / for others

- Part B: the routes. Everything they need is listed in SKELETON section 12;
  `card.rs` is ported here (the card route needs `parse_card_file` and
  `render_card`).
- Runtime image: Debian 12 (ffmpeg 5.1, the reference's major) and libvips
  >= 8.17; on ffmpeg 7+ three upstream argument vectors fail (item 33).
- `convertTargetsForUpload`, `convertTargetsForSelection`, `convertUploads`
  have no action id in the reference build (no client component imports
  them), so they were checked against `convertUpload` and the CONTRACT, not
  against the reference directly.

## Outside the repo

`_scratch/srv-media-a/`: env and scripts, the corpus (`corpus/`, built by
`tests/parity_media/corpus.py`), parity work dirs (`parity/`), card renders
(`card/`, `parity/cards`), library sources copied out of the reference image
(`ref-src/`: mammoth, pdf-lib, lop, xmldom, jszip, pako, ...), the docx
helper's scratch (`docx/`). Databases: `modlight_srv_media_a` on the dev
Postgres (parity rows only). Containers: the reference stack and
`modlight-srvmedia-a-ff` (removed at the end of the session, volumes kept).
