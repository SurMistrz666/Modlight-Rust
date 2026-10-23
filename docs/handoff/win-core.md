# HANDOFF: win-core (the non-GUI crates on Windows)

Worktree `D:\WORK\_modlight_wt\core`, branch `win-core`, from `7ac27b8` ("Windows: build the
workspace on Windows") plus the coordinator's `169cc06` (tiny-skia overflow checks,
cherry-picked). Never pushed. Everything under `app/` except this file.

Rule followed everywhere: on Windows the port does what upstream (the C# app, which is a
Windows program) does; macOS keeps exactly what it did (`cfg(not(windows))` branches,
macOS tests gated with `cfg` rather than changed, Windows tests beside them).

## Build and verify (Windows, Git Bash)

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd app
cargo fmt --all --check
cargo clippy --workspace --exclude modlight-macos --all-targets -- -D warnings
cargo test -p modlight-core -p modlight-tools -p modlight-editor -p modlight-effects
```

`modlight-macos` fails clippy on Windows at the base already (`needless_return` in
`src/keys.rs`, its Windows stub); it is not this branch's file.

## What changed, by topic

### Personal folder (`modlight_core::folders`)
- `modlight_core::personal_folder() -> PathBuf`: `Documents\Modlight` on Windows (the
  shell's `FOLDERID_Documents`, so a moved or OneDrive Documents is followed, as .NET's
  `GetFolderPath(MyDocuments)`), `~/Library/Application Support/Modlight` on macOS (same as
  `main.rs` computes today). Upstream's other ways to move it (`-portable`, `Portable`,
  `PersonalPath.cfg`, registry) are not ported: fixed on both systems.
- `folders::known` (crate-private): `SHGetKnownFolderPath`, `GetSystemDirectoryW`.

### FileHelpers and System.IO.Path (`names/file_helpers.rs`, `names/name_parser.rs`)
- Special folders on Windows are .NET's own mapping (`Environment.Windows.cs`, checked
  against dotnet/runtime): every `SpecialFolder` to its `FOLDERID_*`, .NET's environment
  fallbacks (`APPDATA`, `LOCALAPPDATA`, `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`,
  `CommonProgramFiles`, `CommonProgramFiles(x86)`, `USERPROFILE`, `windir`), `System` =
  `GetSystemDirectoryW`, `MyComputer` and `PrinterShortcuts` (.NET maps it to the virtual
  printers folder) expand to nothing. A folder is looked up only when its `%Name%` is in the
  path.
- `ExpandEnvironmentVariables` on Windows is `ExpandEnvironmentStringsW` (names without
  case).
- Invalid file name characters are .NET's on Windows (`"<>|`, NUL, 1-31, `:*?\/`);
  `SanitizePath` keeps both separators and the drive root.
- `path` on Windows: both separators, `GetRootLength` (drive, `\`, UNC, `\\?\`, `\\.\`),
  `GetFileName`, `GetExtension`, `GetDirectoryName` (with `NormalizeDirectorySeparators`),
  `Combine`, `GetFullPath` = `GetFullPathNameW` (`std::path::absolute`).
- `File.Exists` reads the entry's own attributes (no link following), as .NET on Windows.
- Name codes on Windows: `%cn` = `GetComputerNameW`, `%un`/`%uln` = halves of
  `GetUserNameExW(NameSamCompatible)`, `%n` = CRLF in free text.

### Components (`components/*`, `download/tool.rs`)
- Windows catalogue = upstream's `ModlightComponentCatalog` for the build's `target_arch`
  (provenance below). ExifTool is the one exception: upstream's artefact is gone.
- New packaging `ZipFolder { keep: &[(inside, as)] }`: a zip with one top-level folder whose
  named files/folders are taken out under new names (ExifTool: `exiftool_files` and
  `exiftool(-k).exe` -> `exiftool.exe`; the "(-k)" in the name makes ExifTool wait for a key
  press). Same staging, space check, rename-aside replacement and size ceiling as the rest.
- Unpacking refuses, on Windows, names with a colon (drive, alternate stream) and DOS device
  names (`CON`, `NUL`, `COM1`...), which upstream's `GetFullPath` check also refuses.
- Tool search on Windows: `yt-dlp.exe`, `ffmpeg.exe`, `qjs.exe` (constants
  `download::tool::{YT_DLP_FILE_NAME, FFMPEG_FILE_NAME, QUICKJS_FILE_NAME}`), beside the exe,
  `<profile>\Tools`, PATH; an existing file counts; no Homebrew folders. A relative PATH
  entry is still skipped (upstream would look under the working folder).
- `chmod` is a no-op on Windows (already); the version probe and every child start without
  a console (`process::command`, `process::no_window`).
- The YouTube cookie jar and its folder get upstream's owner-only ACL on Windows
  (`SetNamedSecurityInfoW`: owner = user, one full-control ACE inherited by children,
  protected DACL), best effort with a log line, as upstream.

### ExifTool (`modlight-tools/src/metadata.rs`)
- Windows runs `exiftool.exe` directly (no Perl) with upstream's arguments (`-G -t -m -q`,
  `-all= -overwrite_original -q`), `CREATE_NO_WINDOW` (upstream's `CreateNoWindow = true`).

### Recording (`recording/*`, `modlight-app/src/ffmpeg_window.rs`)
- The screen still reaches ffmpeg as the platform's BGRA frames on stdin (GDI frames on
  Windows). Cameras and microphones on Windows are `-f dshow` with `AppendInputDevice`'s
  `-thread_queue_size 1024 -rtbufsize 256M` (+ `-audio_buffer_size 80` for audio), inputs
  `audio=<mic>`, `video=<cam>[:audio=<mic>]`, each one argument.
- Device listing: `-list_devices true -f dshow -i dummy`, read like `GetDirectShowDevices`
  (headers of old ffmpeg builds, the "(video)"/"(audio)" suffix of new ones, alternative
  names skipped). On Windows any output is a (possibly empty) list, as upstream.
- Encoders on Windows: every codec in enum order; NVENC `-preset -tune -b:v -movflags`,
  AMF `-usage -quality -b:v -pix_fmt yuv420p`, QSV `-preset -b:v`, as `GetFFmpegArgs`.
- `FixSources` on Windows changes only "GDI grab" (as upstream); `ddagrab` and
  `screen-capture-recorder` keep their values and record the screen.
- The options window on Windows: upstream's texts (DirectShow counts, "Path to ffmpeg.exe",
  hardware hints), `dshow (name)` titles, one screen entry with upstream's GDI title, the
  NVENC/AMF/QSV panels (preset, tune, usage, quality), Program Files as the browse start.
  "Get recorder devices" (the screen-capture-recorder installer) is not ported: the screen
  does not go through a DirectShow filter here.
- ffmpeg itself starts without a console window.

### Video converter (`modlight-tools/src/video.rs`)
- Windows offers upstream's 15 encoders with upstream's NVENC/AMF/QSV arguments; macOS keeps
  the VideoToolbox mapping.

### Hotkeys (`hotkeys.rs`, `modlight-app/src/hotkey_page.rs`, editor toolbar)
- Windows: Ctrl/Shift/Alt/Win (upstream's localized names), Print Screen, Scroll Lock (its
  localized text), Pause are themselves, F13 is F13, defaults are `GetDefaultHotkeyList`
  (Ctrl+PrtSc region, PrtSc full screen, Alt+PrtSc window, Shift+PrtSc recording,
  Ctrl+Shift+PrtSc GIF). `gesture(key, control, shift, alt, win)` takes the modifiers by
  their `Keys` meaning.
- The editor shows stored shortcuts as written ("Ctrl+Shift+S") on Windows.

### Texts
- New table `assets/windows/l10n/Windows.resx` (+ `.pl`) -> `modlight_core::l10n::windows`,
  for port-only texts whose Mac wording names macOS: `PersonalFolderFixed`,
  `BackupImportRefused`, `BackupImportDisabled` ("this PC"), `BackupImportDefaultTask`,
  `BackupImportProgram`, `BackupImportFFmpegPath`. The backup report uses it on Windows.

### Settings backup (`backup.rs`)
- Same import safety rules; the token is in Credential Manager on Windows, so no
  `UploadersConfig.json` in a backup either. Names compared exactly (Windows-style hostile
  names added to the tests).

### Everything else
- `update.rs`: a Windows release is the installer `.exe` or a `.zip` (a `.dmg` is not), the
  fallback name `Modlight-setup.exe`; verified and revealed, never run. The server publishes
  one release, so a `.zip` reaches both systems: a per-platform manifest is a server change.
- Folder uploads on Windows send every file (upstream's `Directory.GetFiles`); only a Mac
  leaves `.DS_Store` out.
- "Perform actions" on Windows: `CreateNoWindow = HiddenWindow`; an output differing from
  the input only in case is the input (`OrdinalIgnoreCase`), never deleted as the input.
- Upload spool: on Windows its own files were never deleted (a drive prefix failed the
  path check); fixed, and an id with a colon is refused.
- Watch folders on Windows: upstream's "locked" test (open with no sharing).
- Captures on Windows always go through the platform's `capture_window` (re-reads the
  rectangle, honours "capture client area"); the desktop's process is "explorer".
- Image effects' `TryGetSafeLocalPath` on Windows: local `file:` URIs on a drive only, no
  UNC/device path (also after `GetFullPathNameW`), relative paths from the exe's folder, no
  reparse point (link, junction, mount point) on the way; package extraction treats any
  reparse point as a link.
- Directory indexer on Windows: CRLF, hidden = the hidden attribute, path display trimmed at
  `\`.
- Editor emoji on Windows: Segoe UI Emoji (COLR v1) painted by a new COLR painter on
  tiny-skia (`modlight_editor::colr`); flags show their letters, as Windows does.
- Editor snapshot tests: Windows-only `windows-<file>.png` pictures beside the shared ones
  (33 pictures: 8 scenes x 4 looks + the export gallery), the app's rule.

## Provenance of the Windows components (checked 2026-10-22)

Downloaded with curl into a scratch folder; nothing was run. PE machine types read from the
headers (Python, `zipfile`); zips listed without extracting.

| Component | URL | Size (bytes) | SHA-256 | How verified |
|---|---|---|---|---|
| FFmpeg 8.1 x64 | https://github.com/ShareX/FFmpeg/releases/download/v8.1/ffmpeg-8.1-win-x64.zip | 68599421 | `1838932e32f01e0e7b65fde5ca0ebb2ab131cb6a9ff1a84025ee5b784075576c` | = upstream's pin = GitHub API `digest` (release v8.1, 2026-05-20); one entry `ffmpeg.exe`, 202259968 bytes, PE x64 |
| FFmpeg 8.1 arm64 | https://github.com/ShareX/FFmpeg/releases/download/v8.1/ffmpeg-8.1-win-arm64.zip | 50657497 | `7ac6c5dbbe61037bcce97482549294238d87400d8b7b1cb559949293c3b06dd5` | = upstream's pin = API digest; one entry `ffmpeg.exe`, 133895168 bytes, PE arm64 |
| yt-dlp 2026.08.19 x64 | https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe | 17840399 | `66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a` | = upstream's pin = API digest = `SHA2-256SUMS` line; SUMS GPG signature good (key AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581, temporary keyring); PE x64 |
| yt-dlp 2026.08.19 arm64 | https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_arm64.exe | 21159286 | `05b438997bafc3affdfda9d041353c9d73e04dc842207254b655b0887c4445b0` | same checks; PE arm64 |
| ExifTool 13.59 (win64, both architectures) | https://downloads.sourceforge.net/project/exiftool/exiftool-13.59_64.zip | 11183675 | `44b512b25af500724ba579d0a53c8fc5851628b692dd5e5d94ae4a15c2cba9ec` | Phil Harvey's https://exiftool.org/checksums.txt: SHA-256, SHA-1 `022b902bb3d01b171d5108e2c4be59491eb64444` and MD5 `df4fa1b736f0b8d39bd09b15fbdb865c` match; size = Content-Length; redirects stay https (SourceForge mirror); 629 entries under `exiftool-13.59_64/`, 34532543 bytes unpacked, no absolute names, `..`, links; `exiftool(-k).exe` 58368 bytes PE x64, `exiftool_files/perl.exe` PE x64 |
| Background model | https://github.com/danielgatis/rembg/releases/download/v0.0.0/bria-rmbg-2.0.onnx | 1024331469 | `5b486f08200f513f460da46dd701db5fbb47d79b4be4b708a19444bcd4e79958` | unchanged pin; metadata only: HEAD redirects to release-assets.githubusercontent.com, Content-Length = pin |

Upstream's ExifTool pin, `https://github.com/ShareX/ExifTool/releases/download/v13.29/exiftool-13.29-win64.zip`
(`ae8e1fc9...e1cfb6`, 10146032 bytes), is gone: the download answers 404 and
`api.github.com/repos/ShareX/ExifTool` answers 404. 13.59 is the version the macOS pin uses.
Watch: SourceForge keeps only recent ExifTool versions; when 13.59 is retired, bump the pin
from checksums.txt (both systems).

## For the app (files this branch does not own)

- `main.rs`: `fn profile_dir() -> PathBuf { modlight_core::personal_folder() }` (replaces
  `dirs::data_dir().join(APP_NAME)`; same result on macOS); its header line names
  `~/Library/Application Support/Modlight` only.
- Texts that are `l10n::mac` today, upstream's key on Windows (module `s` =
  ShareX.resx, `tools` = ShareX.Tools.resx):
  - `capture_flow.rs` `HotkeyProblem` -> `s::MainWindow_HotkeyProblem`.
  - `settings.rs`: `PageMacIntegration` -> `s::SettingsWindow_PageWindowsIntegration`,
    `HintMacIntegration` -> `s::SettingsWindow_HintWindowsIntegration`, `PageLocalFolders`
    -> `s::SettingsWindow_PageLocalFolders`, `HintLocalFolders` ->
    `s::SettingsWindow_HintLocalFolders`, `HintFileNames` -> `s::SettingsWindow_HintFileNames`,
    `HintWatchFolders` -> `s::SettingsWindow_HintWatchFolders`, `AccountSignedOutSync` ->
    `s::SettingsWindow_AccountSignedOutSync`.
  - `app_pages.rs`: `MacTitle` -> `s::ApplicationSettingsWindow_Windows` (or
    `_Integration`); `StartAtLogin` -> `s::ApplicationSettingsForm_cbStartWithWindows_Text`
    ("Run Modlight when Windows starts"); `StartAtLoginDisabledByUser` ->
    `s::FirstTimeConfigWindow_StartWithWindowsDisabledByUser` (also `_DisabledByPolicy`,
    `_EnabledByPolicy`); `StartAtLoginRequiresApproval`, `StartAtLoginNotAvailable`,
    `OpenLoginItems` -> none on Windows; `FinderServiceHint`/`FinderServiceButton` -> the
    check boxes `s::ApplicationSettingsWindow_ShowUploadWithShareXButtonInWindowsExplorerContextMenu`,
    `..._ShowEditWithShareXButtonInWindowsExplorerContextMenu`,
    `s::ApplicationSettingsWindow_ShowShareXInSendToMenu`; `PersonalFolderFixed` ->
    `windows::PersonalFolderFixed` (new, unless `PersonalPath.cfg` is ported);
    `DontShowPrintDialog` -> `s::ApplicationSettingsWindow_DontShowWindowsPrintDialog`.
  - `first_run.rs`: `FirstRunSubtitle` -> `s::FirstTimeConfigWindow_Subtitle`; the start
    texts as above (`s::FirstTimeConfigWindow_StartWithWindows`); `FinderServiceHint` ->
    `s::FirstTimeConfigWindow_ShellContextMenu` and `s::FirstTimeConfigWindow_SendToMenu`;
    `FirstRunOptionsLater` -> `s::FirstTimeConfigWindow_OptionsLater`.
  - `tools/pin.rs` `PinWheelHint` ->
    `tools::PinToScreenOptionsWindow_Mouse_wheel_changes_scale_Ctrl_mouse_wheel_changes_opacity`.
  - `tools/jobs.rs` `BorderlessNeedsAccessibility`, `TopMostUnavailable` -> not shown on
    Windows (both work there as upstream).
- Shortcuts page: egui reports neither Print Screen, Scroll Lock, Pause nor the Win key, so
  they cannot be typed on Windows yet; the platform would have to report them (for example
  `GetAsyncKeyState` while a row captures) and the page call
  `hotkeys::gesture(Some("PrintScreen"), ctrl, shift, alt, win)`.
- Useful new API: `download::tool::{YT_DLP_FILE_NAME, FFMPEG_FILE_NAME}`,
  `recording::options::capture_device::device_title(name)`,
  `components::archive::extract_zip_folder`, `names::file_helpers::special_folder_path`.
- The update window reveals the verified `.exe`/`.zip` (platform reveal = Explorer).
- `tests/tools_video.rs` says "no ffmpeg on this Mac"; `qa_media.rs` and others use
  `/Users/alex/...` sample paths (app files).
- `docs/MACOS.md` / `STATUS.md` were not edited: the substitutions above are upstream's
  behaviour on Windows, so the Windows column of MACOS.md is what Windows now does.

## Remaining differences from upstream on Windows

- Personal folder fixed (no `PersonalPath.cfg`, `-portable`, registry).
- A relative PATH entry is not searched for tools (hardening).
- "Get recorder devices" not ported; the screen goes through the pipe, not gdigrab/ddagrab.
- The custom FFmpeg command template starts after the pipe input (upstream's contained the
  gdigrab input with `$area_x$` etc.).
- yt-dlp's update end-to-end test runs only on Unix (it needs an executable that prints a
  version); the release and digest rules are tested on both.
- Emoji are painted by the port's COLR painter, not Direct2D: the glyphs looked at (faces,
  people with skin tones, ZWJ families, hearts, the rainbow) match Windows' own rendering
  by eye, but pixels are not compared with Direct2D's.
