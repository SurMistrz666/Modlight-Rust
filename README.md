# Modlight (Rust)

A Rust rewrite of [Modlight](https://github.com/Nimfek/Modlight), a self-hosted file and
screenshot host, and of [Modlight App](https://github.com/Nimfek/Modlight-App), its desktop
client (a ShareX fork for Windows), rebuilt here as one Rust app for macOS and Windows.

The server is a 1:1 replacement for the original Next.js server: same URLs, same API, same
responses, same PostgreSQL schema. An install can switch between the two implementations in
both directions. The only intended differences are security and bug fixes, listed below and in
[`server/docs/SKELETON.md`](server/docs/SKELETON.md) (section 9).

## What's inside

| Path | What it is |
|---|---|
| [`server/`](server/) | The server: Rust (axum, sqlx, PostgreSQL), ~90k lines. The web panel keeps the original React components, served as a single-page app without Next.js. Ships as one Docker image. |
| [`app/`](app/) | The desktop app for macOS and Windows: Rust (egui), ~190k lines in eight crates. Everything but the system glue (`modlight-macos`, `modlight-windows`) is shared. |
| [`docs/audyt/`](docs/audyt/) | Security audits of the original code and of this rewrite (in Polish). |
| [`docs/handoff/`](docs/handoff/) | Notes from each stage of the port. |

### Server

- Web panel: library, folders, trash, duplicate finder, short links, pastes, profile, API
  tokens, administration (accounts, quotas, disk, mail, registration).
- Uploads: ShareX-compatible `POST /upload` streamed with constant memory, and resumable
  uploads over tus 1.0 (`/api/files`) for the panel (Uppy) and the desktop app.
- Storage on local disk or any S3-compatible service.
- Public file pages, galleries and albums with ZIP download; previews for Discord, Telegram,
  Slack and other link unfurlers (Open Graph, oEmbed, rendered cards).
- Media processing with the same libvips and FFmpeg versions as the original: thumbnails,
  animated previews, timeline scrubbing, posters, "download as" conversions between image,
  video and audio formats, DOCX rendering.
- Accounts compatible with the original's Better Auth data: password hashes, sessions,
  two-factor authentication with backup codes, e-mail verification, password reset,
  invitations, admin roles and bans.
- Optional Cloudflare Turnstile on sign-in, sign-up, password reset and invitations
  (Admin → Settings; off until keys are entered).
- English and Polish.

### macOS app

- Sign-in through the browser (loopback, RFC 8252); the token is stored in the Keychain.
- Your library as tiles: thumbnails, animated previews, a details drawer with every link
  format, drag and drop between folders and from Finder, trash.
- Uploads with live progress, speed and time left; an offline queue; large files resume where
  they stopped.
- Screen capture: full screen, monitor, window, region (with on-the-spot annotation), last
  region, auto capture, scrolling capture; global hotkeys.
- Screen recording to video and GIF (ScreenCaptureKit + FFmpeg).
- Video downloads with yt-dlp (YouTube, TikTok, X); components are downloaded on demand and
  verified against pinned SHA-256 digests.
- The ShareX image editor: arrows, shapes, text, speech balloons, step numbers, blur,
  pixelate, highlight, magnify, spotlight, emoji, crop, cut-out, undo/redo.
- 230 image effects from the editor library plus the 51 classic ShareX effects with presets.
- ShareX tools: colour picker and screen colour picker, pin to screen, image viewer, print,
  image combiner, video converter, metadata viewer and stripper, directory indexer, QR codes,
  OCR (Apple Vision), image analysis with your own OpenAI, Gemini or OpenRouter key,
  background removal (ONNX, Core ML), upload history.
- Settings and theme synchronised with the server, in the same format as the Windows app.
- Tray menu, notifications, Finder service "Upload with Modlight", start at login.

### Windows app

The same app, with upstream's Windows mechanisms back in place of the macOS ones
([`app/docs/WINDOWS.md`](app/docs/WINDOWS.md)): the token in Credential Manager; capture
through GDI with the region overlay above the taskbar; recording with DirectShow devices
and the NVENC, AMF and Quick Sync encoders; Windows OCR; GDI printing; the YouTube
sign-in in WebView2; background removal on DirectML; "Start with Windows", "Upload with
Modlight" in Explorer's context menu and "Send to"; one running copy that takes the files
a later start is given; the tray, taskbar progress and Print Screen hotkeys.

## Running it

### Server (local Docker stack)

Requires Docker (on macOS, for example [Colima](https://github.com/abiosoft/colima)).

```sh
server/docker/setup-local.sh          # first time only: writes server/docker/.env with random secrets
docker compose -f server/docker/docker-compose.yml up -d --build
```

The panel is at http://127.0.0.1:8430. The port is published on the loopback address only.
**The first account registered on an empty install becomes the administrator.** Day-to-day
operation, backups and upgrades: [`server/README.md`](server/README.md).

### macOS app

Requires the Rust toolchain on an Apple Silicon Mac (macOS 13 or newer; capture features use
APIs from macOS 14).

```sh
app/scripts/build-app.sh               # builds app/dist/Modlight.app (ad-hoc signed)
```

Open the app, choose **Sign in**, enter your server's address and approve the app in the
browser. The first clean build downloads ONNX Runtime (pinned SHA-256) for the background
remover.

### Windows app

Requires the Rust toolchain (MSVC target) and the Visual Studio Build Tools with a Windows
SDK, on Windows 10 2004 or newer (x64; arm64 builds the same way).

```powershell
app\scripts\build-app.ps1               # app\dist\Modlight\ and Modlight-<version>-windows-<arch>.zip
```

The folder runs where it is (no installer); settings live in `Documents\Modlight`. Sign in
as on the Mac.

## Development

```sh
cd server && cargo test                          # ~1250 tests; DB tests need a Postgres URL
cd server/web && npm run verify                  # SPA: typecheck, tests, build, bundle audit
cd app && cargo test --workspace                 # macOS or Windows; headless snapshots included
```

Behaviour is checked against the original server running side by side: parity suites in
`server/tests/parity*` compare statuses, headers and bodies for every route, page and action
(several thousand cases), and the original end-to-end suite runs against both servers.
Coverage of every upstream file: [`server/docs/COVERAGE.md`](server/docs/COVERAGE.md).
Porting rules: [`server/docs/PORT.md`](server/docs/PORT.md), [`app/docs/PORT.md`](app/docs/PORT.md).
macOS substitutions for Windows features: [`app/docs/MACOS.md`](app/docs/MACOS.md); what the
Windows build uses: [`app/docs/WINDOWS.md`](app/docs/WINDOWS.md).

## Security

An audit of the original code found no data exfiltration, telemetry or backdoors. This rewrite
fixes what the audits found:

- view limits ("burn after reading") could be bypassed with `Range` requests or
  `Sec-Fetch-Dest` headers;
- the ShareX endpoint wrote up to ~512 MB to disk before checking the token;
- two concurrent sign-ups on an empty install could both become administrator;
- rate limits trusted a client-supplied `X-Real-IP` header without a proxy;
- delete tokens and TOTP codes were compared in non-constant time;
- oEmbed described files that have a view limit;
- on S3, a finished tus upload stayed downloadable at its upload URL regardless of privacy;
- replaced avatars and wallpapers were never deleted;
- album ZIP downloads always failed (HTTP 500) in the original;
- in the app: a restricted sign-in loopback (no wildcard CORS), downloads that must match a
  known size and digest, a backup import that cannot redirect the account token or delete
  files, programs from imported settings arriving disabled, and decoding limits for images.

Reviews of this code: [`docs/audyt/rust-serwer.md`](docs/audyt/rust-serwer.md) and
[`docs/audyt/rust-aplikacja.md`](docs/audyt/rust-aplikacja.md). Neither component sends data
anywhere except the account's own server, the mail and S3 services an administrator
configures, Cloudflare Turnstile when enabled, and GitHub for verified app components.

## License

This is a derivative work, so the original licenses apply: the server is under the
[GNU AGPL-3.0](server/LICENSE) (as Modlight), the app under the [GNU GPL-3.0](app/LICENSE.txt)
(as ShareX and Modlight App; third-party notices in [`app/Licenses/`](app/Licenses/)). If you
run the server as a service for other people, you must offer them its source code.

## Credits

- [Modlight](https://github.com/Nimfek/Modlight) and [Modlight App](https://github.com/Nimfek/Modlight-App)
  by Nimfek, the originals this is ported from.
- [ShareX](https://github.com/ShareX/ShareX) by the ShareX Team, the application Modlight App
  is built on.
- [libvips](https://www.libvips.org), [FFmpeg](https://ffmpeg.org), [tus](https://tus.io),
  [egui](https://github.com/emilk/egui), [ONNX Runtime](https://onnxruntime.ai) and the rest
  of the open-source stack listed in each `Cargo.toml` and `package.json`.

This project is not affiliated with or endorsed by Nimfek or the ShareX Team.
