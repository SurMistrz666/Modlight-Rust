# Status of the work: Modlight in Rust

Last updated: 2026-10-20: EVERYTHING IS DONE and merged into `main`; the server in Docker at 127.0.0.1:8430, `app/dist/Modlight.app` built. Pushed to the private repo https://github.com/SurMistrz666/Modlight-Rust (branch `main`). What remains: checking the app in a real window with the owner (macOS permissions; the lists are in `docs/handoff/app-3.md`, `app-4.md`, `app-5-tools.md`, `app-5-effects.md`).

## Goal (the owner's decisions of 2026-10-18)

- Download Nimfek/Modlight (the server) and Nimfek/Modlight-App (the Windows client), check for vulnerabilities,
  logic errors and whether the code exfiltrates data.
- Rewrite **both** in Rust preserving behavior 1:1; the app is to run on the Mac
  and have **all** the ShareX tools, built in stages.
- Run only locally in Docker (127.0.0.1), expose nothing to the world.
- At the end push to the owner's private repo on GitHub (the account SurMistrz666).

## Done

1. **Audit** (3 agents, static analysis): the reports are in `docs/audit/`.
   Result: nothing exfiltrates data, no backdoors and no hidden accounts. Fixes to make
   in the Rust version: `server/docs/PORT.md` section 12 and `app/docs/PORT.md` section 5.
2. **Docker**: Colima from Homebrew, the profile `modlight` (8 CPUs, 16 GB, 120 GB of disk).
3. **Reference**: the original TypeScript server in `_wzorzec/` at http://127.0.0.1:8420
   (Postgres 127.0.0.1:55420). Secrets in `_wzorzec/.env`, test accounts in
   `_wzorzec/qa-accounts.env` (do not print the values).
4. **Repository** `~/Desktop/modlight` (git, commit author: noreply SurMistrz666).
   Rules: `server/docs/PORT.md`, `app/docs/PORT.md`.
5. **The Rust server skeleton** merged into `main` (588a5a1): configuration, a drizzle-compatible
   migrator (a schema identical to the original), models of all tables, RPC for actions
   (`/_rpc`), page data (`/_page`) and the SPA shell, reading of sessions compatible with Better Auth
   (checked against a session from the original), API tokens and scopes, `/api/health`, 178 tests.
   Description: `server/docs/SKELETON.md`.

## In progress (paused, each branch has its own `HANDOFF.md`)

| Branch | Worktree | What it does |
|---|---|---|
| `web-spa` | merged into `main` | the web panel is done (`npm run verify` green), CONTRACT.md corrected in full (508eff7). The backend work from section 12 of the contract goes in the server branches |
| `app-1` | merged into `main` (dff2386) | the app for the Mac, stages 1 and 2: sign-in, library, appearance (screenshots of 42 screens x light/dark x EN/PL), the `Modlight.app` script, uploading (queue, tus with resuming, offline, history, pasting, shortener, watched folders). Windowless only; the real window is to be checked with the owner |
| `app-3` | merged into `main` (a42c49f) | stage 3: all kinds of captures (an overlay on every screen), shortcuts (Cmd+Shift+1/2/7/8/9), video and GIF recording (ScreenCaptureKit + ffmpeg), components with SHA-256, yt-dlp downloading, YouTube sign-in, the Server tab, all the settings pages, start at login, the Finder service; 470 tests. The list to check in a real window: `docs/handoff/app-3.md` |
| `srv-auth` | merged into `main` (bbde0a3) | accounts, Better Auth-compatible sign-in, 2FA, admin, mail, releases, the auth pages and layout (app); the comparison scripts against the original are green, account/session/2FA compatibility in both directions 227/227; fixes 3, 4, 7 deployed. Details and the remaining differences: `docs/handoff/srv-auth.md` |
| `srv-library` | merged into `main` (1be08bc) | uploading (ShareX with fix 2, tus compatible with Uppy and the app), local and S3 storage, folders, trash, duplicates, short links, avatars and wallpapers, the libvips binding, the library pages; 975 tests, comparisons with the original green. Open items for media: `docs/handoff/srv-library.md` |
| `srv-media-a` | merged into `main` (78eee7c) | conversions, moving previews and timelines, covers, docx (mammoth), OG cards, pdf; 1212 tests; with ffmpeg 5.1 the outputs are byte for byte like the original, the cards pixel for pixel |
| `srv-integration` | merged into `main` (0933c6d) | the server is DONE: the Docker image (libvips 8.17.3 as in sharp, ffmpeg 5.1.9), the `modlight-rust` stack at 127.0.0.1:8430 (running, empty database: the first account = admin), all comparisons with the original green, e2e 12/15 as on the original, all 7 fixes with tests, `server/README.md` (originally in Polish), `server/docs/COVERAGE.md` |
| `app-5-effects` | merged into `main` (ca86544) | 230 editor effects (677 reference images from the original SkiaSharp in Docker) + 51 classic ShareX effects with presets and .sxie packs; the effects window wired up |
| server security review | `docs/audit/rust-server.md` (d665cb6) | no Critical or High vulnerabilities, no sending of data, 7 fixes effective; the 2FA code comparison already in constant time |
| app security review | `docs/audit/rust-app.md` (21b3764) | no sending of data, the rules from the audit preserved; fixed W1 (a backup import will not replace the host, the token is bound to the host), W2 (the queue deletes only its own files), W3 (foreign programs and the ffmpeg path from an import arrive disabled), S3 (image decoding limits); 558 tests |
| `app-5-tools` | merged into `main` (88ea827) | ShareX tools: color pickers, pinning, browser, printing, image combiner, video converter, metadata (ExifTool + /usr/bin/perl), folder index, QR (rxing), OCR (Vision), AI analysis (keys in the Keychain), background removal (ONNX Runtime through `ort`, Core ML), history windows; 649 tests. The first clean build downloads ONNX Runtime from cdn.pyke.io (pinned SHA-256) |
| `app-4` | merged into `main` (e303cd9) | the image editor: all annotations, history, zoom, interface, the effects registry, annotations in the area-capture overlay, "Beautify"; 544 tests in app/. The list to check in a window: `docs/handoff/app-4.md` |
| `srv-media-b` | merged into `main` (e549b53) | file serving, public pages, embeds, oEmbed, albums from zip, deletion by token; fixes 1 (view limit: the `ml_view_<id>` pass), 5, 6; 1080 tests, 1064 comparisons matching. Waiting for part A to be wired in `media/part_a.rs`. A bug found in the original: the album zip always returns 500 |

Ready-made prompts for subsequent agents: `_scratch/prompts/` (`common-server.md`,
`srv-library.md`, `srv-media.md`).

## Further order

1. Review the `HANDOFF.md` of the three branches, finish them (resume the agents with a note),
   check and merge into `main`.
2. Server: `srv-library`, then media (part A: processing, part B: serving,
   public pages, embeds), then integration: the Docker image at 127.0.0.1:8430,
   the Playwright tests from the original, comparisons of responses with the reference, the fixes from the audit.
3. The app for the Mac: stage 2 (uploading, offline queue, resuming), stage 3 (captures,
   recording, yt-dlp downloading, components, settings), stage 4 (the image editor),
   stage 5 (effects and the remaining ShareX tools).
4. A security review of the Rust code, then the private repo on GitHub.

## Decisions to confirm with the owner

- The redirect of `/dashboard` and pages not found: the original answers 200, the skeleton 307/404.
- A bad `%` character in the address: the original 500, the skeleton 400.
- The "501: not ported to the Rust server yet" box in the panel, visible in every build until the port is complete.
- The name of the private repo on GitHub (one `modlight` repo or two).

## Handing over to another session

The full note for the next Claude session: `~/Desktop/claude/stan_agentow/modlight-rust.md`
(outside the repo, with links to the test accounts and the session). The agents from session 25670274 cannot be resumed
from another session: a new session starts fresh agents from the `HANDOFF.md` of the given branch and the prompts
from `_scratch/prompts/`.

## Resuming

```bash
colima start --profile modlight
docker compose -f ~/Desktop/modlight/_wzorzec/docker-compose.yml up -d
~/Desktop/modlight/server/scripts/dev.sh db    # brings up the dev Postgres at 127.0.0.1:55421
```

Ports (127.0.0.1 only): the reference 8420, Rust dev 8421 (the agents also 8423 and further),
the dev Postgres 55421, the target Docker with the Rust version 8430.

Rules for the agents' work: at most 3 at a time, each in its own worktree, run nothing
from `upstream/`, do not open the app on the owner's screen (QA mode only).
