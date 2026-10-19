# HANDOFF: app-3-media (phase 3 slice: components, Download tab, app update)

Worktree `/path/to/modlight/_wt/app-3-media`, branch `app-3-media`,
branched from `app-3` at `dff2386` (and fast-forwarded to `app-3` `3eca62c`). Everything
under `app/`. Never pushed. Rules: `app/docs/PORT.md`; state before this slice:
`docs/handoff/app-1.md`.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # all green at the last commit, incl. 8420 tests
target/debug/Modlight --qa-shot tab-download-busy out.png [--qa-lang pl] [--qa-light]
```

New QA screens (`qa_media::SCREENS`, appended to `qa::SCREENS`), each with snapshots in
four looks under `crates/modlight-app/tests/snapshots/<screen>/`:
`tab-download`, `tab-download-busy`, `tab-download-missing`, `download-youtube`,
`components`, `settings-components`, `update-available`, `update-download`.
Renders I looked at are in `/path/to/modlight/_scratch/app-3-media/out/`.

## What was built

### modlight-core (no GUI)

- `components/` - port of `ModlightComponent(s)`, `ModlightComponentDownload`,
  `ModlightComponentUpdates`, `ModlightToolUpdate`, `ModlightUpdateFile`:
  - `mod.rs`: the catalogue (pinned URL, SHA-256, size, packaging, owned entries,
    version arguments), paths (`<profile>/Tools`, `<profile>/Models`), **`installed_path(profile, id)`**
    for the lead (ids `ffmpeg`, `yt-dlp`, `exiftool`, `background-model`), the
    `ExternalComponents*` settings, `migrate` (drops a dead custom ffmpeg path once).
  - `net.rs`: https-only fetch rule, `github.com`-only artefact rule, redirect policy,
    and the debug-only `net::testing::{allow_origin, set_github_api}` switches tests use
    (they do not exist in a release build).
  - `download.rs`: resumable download with Range, 60 s stall watchdog, cancel. A size
    must be known in advance (catalogue / release / manifest), never `Content-Length`; a
    short body is kept for resume, a long one is deleted, no size = nothing requested.
  - `verify.rs`: SHA-256 gate, delete on mismatch or on no digest.
  - `archive.rs`: zip single-file and tar.gz folder extraction; declared-size ceiling with
    upstream's `ZipManager` message, bytes actually written counted against it, entries
    leaving the folder skipped (upstream's rule), links/devices never created.
  - `install.rs`: one download at a time (global gate), staging in `.incoming`, part name
    carrying the digest, space check (`statvfs`), rename-aside replacement, retired-file
    and staging sweeps, remove.
  - `tool_release.rs`: GitHub release parsing and fetch for the yt-dlp update. **Both**
    the API `digest` and the release's `SHA2-256SUMS` must be present and agree; https and
    host exactly `github.com`.
  - `updates.rs`: version probe (argument vector, 20 s, killed with its group), latest
    known, `update(only_when_newer)`, daily schedule with 30 min failure backoff.
  - `process.rs`: children in their own process group, killed as a group on cancel, and
    by an `atexit` hook when the app exits (also through `process::exit`).
- `download/` - port of `Presentation/Download/*`: `link` (DownloadLink whitelist),
  `format` (DownloadFormat, `-S res:N` sort, `--ffmpeg-location`), `run` (YtDlpRun:
  argument vector with `--` before the URL, no `--exec`, marker protocol, progress
  reading, resolution verdict, file resolution), `error` (YtDlpErrorKind/Reader/Error with
  upstream's texts), `cookies` (Netscape jar 0600 in a 0700 folder, names-only reading,
  `file_for` YouTube links only), `tool` (search), `version_check`, `item` (rows and
  queue as a state machine), `settings` accessors (`Download*` members, names or numbers).
- `update.rs` - `ModlightUpdateChecker`/`ModlightRelease`/`ModlightReleaseClient` and the
  downloader's verify step. `RELEASE_HOST = option_env!("MODLIGHT_DEFAULT_HOST")`, checked
  at compile time (https://, no trailing slash, or the build fails). Empty = updates off.

### modlight-macos

- `webview.rs` - the YouTube sign-in window: native `NSWindow` (1000x760, min 560x480,
  centred, dark/light appearance) with a `WKWebView` on a **non-persistent**
  `WKWebsiteDataStore`; navigation delegate checks the address (youtube.com, not
  /signin), waits 800 ms, reads all cookies from `WKHTTPCookieStore`, closes itself on a
  completed sign-in. Outcome polled with `take_result()`. Opens only on the main thread.

### modlight-app

- `download_tab.rs` - the Download tab (replaces the plan card in `shell.rs`).
- `components_window.rs` - the components list, drawn in the window a feature opens
  (`require`) and in Settings -> Components (`settings_page`, one line in `settings.rs`).
- `update_window.rs` - hourly check (Normal mode, release host set, `AutoCheckUpdate`),
  the "update is available" window, the downloader window; `settings_card` for the
  Application page (not placed: see "For the lead").
- `media_windows.rs` - startup (`on_start`) and per-frame (`viewports`) hooks.
- `qa_media.rs` - fixtures and dialog drawing for the QA screens.

Edits to shared files (all small, appended): `lib.rs` (modules), `state.rs` (three fields
`downloads`, `components`, `updates`), `app.rs` (`on_start` in `start`, `viewports` after
`upload_viewports`, `downloads_shutdown` in `on_exit`), `shell.rs` (the Download tab arm),
`settings.rs` (the `application.components` arm), `qa.rs` (screen list, prepare/draw/
is_dialog hooks), `dialogs/confirm.rs` (`ConfirmAction::RemoveComponent`, appended),
`myspace/actions.rs` (its arm in `confirm_finished`), `Cargo.toml`s (core: reqwest, zip,
tar, flate2, libc, tokio `process`; macos: objc2-web-kit and a few AppKit/Foundation
features).

## Tests

- core unit tests (in the modules): argument building (video/audio/quality, cookies only
  for YouTube, `--` before the URL, no `--exec`, ffmpeg location), DownloadLink, progress
  lines (invariant culture, estimates as upstream), error classification with real yt-dlp
  lines, Netscape jar writing (0600/0700) and reading, catalogue sanity, zip/tar limits
  and traversal, digest agreement rules, manifest parsing, updates off without a host,
  process-group kill takes the grandchild along.
- `modlight-core/tests/media_net.rs` (stub HTTP server on 127.0.0.1, random port): resume
  from a part file, short/long/no-length bodies, stall, refused http, wrong hash leaves
  nothing, every packaging installs and removes, the yt-dlp update end to end (API digest
  only / sums only / disagreement / non-GitHub artefact = refused; agreeing + newer =
  installed; then AlreadyCurrent; unreadable version = never automatic), the app update
  (404 = up to date, `.exe` = not for this Mac, download + verify + reuse, tampered bytes
  deleted, short body refused).
- `modlight-core/tests/download_flow.rs`: a FAKE yt-dlp shell script (records its argv,
  prints upstream-shaped markers and progress, writes a file) driven through `run::run`
  and the queue rows; the file is uploaded to 8420 through `UploadManager.upload_file`
  and deleted (skips the upload half when 8420 or `qa-accounts.env` is missing); a TikTok
  link never gets the jar; a cancelled run stops at once.
- Snapshots for the eight new screens.

## Provenance of the pinned components (checked 2026-10-20)

Full write-up with the inspection output: `/path/to/modlight/_scratch/app-3-media/provenance.md`
(artefacts in `_scratch/app-3-media/dl/`, raw API/HTML in `meta/`). Nothing downloaded was run.

| Component | URL | Version | Size (bytes) | SHA-256 | Hash published by | How verified |
|---|---|---|---|---|---|---|
| yt-dlp | https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos | 2026.08.19 (same release as the Windows pin; also /releases/latest today) | 37146048 | `0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202` | yt-dlp: `SHA2-256SUMS` (GPG-signed, key AC0CBBE6848D6A873464AF4E57CF65933B5A7581) and GitHub API `digest` | API digest == SUMS line == local hash; size == API; SUMS signature good. Universal Mach-O, ad hoc signed, links libSystem and libz only |
| ffmpeg | https://ffmpeg.martin-riedl.de/download/macos/arm64/1774549676_8.1/ffmpeg.zip | 8.1 (built 2026-03-26) | 27185234 | `cc3a7e0cce36c5eca6c17eeb93830984c657637a8e710dc98f19c8051201fa3a` | Martin Riedl: `ffmpeg.zip.sha256` at the same versioned path | local hash == published; size == Content-Length; zip has one entry `ffmpeg` (62582864 bytes, sha256 `1dda5d1b...1926`); thin arm64, only /usr/lib and system frameworks; Developer ID "Martin Riedl (KU3N25YGLU)", hardened runtime, `codesign --verify --strict` valid |
| ExifTool | https://downloads.sourceforge.net/project/exiftool/Image-ExifTool-13.59.tar.gz | 13.59 (13.29, the Windows pin, is no longer published anywhere) | 7916358 | `668ea3acececb7235fbd0f4900e72d5f12c9b07e5c778fd36cb1e9b5828fd65a` | Phil Harvey: https://exiftool.org/checksums.txt (= checksums-13.59.txt) | SHA-256, SHA-1, MD5 all match; size == Content-Length; 1254 entries under `Image-ExifTool-13.59/`, 32.6 MB, no absolute paths, `..`, links or devices; redirect chain stays https |
| Background model | https://github.com/danielgatis/rembg/releases/download/v0.0.0/bria-rmbg-2.0.onnx | bria-rmbg-2.0 (Windows pin, unchanged) | 1024331469 | `5b486f08200f513f460da46dd701db5fbb47d79b4be4b708a19444bcd4e79958` | rembg source pin `rembg/sessions/bria_rmbg.py` (main) | metadata only (1 GB not downloaded): API size and CDN Content-Length == pin, source pin == pin; GitHub `digest` is null for this asset |

Candidates rejected for ffmpeg: osxexperts.net (per-major URLs replaced in place, no hash
for 8.1, not Developer ID signed); eugeneware/ffmpeg-static (6.x only, repackaged
osxexperts, no publisher checksum). Weakness of the choice: the `.sha256` comes from the
same host as the zip. A second, independent check would be `codesign --verify --strict
-R='anchor apple generic and certificate leaf[subject.OU] = "KU3N25YGLU"'` after
extraction (not implemented: upstream has no such step).

Watch: SourceForge keeps only the last few ExifTool versions; the 13.59 URL will go 404
when 13.59 is retired. Then bump the pin (version, URL, size, SHA-256 from checksums.txt)
or mirror the exact bytes.

## macOS substitutions (for MACOS.md)

| Windows (upstream) | macOS (this port) | Notes |
|---|---|---|
| Components in `Documents\Modlight\Tools` / `Models` | `~/Library/Application Support/Modlight/Tools` / `Models` | Same staging (`.incoming`), part names, retired-file names. |
| ShareX/FFmpeg 8.1 win zip | Martin Riedl's macOS arm64 8.1 zip (one `ffmpeg`) | See provenance. |
| yt-dlp.exe / yt-dlp_arm64.exe | `yt-dlp_macos` (universal), installed as `Tools/yt-dlp`, mode 755 | Updates follow `yt-dlp_macos` of the latest release. |
| ExifTool 13.29 Windows zip (`exiftool.exe` + `exiftool_files`) | ExifTool 13.59 Perl tarball; `exiftool` + `lib/` in `Tools/ExifTool/` | Run it as `/usr/bin/perl <Tools/ExifTool/exiftool>` (the shebang is `env perl`). `installed_path("exiftool")` is the script. Apple may remove /usr/bin/perl one day. |
| Installer component page + `SkipComponents` registry value + first-run window | No installer: nothing is declined up front and the first-run window never opens | Components are offered by the features that need them and by Settings -> Components, as upstream does on later launches. |
| `ChildProcessJob` (job object, kill on close) | Process group per child, group SIGKILL on cancel/pause, `atexit` kill on exit | A crash or SIGKILL of Modlight leaves the children to finish on their own. |
| Tool search: beside exe, `Tools`, PATH | Same, plus Homebrew (`/opt/homebrew/bin`, `/usr/local/bin`), MacPorts (`/opt/local/bin`), `~/.local/bin` | An app opened from Finder inherits only the system PATH. |
| Bundled QuickJS (`Tools/qjs.exe`) | Not bundled; `Tools/qjs` beside the executable, `<profile>/Tools/qjs`, then the search above (Homebrew `quickjs`) | No trustworthy macOS QuickJS build with published checksums was picked; yt-dlp also finds deno on PATH itself. |
| WebView2 with its own profile folder for the YouTube sign-in | `WKWebView` in a native window, non-persistent data store, Safari product token in the user agent | Nothing but the jar outlives the window; "Sign in again" asks for the Google password again. No "runtime missing" state. |
| Jar protected by an owner-only ACL on folder and file | Folder 0700, file created 0600 (atomic rename) | yt-dlp rewrites the jar after a run; the folder's mode is what protects the replacement. |
| Update: Inno Setup `.exe` downloaded, verified, run elevated, app exits | `.dmg`/`.zip` downloaded, size and SHA-256 verified, **revealed in Finder**; nothing installed or run; the app keeps running | A manifest whose file is not `.dmg`/`.zip` is answered "up to date". The question uses upstream's "Would you like to download it?" wording. |
| Folder picker (`OpenFolderPickerAsync`) | `rfd` (NSOpenPanel); QA modes use `<profile>/Downloads` without a panel | |
| "Show in folder" | `open -R` (Effect::Reveal) | |

## Status rows (for STATUS.md)

| Upstream | State | Notes |
|---|---|---|
| Download tab (`DownloadView`, yt-dlp) | done | Queue with pause/resume/cancel/retry/remove, format and destination strip, folder, ffmpeg and yt-dlp notices, error wording, update-and-retry, upload through the task flow, saved notice. Not run against a real yt-dlp yet (real-window check). Snapshots `tab-download`, `tab-download-busy`, `tab-download-missing`, `download-youtube`. |
| YouTube sign-in (`YouTubeSignInForm`, `YouTubeCookies`) | partial | Implemented (WKWebView); never opened by automation; real-window check needed. |
| Components window and Settings -> Components (`ComponentsView`) | done | Snapshots `components`, `settings-components`. Real downloads not exercised in a window. |
| Components engine (`ModlightComponents`, downloads, yt-dlp updates) | done | Tested against a stub server. |
| Update check (`ModlightUpdateChecker`, fixed host) | done (off by default) | `.dmg`/`.zip` only, reveal in Finder. Snapshots `update-available`, `update-download`. |
| Settings -> Application "Updates" card | phase 3 (lead) | `update_window::settings_card` is ready; invisible without a release host. |

## For the lead

- `modlight_core::components::installed_path(profile, "ffmpeg")` for screen recording;
  when it is `None`, open the window with
  `crate::components_window::require(state, components::FFMPEG, s::Components_FeatureRecording, then)`.
  `AfterClose` today has only Download-tab continuations; append a variant for recording
  and handle it in `download_tab::after_components` (or move that dispatcher).
- `update_window::settings_card(ui, state, colors)` belongs in the Application page's
  "Updates" card (upstream `UpdatesVisible = HasReleaseChannel`).
- About window: `modlight_core::update::check()` gives the same answer the timer gets.
- `docs/MACOS.md` / `docs/STATUS.md` were not edited: fold in the two tables above.
- Server gap (not touched): `GET /api/releases/app` publishes ONE release. A Mac build
  will only ever be offered when the published file is a `.dmg`/`.zip`, which then hides
  the Windows installer from Windows clients. A per-platform manifest is a server change.

## Check in a real window with the owner (and what macOS may ask)

1. Download tab, Install yt-dlp from the notice (37 MB from github.com): the components
   window opens with the headline, progress, "Checking the file against its checksum",
   Installed. Then a YouTube, a TikTok and an X link with Disk / Modlight / Both; pause,
   resume (the .part continues), cancel, remove while running (yt-dlp and ffmpeg gone in
   Activity Monitor), Clear finished, Show in folder, the saved notice's Open / Open folder
   / Upload, the upload notice's Copy link. Prompts: none expected; writing into
   ~/Downloads, ~/Movies or ~/Desktop is allowed because the folder was picked in the
   open panel (if a folder is typed into the settings file instead, macOS may ask
   "Modlight would like to access files in your Downloads folder").
2. "Choose folder" (NSOpenPanel): no permission prompt.
3. Install FFmpeg and ExifTool from Settings -> Components; Remove (the Yes/No question);
   "Check now"; Update on yt-dlp; untick/tick; "Install what is missing". The 1 GB model is
   optional. After an hour-old launch, the 45 s scheduled yt-dlp check (log line
   "scheduled check for yt-dlp says ...").
4. YouTube sign-in (Download tab -> YouTube config -> Sign in): a 1000x760 window with
   Google's page; after signing in it closes by itself, the dot turns green,
   `~/Library/Application Support/Modlight/YouTubeSignIn/cookies.txt` is 0600 in a 0700
   folder; an age-gated video then downloads; Sign out deletes the jar. Prompts: none
   expected from macOS (WebKit, no Keychain: the data store is non-persistent). Google
   may still refuse an embedded browser ("This browser or app may not be secure") - if
   so, that is the thing to report back.
5. Quitting with a download running (Cmd+Q and tray Exit): no yt-dlp or ffmpeg left.
6. Update (needs a build with `MODLIGHT_DEFAULT_HOST=https://...` and a `.dmg` published
   on that server): the question at launch, Yes -> the downloader -> Finder shows the
   verified file. No prompt expected (Finder reveal needs none).

## Remaining differences from upstream

- yt-dlp self-update requires BOTH published digests (upstream accepted either one with
  a log note); an unreachable checksum file means no update (audit's rule).
- App update: size required, `.dmg`/`.zip` only, reveal instead of install (above).
- `DownloadLink` also refuses addresses with user:password (hardening).
- Network errors also recognise macOS's getaddrinfo text ("nodename nor servname provided").
- Components window and settings page share one row state (upstream had one view model
  per host); closing the window still stops only what the window started.
- `Require` is not blocking: the window opens and the feature continues when it closes.
- Remove asks with the existing Yes/No box (warning glyph; upstream's Question glyph).
- Proxy settings are not applied to component or update downloads (as for uploads).
- The empty-queue card never rises above the settings strip when the window is short.
- The quality picker's tooltip sits on the picker's area (egui has no per-ComboBox tip).
- No "youtube-sign-in" QA screen: the window is native AppKit + WebKit with no egui
  chrome to snapshot; the panel that opens it is `download-youtube`.
- `--windows-filenames` is still passed to yt-dlp (upstream's argument vector, kept 1:1;
  names stay portable to Windows).

## Gotchas

- Hand-placed buttons go through a child `Ui` (`button_here` in each module):
  `allocate_rect` on the list's own `Ui` moves its cursor and the next card overlaps.
- QA renders must not read the disk for what is installed: `Components.qa_installed` and
  `Components.display_profile`, `download_tab::qa_fill` set the facts.
- The yt-dlp update test switches the GitHub API origin process-wide
  (`net::testing::set_github_api`); keep such tests in one test function.
- `cookies::YouTubeCookies::is_signed_in` caches against path, length and mtime.
- `components::install` has a global one-at-a-time gate; tests that install in parallel
  queue behind each other (fine, just slower).

## State

All work committed on `app-3-media`; last commit listed in the final report. No
processes left running. Scratch: `/path/to/modlight/_scratch/app-3-media/`
(provenance, downloaded artefacts for hashing, renders).
