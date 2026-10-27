# Modlight App for macOS: Rust port, working rules

This directory holds the Rust rewrite of Modlight App
(upstream: `../../upstream/Modlight-App`, a ShareX fork for Windows in
C#/.NET 10 with an Avalonia UI, GPL-3.0). The upstream checkout is the
**specification**: read the C# and the `.axaml` before writing a screen.

## 1. Goal

- A native macOS app (Apple Silicon first, `Modlight.app`) that does what the
  Windows app does, 1:1: the same screens, the same flows, the same settings,
  the same talk with the server (endpoints, headers, bodies, retry rules), the
  same texts in English and Polish.
- Where Windows has no macOS equivalent, the closest macOS mechanism, and the
  substitution is written down in `docs/MACOS.md`:
  DPAPI -> Keychain, Explorer context menu -> Finder Services / Quick Action,
  start with Windows -> login item, Print Screen -> configurable global hotkeys
  that do not clash with the system ones, Inno Setup installer -> `.app` bundle
  (+ optional DMG) with components downloaded from the Components window,
  WebView2 -> WKWebView, Windows OCR -> Vision, GDI/DXGI capture ->
  ScreenCaptureKit, ShareX import -> not applicable (no ShareX on macOS).
- The same app also builds for Windows (added after the five phases): there,
  upstream's own Windows mechanisms come back in place of the macOS
  substitutes, through `crates/modlight-windows`, a crate with the module and
  function names of `modlight-macos` (the app reaches whichever one its build
  uses as `crate::platform`). What it uses is in `docs/WINDOWS.md`. Shared
  crates choose with `cfg(windows)`; nothing Windows-only may change what
  macOS does.
- Settings synchronised through `GET/PUT /api/client-settings?client=desktop`
  and `/api/theme` use the same document format as the Windows app
  (`ShareX/Modlight/ModlightSettingsDocument.cs` allow-list), so a Windows
  and a Mac install of the same account share settings.

## 2. Phases

1. Foundations and library: workspace, API client, sign-in, credential
   storage, main window shell with the Avalonia look (themes, skins, Lucide
   icons), My space (folders, files, thumbnails, animated previews, details
   drawer, link formats, move, rename, trash, pager, duplicates), tray icon,
   notifications, settings and theme sync, QA harness.
2. Uploads: queue, progress/speed/ETA, history, resumable tus uploads,
   offline queue, drag and drop, clipboard upload, paste, URL shortener,
   after-upload actions, watch folders.
3. Capture and media: region/window/fullscreen/monitor capture, hotkeys,
   after-capture tasks, screen recording and GIF (ffmpeg), components
   (ffmpeg, yt-dlp, model) with pinned SHA-256, Download tab (yt-dlp),
   admin disk view, account/application/task/hotkey settings, first run,
   about, debug log.
4. Image editor (port of `ShareX.ImageEditor`).
5. Image effects and the remaining ShareX tools (color picker, ruler, QR,
   OCR, pin to screen, image combiner/splitter/thumbnailer, video
   converter/thumbnailer, metadata, hash checker, analyze image, background
   remover...), whatever exists in upstream `ShareX.Tools`, `ShareX.Avalonia`
   and `ShareX.ImageEffectsLib`.

## 3. Stack

- Rust stable (`/opt/homebrew/opt/rustup/bin`), edition 2024.
- GUI: eframe/egui (multi-viewport for overlays, toasts, pinned images),
  `egui_extras` for images, `egui_kittest` for headless snapshot tests.
- tokio on a background runtime, `reqwest` with rustls, serde.
- macOS: `objc2` family (`objc2-app-kit`, `objc2-foundation`,
  `objc2-screen-capture-kit`, `objc2-vision`, `objc2-web-kit`,
  `objc2-service-management`), `tray-icon` + `muda`, `global-hotkey`,
  `arboard`, `rfd`, `notify`.
- Credentials in the login Keychain written and read through
  `/usr/bin/security` (an item created by the app binary itself is tied to that
  build and every rebuild would prompt).
- Windows: `windows-sys` (Win32) and `windows` (WinRT, COM, renamed `winrt` in
  `modlight-windows`), `webview2-com`, `tray-icon`, `global-hotkey`, `arboard`,
  `rfd`, `notify`, `winresource` (icon and manifest, `build.rs`). Rust with the
  MSVC target; credentials in Credential Manager.
- Workspace crates: `modlight-api` (server protocol, no GUI), `modlight-core`
  (settings, queue store, tasks, components, no GUI), `modlight-macos` and
  `modlight-windows` (platform glue), `modlight-app` (the GUI binary), more as
  phases need.

## 4. QA mode and the owner's screen

- Never start the app in a way that touches the owner's real data or makes
  macOS ask for anything (Keychain, Screen Recording, Accessibility,
  Automation, notifications). Every automated run uses `--qa-profile <dir>`:
  settings and queue in that directory, credentials in a plain file there,
  capture from a fake source (a PNG), no tray, no global hotkeys, no login item.
- Screens are verified with `egui_kittest` snapshot tests (PNG files under
  `tests/snapshots/`) and a `--qa-shot <out.png>` mode that renders a named
  screen headlessly. Do not open windows on the owner's screen.
- The server for development is the reference instance of the ORIGINAL server
  at http://127.0.0.1:8420 (Docker, this Mac only). Create test accounts and
  API tokens there freely; record any credentials you create in
  `app/.qa/credentials.env` (gitignored), never in chat output.

## 5. Security rules carried over from the audit (`docs/audit/original-client.md`)

- The API token travels only in the `x-api-key` header to the account's host,
  never in a URL, never to another host, never across a redirect.
- Not upstream: the grant that opens an account's encrypted files (server
  `docs/ENCRYPTION.md` section 6) is a secret like the token. It travels only in
  `X-Modlight-Unlock`, only with the content requests (pictures, downloads, conversions)
  of the credential it was given for, and is kept like the token (Keychain / Credential
  Manager, item `vault-grant`) only when the person asks to be remembered. A `423 Locked`
  is never read as a refused token: 401 alone means signing in again.
- The app updates itself only from the server it is signed in to (its Downloads,
  `GET /api/downloads`), over https (plain http only on the loopback interface), with
  the account's token in `x-api-key` and no redirect followed; a build's size and SHA-256
  are checked before it is unpacked, and nothing is installed unasked. On a Mac a new
  build replaces the running one only when it is signed by the same developer (team
  identifier and the running app's designated requirement); an ad hoc build never
  installs an update. Windows builds carry no publisher signature yet, so on Windows the
  server and TLS are what vouch for a build (signing them and checking the signer as on
  a Mac is open). Details in `MACOS.md` and `WINDOWS.md`, "Updates and publishing".
  (Until 2026-10-27 the update host was fixed at build time, `MODLIGHT_DEFAULT_HOST`,
  which now only fills the sign-in box.)
- Every downloaded executable or model is checked against a pinned SHA-256
  before it is used, and a download without a known size or hash is a failure.
- Settings sync sends only the allow-listed fields (no token, AI keys, proxy
  password, local paths, window positions, cookies).
- Sign-in loopback: 127.0.0.1 only, random port, CSPRNG `state` compared in
  constant time, one shot; CORS limited to the server's origin (no `*`, no
  `Access-Control-Allow-Private-Network`); a request with a wrong `state` is
  answered and ignored, it does not abort the sign-in.
- Child processes (ffmpeg, yt-dlp) get argument vectors, with `--` before any
  user-supplied URL or path.
- YouTube cookies stay in a local file, only for yt-dlp, only for YouTube URLs.
- Do not port dead upstream code: `KeyboardHook`, AppVeyor/GitHub ShareX
  update checks, puush leftovers, getsharex.com links.

## 6. Rules for everyone working here

- `upstream/` is read-only. Never run anything from it.
- Work in your own git worktree and branch; small commits, English messages
  ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Code, identifiers and comments in English. UI text from the upstream
  `.resx` translations (English + Polish), loaded from files, not hard-coded. Texts
  upstream never had: `assets/mac/l10n/Mac.resx` (macOS only),
  `assets/windows/l10n/Windows.resx` (Windows only), `assets/port/l10n/Port.resx` (both
  systems: the build line, the app's updates), each with its `.pl.resx`.
- Keep `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` green.
- Never print secrets. Never bind to `0.0.0.0`. Never push to a remote.
