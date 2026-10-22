# Security audit of the Modlight-App client

Date: 2026-10-18. Static analysis only (reading, grep, file, xxd, git, llvm-objdump). Nothing from the repository was run.

## Scope and method

- Copy of the client: `/Users/marcinasdfg/Desktop/modlight/upstream/Modlight-App` (a single commit `b178a09` by the author Nimfek, 2026-10-18).
- The WHOLE client was examined (all projects), not only `ShareX/Modlight/*`.
- The client is a fork of ShareX. To tell the author's changes apart from upstream code, I cloned the original ShareX into `/Users/marcinasdfg/Desktop/modlight/_audyt/upstream-sharex` and determined the base commit: **`9c81fb01` ("Add configurable built-in toolbars for editor hosting", 2026-08-17)**. It is the ShareX commit with the fewest differences in shared files (the editor/effects/capture libraries are practically unchanged apart from renaming). All verdicts below are based on a comparison of the fork against that commit.
- Diff statistics: 164 files added, 726 removed, 151 changed. The author removed the whole ShareX "uploaders" subsystem (dozens of services with API keys, OAuth, FTP/SSH, the native browser messaging host, Steam, Microsoft Store) and replaced it with a single Modlight server.

## MAIN VERDICT: does the application exfiltrate data

**No. I found no backdoor, no telemetry and no sending of data to a third party.** The code is clearly written defensively (token only in a header, an allow-list of fields in sync, pinned SHA-256 digests, host validation). The detailed verdict and the full list of connections are in the section "Does the application exfiltrate data". There are no findings of Critical or High severity. The findings are mostly notes for the Rust port and a few Medium/Low minor items.

---

## What the application does (to be rewritten in Rust)

A Windows desktop client for a self-hosted Modlight server (Next.js + PostgreSQL). Sign-in goes through the browser (RFC 8252 loopback), after which every screenshot, recording or file lands in the account's library.

### Windows (ShareX/Presentation/**)
- **MySpace**: the library: folders and files as tiles, animated video/GIF preview, a details drawer, drag and drop, trash.
- **SignIn**: sign-in: a server address field + sign-in through the browser; as a fallback, pasting a token from the panel.
- **Download**: downloading through yt-dlp (YouTube/TikTok/X), optional YouTube sign-in in WebView2.
- **Admin**: a view of the server's disk (admin role only).
- **Components**: downloading ffmpeg/yt-dlp/the ONNX model/ExifTool.
- **Settings / AccountSettings**, **Upload (Paste)**, **Notification** (toasts), **FirstTimeConfig**, plus the task settings, hotkey, image editor, effects and capture windows carried over from ShareX.

### Server endpoints used by the client (method, path, purpose)
All go to the account's host (`account.Host`), with the token in the `x-api-key` header:
- `GET /api/me`: who the token belongs to (tag, name, e-mail, avatar, link host, quota, role) (`ModlightApi.cs:191`)
- `POST /upload`: plain upload (file/image/text), multipart `upload`, optional `folderId` (`ShareX.UploadersLib/FileUploaders/Modlight.cs:125`)
- `POST /api/files` (tus 1.0.0), `HEAD`, `PATCH`, `DELETE`: resumable offline upload (`ModlightResumableUpload.cs:98`)
- `GET/PUT /api/client-settings?client=desktop`: settings sync (a versioned blob) (`ModlightSettingsSync.cs:574`)
- `GET/PUT /api/theme`: skin sync (`ModlightTheme.cs:53,61`)
- `GET /api/folders`, `POST/PATCH/DELETE /api/folders`: folders (`ModlightLibrary.cs`)
- `GET /api/uploads`, `PATCH/DELETE /api/uploads`: list/edit/trash files (`ModlightLibrary.cs`)
- `GET /api/duplicates`, `POST /api/duplicates`: duplicates (`ModlightDuplicates.cs`)
- `GET /api/admin/disk`: disk statistics (admin) (`ModlightServerDisk.cs:346`)
- `POST /api/shortlinks`: link shortening (`ModlightURLShortener.cs:105`)
- `GET /api/releases/app`: update manifest (see below; a FIXED host, not the account's) (`ModlightReleaseClient.cs:94`)
- `GET {host}/authorize?port=&name=&state=`: opened in the browser during sign-in (`ModlightSignIn.cs:140`)
- `GET .../download`, `.../convert`, thumbnails: file download, conversion and thumbnails.

### External hosts (other than the account's server)
- `github.com` / `api.github.com`: downloading components (ffmpeg, yt-dlp, the model) and yt-dlp auto-update (pinned addresses).
- `accounts.google.com` / `youtube.com`: only in the YouTube sign-in window in WebView2 (started manually).
- `api.openai.com` / `generativelanguage.googleapis.com` / `openrouter.ai`: only the "Analyze Image" tool, and only when the user enters a key and uses the feature.
- the update host = `ModlightDefaultHost` (EMPTY in the code; set when a distribution is built).

---

## Does the application exfiltrate data (full list, evidence)

### Outgoing connections: where to, what, when, whether the user decides

| Destination | What is sent | When | User's decision |
|---|---|---|---|
| `account.Host` (the account's server) | token in `x-api-key`, files, names, folder metadata, settings/skin blob | upload, library browsing, sync | deliberate sign-in; upload according to the task settings |
| update host (fixed, build-time) | only `GET /api/releases/app` + download of the installer | at start + manually | installation only with consent and a matching SHA-256 |
| `github.com`, `api.github.com` | plain GETs (no user data) | downloading components, yt-dlp auto-update every 24 h | components chosen in the installer/settings |
| Google/YouTube (WebView2) | the user's Google sign-in | only when they click "sign in to YouTube" | fully deliberate |
| OpenAI/Gemini/OpenRouter | image + the user's key | only "Analyze Image" with the user's own key | fully deliberate |

Evidence that nothing leaks:
- The **token** goes only into the `x-api-key` header on `account.Host`; never into a URL (so that it does not end up in proxy logs). Confirmed across all `Modlight*` files. `ModlightSignIn.Save` (lines 419-451) and `Adopt` (359-385) do not overwrite the host/token with the server's reply; the comment says it outright: "letting a reply rewrite the credential it was authorised by is how a redirect becomes an account takeover".
- **Settings sync is an allow-list** (`ModlightSettingsDocument.cs:96-208`). It does not include: the token, the AI keys (`OpenAIAPIKey/GeminiAPIKey/OpenRouterAPIKey`), the proxy password, local paths, window positions, the printer name, or the YouTube cookies (they are not in the config at all). Explicitly excluded in `NormalizeTaskSettings` (it removes `AIOptions`, `CLIPath` and paths).
- **YouTube cookies** (`YouTubeCookies.cs`): the file `YouTubeSignIn/cookies.txt` in the profile folder, protected by a DPAPI ACL (`Prepare`/`ProtectFile`), handed ONLY to yt-dlp via `--cookies` and ONLY for YouTube links (`FileFor` returns null for non-YouTube). It is never uploaded, never goes into the config and is never logged (only the number of cookies is logged). `NetscapeCookieFile.cs` is pure file I/O, with no network. A dedicated profile instance of WebView2 is used (its own `userDataFolder`), NOT the user's Chrome cookie store, NOT `--cookies-from-browser`.
- **No telemetry/analytics**: grep found no Sentry/AppCenter/GA/Mixpanel/Plausible or similar.
- **No disabled TLS validation**: there is no `ServerCertificateCustomValidationCallback`/`DangerousAccept...`. `HttpClientFactory` and the custom handlers (`ModlightComponentDownload`, `ModlightResumableUpload`) use the default validation; `ResumableUpload` additionally sets `AllowAutoRedirect=false` (it does not pass the token through a redirect) and `ResolveLocation` enforces its own host/scheme.
- **No ShareX update check**: the old GitHub channel `ShareX/ShareX` and the AppVeyor dev build were cut off (`ShareXUpdateManager.cs`: a comment explains that with version 0.7.x every start would see an "update" and install the ShareX installer). `getsharex.com` remains only as a web/docs link (`Links.cs:30`, `BackgroundRemoverViewModel.cs:39`), not as a source of code.

### Access to private data (clipboard, keyboard, files, browser)
- **Clipboard, reading**: only on demand: the "Paste" preview pre-fills its content (`PasteWindow.axaml.cs:65 ClipboardHelpers.GetText`) and there is a manual "Clipboard upload". **There is no clipboard monitor** (grep: no `AddClipboardFormatListener`/`WM_CLIPBOARDUPDATE`). Writing: copying links/images.
- **Keyboard**: global hotkeys through `RegisterHotKey` (not a keylogger). `KeyboardHook` (WH_KEYBOARD_LL) exists in `ShareX.HelpersLib/Input/KeyboardHook.cs` as a leftover from ShareX, but **it is never instantiated** (grep for `new KeyboardHook` finds nothing). Do not carry it over to the Rust port.
- **Files/browsers**: no reading of browser files, history or password stores. WatchFolder and AutoCapture work only after deliberate configuration by the user (`WatchFolderEnabled`, a manual AutoCapture window with a timer); they are not hidden.

---

## Findings (severity, file:line, scenario, confidence, fix)

### [Medium] Sign-in loopback: ACAO:* + Allow-Private-Network; possible sign-in DoS (NOT token injection)
`ShareX/Modlight/ModlightLoopbackListener.cs:172-178`: on an OPTIONS preflight it answers `Access-Control-Allow-Origin: *`, `Access-Control-Allow-Private-Network: true`. It listens on a random port on `127.0.0.1` (`:81`) and accepts the first valid POST.
- **Injection of an attacker's token is NOT possible.** `ModlightSignIn.cs:138` generates `state` = 32 characters from `RandomCrypto` (CSPRNG, `RandomCrypto.cs:36 RandomNumberGenerator.Create`), and `NonceEquals` (`ModlightSignIn.cs:402-417`) compares it in constant time; on a mismatch the sign-in is **aborted** (StateMismatch), not continued. A foreign page or process does not know `state`, so it cannot supply an acceptable body.
- **Real risk (Low/DoS)**: any local process can connect to the port and send a POST with a wrong `state` before the browser does; the listener returns that first body, and SignIn rejects it and ENDS the sign-in, so signing in can be prevented (the account cannot be taken over). The window is 10 minutes.
- **Scenario**: a malicious web page open during sign-in does a `fetch` to `127.0.0.1:port`; without `state` all it achieves is aborting the sign-in.
- **Confidence: confirmed** (the whole path listener -> SignIn was read).
- **Fix in the Rust port**: keep the mandatory, constant-time `state` check, the single shot and the random port. Consider narrowing ACAO to the server's origin instead of `*` and not leaving the loop on a wrong `state` (resistance to DoS). Do not base security on CORS alone.

### [Low] `FileDownloader` treats a download without Content-Length as complete
`ShareX.HelpersLib/FileDownloader.cs` (a change in the fork, ~138-175): returns success when `DownloadedSize >= FileSize`; with no `Content-Length`, `FileSize=0` and any download (even an empty one) counts as "complete".
- In the released client only the updater calls this path, and it verifies `ExpectedSha256` before running anything, so a mismatching file is rejected (`DownloaderWindow.VerifyDownloadAsync`). The build-time tool `ShareX.Setup/Program.cs` (downloading ffmpeg/exiftool) does not check the hash, but it is not shipped to users.
- **Confidence: confirmed.** **Fix in the Rust port**: do not treat a download as successful without a known size/hash; every downloaded executable must have its SHA-256 verified.

### [Low] The yt-dlp auto-update downloads and replaces an executable without interaction
`ShareX/Modlight/ModlightComponentUpdates.cs`: every 24 h (`CheckInterval`), enabled by default (`ApplicationConfig.ExternalComponentsAutoUpdate = true`), it downloads the latest `yt-dlp.exe` from `yt-dlp/yt-dlp releases/latest` and replaces the existing one.
- The safeguards are solid: `IsArtefactUrl` enforces `https` and the host exactly `github.com` (`ModlightToolUpdate.cs`), the digest is taken from the GitHub API `digest` field and/or the `SHA2-256SUMS` file, and when both are present they must agree (`WithPublishedDigest`), otherwise it refuses. yt-dlp is run afterwards, so this is a "download and execute" path.
- **Confidence: confirmed.** This is a deliberate trade-off (yt-dlp goes stale within weeks). **Fix in the Rust port**: keep the requirement that two digest sources agree and the pin to `github.com+https`; consider making it visible that auto-update is on by default.

### [Low] ExifTool: SHA-256 without independent publication
`ShareX/Modlight/ModlightComponent.cs:287`: the ExifTool digest is the author's own hash; the comment (`:157-161`) honestly notes that the `ShareX/ExifTool` release does not publish a checksum. I checked: the `ShareX/ExifTool` repository does not exist today / is unavailable through the API (404). The other three digests I **verified and they match**: ffmpeg x64 `1838932e...` and arm64 `7ac6c5db...`, and yt-dlp x64 `66674953...` and arm64 `05b43899...` match the `digest` field of the GitHub releases; the rembg model `5b486f08...` matches the value in the rembg source (`rembg/sessions/bria_rmbg.py`). **Fix in the Rust port**: without a publication, keep the pin as a constant in the code and do not add "latest" tracking for ExifTool.

### [Info] Dead code left over from ShareX (do not carry over)
- `TaskHelpers.DownloadAppVeyorBuild()` (`:2298`) and `AppVeyorUpdateChecker`/`GitHubUpdateChecker`: no call anywhere in the fork (grep confirms there is no caller).
- `KeyboardHook` (described above): unused.
- `Links.Website = https://getsharex.com`: opened in About/menu; cosmetic, not a leak.

---

## Logic errors

I reviewed the upload queue, settings sync and downloading. The code is unusually carefully written with respect to exactly these errors; below is what I found, mostly confirmation that errors are absent, plus one remark.

1. **Upload to the wrong server after an account change: does NOT occur.** `ModlightUploadQueue.Matches` (`:1066`) compares the host (TrimEnd '/') and the tag; a queue item for server A is skipped after signing in to B (`AccountChanged`), with a 14-day deadline, and NOT sent to B. `Verify` (`:653`) checks that the source file exists and has the same size/modification time, otherwise it starts over. Confidence: confirmed.
2. **Loss/double send: guarded against.** An item holds its claim until the result is stored (comment `:588-594`), so a Cancel right after success is a no-op (it does not create a second "cancelled" for an already uploaded file). The queue is written atomically (tmp + `File.Move` over, `ModlightQueueStore.Save`). `IsWorthQueuing` (`:974`) queues only a missing response and 408/429/502/503/504; 507 (no space) is reported at once, not in a loop. Confidence: confirmed.
3. **Fresh-install guard in sync** (`ModlightSettingsPlan.cs:172`): an installation without a confirmed marker must NOT push its settings: the cloud wins, so a fresh install will not overwrite hotkeys built up over years. The marker is not synced (rightly). Unknown schema -> it does nothing. Confidence: confirmed, the logic is sound.
4. **(Low) `FileDownloader`**: described above; the only real logic error (success when Content-Length is missing), neutralised by the updater's SHA verification.
5. **Trash instead of permanent deletion**: all delete actions in the UI (MySpace, duplicates) call the `"trash"` mode (`MainWindow.MySpace.Actions.cs:595`, `DuplicatesViewModel.cs:579`, `ModlightLibrary.TrashAsync:652`). Permanent deletion is not wired to the UI. The default selection in duplicates keeps the oldest copy (`IsSelected = !IsOldest`). Confidence: confirmed, safe.
6. **Argument injection into yt-dlp/ffmpeg from a URL: does NOT occur.** `YtDlpRun` builds arguments as a list (`ArgumentList`, not a string) and ends with `--` before the URL (`:505-506`); the URL goes through the allow-list `DownloadLink.TryParse` (only absolute http/https with a host, `:63-100`). No `--exec`. yt-dlp gets `--ignore-config` only when queried for its version; when downloading, the user's config could add something, but that is the user's local file, not network input. QuickJS is passed by full path. Confidence: confirmed.

---

## Code execution at build/install time

- **Directory.build.props / .targets / Directory.Packages.props**: no `Exec`, no downloads. `ModlightDefaultHost` is EMPTY in the source (the only `https://files.example.com` is an example in a comment). The `.targets` are identical to upstream. The NuGet packages are standard (Avalonia 12.1.1, Newtonsoft 13.0.4, SkiaSharp, ZXing, Microsoft.Web.WebView2 1.0.4129.50, ML.OnnxRuntime.DirectML, Vortice); `FluentFTP` and `SSH.NET` were removed. There is no `nuget.config` (the source = the default nuget.org). Central versioning. No unusual or impersonating packages.
- **ShareX.UploadersLib.csproj**: `Target ModlightBuildInfo` (`:29-43`) generates the constant `ModlightBuildInfo.DefaultHost` at compile time from `ModlightDefaultHost`, validating `https://` and no trailing `/`. Harmless (WriteLinesToFile).
- **.github/workflows/build.yml**: `windows-latest`, `dotnet build ShareX/ShareX.csproj -c Release` + `dotnet run --project ShareX.Tests`. Permissions `contents: read`. No secrets, no artifact publishing, no pulling of scripts.
- **ShareX.Setup/Program.cs**: a build-time tool: downloads ffmpeg/exiftool from `github.com/ShareX/...` (without checking SHA: build-time, not on the user's machine), calls a local ISCC.exe. No surprises.
- **InnoSetup/Modlight-setup.iss**: a separate `AppId`/`AppMutex` (GUID `3030612E-...`, != ShareX `82E6AC09-...`), so it does not take over ShareX's entry. The registry is touched only under `Software\Classes\...\Modlight`, `Software\Modlight\Setup` (SkipComponents) and `Control Panel\Keyboard` PrintScreen (an optional task). The ShareX import copies **only** `ApplicationConfig.json` and `HotkeysConfig.json`, deliberately NOT `PersonalPath.cfg`, NOT `History.db`, NOT `UploadersConfig.json` (comment `:200-226`: the last one contains API keys of removed services; copying it would carry secrets over). `taskkill` on ShareX without `/f` (a polite close). This fixes real pitfalls and introduces no new ones.
- **Tools/generate-skins.py**, **ValidateTranslations.ps1**, **Generate-LucideIcons.ps1**: developer tools. The Python imports only the stdlib (argparse/io/json/math/os/re/sys), reads the panel's files and writes `Skins.axaml`/`SkinVariants.g.cs`; no subprocess/eval/exec/network. The PS1 files have no `Invoke-WebRequest/Expression`, `DownloadString` or registry access. They are not part of the client build.
- **.resx**: no serialized binary object other than the standard ones (WinForms icons/bitmaps in the editor's `*.resx`, unchanged from upstream; in the changed files the differences are only `ShareX.HelpersLib`->`Modlight.HelpersLib` in type names and title texts). `KnownTypesSerializationBinder` restricts effect deserialization to subclasses of `ImageEffect`; no `TypeNameHandling` is set on data from the network.
- **Binaries in the repository**:
  - `Tools/quickjs/qjs.exe` (SHA-256 `433a35a5...`) and `libwinpthread-1.dll` (`1933c3f0...`): PE32+ x86-64, MinGW/Fedora, "QuickJS version 2026-06-04". Import table (llvm-objdump): `qjs.exe` imports only KERNEL32, msvcrt, and `MessageBoxW` from USER32, plus pthreads from libwinpthread; **no ws2_32/wininet/winhttp** (no networking at all). `libwinpthread` imports only KERNEL32/msvcrt. Statically there is no red flag; a full byte-level analysis cannot be done statically, but the import table is clean. The remaining binaries (.ico icons, .wav, .cur, the .ttf lucide, the wizard .bmp, .webp screenshots, docs/logo.svg) are resources: no .dll/.exe masquerades as another file.
  - `ShareX.Setup/InnoSetup/puush`: an **empty file (0 bytes)**, inherited from upstream (where `-puush` was puush.me branding). In the fork the puush mode was removed entirely from `Program.cs`/`MainForm.cs`, and the file is no longer listed in the `.iss`. Harmless, to be deleted.

---

## Notes for the Rust rewrite (what to watch for / not to carry over)

1. Keep: the token only in the `x-api-key` header; the update host FIXED (build-time), never taken from the sign-in field; SHA-256 verification of the installer and components before running them; an allow-list of fields in sync (no secrets or paths); YouTube cookies only local and only for yt-dlp; subprocess arguments as a list + `--`.
2. Loopback: a mandatory constant-time `state` from a CSPRNG, a single shot, a random port; narrow CORS and do not abort listening on a wrong `state`.
3. Do not carry over: `KeyboardHook`, `DownloadAppVeyorBuild`/AppVeyor/GitHubUpdateChecker, puush leftovers, `getsharex.com` links.
4. Downloading: never treat a download as successful without a known size/hash.
5. The token is stored encrypted with DPAPI (`[JsonEncrypt]` + `DPAPIEncryptedStringPropertyResolver`); on the Mac the equivalent is the Keychain; do not store the token/AI keys as plain text.
