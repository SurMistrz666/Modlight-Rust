# Modlight on Windows: what the Rust port uses, and where it differs from upstream

Upstream (`Modlight-App`) is a Windows program, so on Windows the port goes back to
upstream's own mechanism wherever `MACOS.md` lists a macOS substitute. This file records
what each one is in Rust, and the few places where the Windows build still differs from
upstream. Everything not listed behaves as upstream does, and as `STATUS.md` says.

The code: the platform crate is `crates/modlight-windows` (Windows only; it has the module
and function names of `crates/modlight-macos`, and the app reaches whichever one its build
uses as `crate::platform`). Shared crates choose with `cfg(windows)`; macOS keeps exactly
what it did.

## Build, package, run

| What | How | Notes |
|---|---|---|
| Toolchain | Rust stable (1.99+) with the MSVC target, Visual Studio Build Tools with a Windows SDK | Nothing else to install; ONNX Runtime and WebView2's loader are linked in. |
| Build | `cargo build --release -p modlight-app` in `app/` | `crates/modlight-app/build.rs` embeds upstream's `Modlight_Icon.ico`, the version information and a manifest (Windows 10/11, per-monitor DPI awareness v2, long paths, common controls 6, `asInvoker`). The program is a GUI program (`windows_subsystem = "windows"`); `--qa-shot`, `--help` and argument errors attach to the console they were started from. |
| Package | `app\scripts\build-app.ps1 [-OutDir <dir>]` | `dist\Modlight\` (Modlight.exe, DirectML.dll, the licences) and `dist\Modlight-<version>-<build>-windows-<arch>.zip` with its SHA-256 and its description (`.json`), published to the server's Downloads when configured (see "Updates and publishing"). No installer (upstream's Inno Setup script is not ported): the folder runs where it is. |
| ONNX Runtime | `ort` with the `directml` provider on Windows (`coreml` on macOS) | pyke publishes no Windows build with Core ML. `DirectML.dll` ships beside the exe (`copy-dylibs` puts it next to every build). |
| Debug builds | `[profile.dev.package.tiny-skia] overflow-checks = false` | tiny-skia's portable u16 arithmetic (the x86_64 path) wraps by design; with overflow checks it panicked in debug builds. |

## Platform services

| Upstream | Windows port | Notes |
|---|---|---|
| Personal folder `Documents\Modlight` (`Program.PersonalFolder`), `PersonalPath.cfg` | The same: the shell's Documents folder (a moved or OneDrive Documents is followed), or the folder `PersonalPath.cfg` names (beside the exe, else inside `Documents\Modlight`), which the Paths page writes | `-portable` and the registry override are accepted and ignored. |
| Token encrypted with DPAPI inside `UploadersConfig.json` | Credential Manager, generic credential `Modlight/api-token` (DPAPI-protected for this user), the token stored with the origin it was issued for, as the macOS Keychain item | AI keys are credentials `Modlight/ai-*-api-key`. No `UploadersConfig.json` is written, so none goes into a settings backup. |
| `Process.Start(url)` | `ShellExecuteExW`, http and https only; characters that could leave the browser's quoted `%1` are percent-encoded | |
| Open file / folder, "Show in folder" | `ShellExecuteExW` on an existing absolute path; `SHOpenFolderAndSelectItems` (fallback `explorer.exe /select,`) | |
| Delete to the Recycle Bin | `SHFileOperationW` with undo, no questions | A drive without a Recycle Bin deletes for good, as upstream. |
| Culture and UI language | `GetUserDefaultLocaleName`, `GetUserPreferredUILanguages` | |
| Client-area animations | `SPI_GETCLIENTAREAANIMATION` | |
| Network availability | `GetAdaptersAddresses`: an adapter that is up, not loopback or a tunnel, with a unicast address | |
| Clipboard | arboard for text and file lists; pictures read here as the "PNG" format, else `CF_DIBV5`/`CF_DIB` turned into a BMP file, and decoded under the app's limits. Asked for in upstream's order (`ClipboardUpload`): a picture, then text, then a file list | A Mac asks for the file list first. Chromium's 32-bit `BI_RGB` with an alpha mask keeps its transparency; all-zero alpha counts as opaque. |
| Ctrl+V on the main window | A `WH_KEYBOARD` hook of the UI thread | Also gives the shortcuts page Print Screen, Scroll Lock, Pause, the media keys and the Win modifier, which egui never reports. |
| Task sounds | `PlaySoundW` with upstream's WAV files | |
| Global hotkeys (`RegisterHotKey`) | `global-hotkey` (which calls `RegisterHotKey`); keys global-hotkey has no code for (Oem8, the ABNT keys) through `RegisterHotKey` directly | WinForms key codes are virtual-key codes; Ctrl, Shift, Alt and Win are themselves, and the defaults are upstream's Print Screen set. |
| Start with Windows (`StartupManager`) | `Modlight.lnk` with `-silent` in the user's Startup folder; Task Manager's switch read from `StartupApproved\StartupFolder`; "another copy starts with Windows" note | Refused for a build running from a cargo `target` folder. |
| Explorer context menu "Upload with Modlight", "Send to" | `HKCU\Software\Classes\*\shell\Modlight` and `Directory\shell\Modlight` (`"<exe>" "%1"`, the exe's icon, the text in the current language, registered again when the language changes); `Modlight.lnk` in the SendTo folder | "Edit with Modlight" is not offered. |
| Single instance and the command line (`Program.cs`) | Mutex `Local\Modlight-<SID>`, pipe `\\.\pipe\Modlight-<SID>-<session>` (local clients of this user only, the pipe's owner checked, 1 MiB cap, only existing absolute paths queued, nothing received is ever run); a later start hands its files (or "come forward") to the first and exits, also when the first does not answer within 5 s (as upstream); only a start that cannot open the mutex at all runs when nobody answers | Names are the port's own, not upstream's (upstream's pipe never answers). The words are read as upstream's `CLIManager` reads them: one with a dash is a command, the word right after a command is its parameter, any other word is a file or folder to upload. `-silent`/`-s` starts in the tray, `-multi`/`-m` skips the check; `-portable`, `-sandbox`, `-NoHotkeys`, the task commands and `-ImageEffect` are accepted and logged with their parameters (so upstream's `.sxie` association, `-ImageEffect "%1"`, uploads nothing). A URL on the command line is not downloaded (upstream's `DownloadAndUploadFile`): only existing paths are taken. |
| Tray icon (`WinFormsTrayIconService`) | `tray-icon`: right button opens the menu, left, double and middle clicks run the configured tray actions with the system's double-click time | |
| Taskbar progress (`TaskbarManager`) | `ITaskbarList3` | |
| Child processes (`ChildProcessJob`) | A job object per child that kills what is left when it closes; children start without a console window | A crash of Modlight ends its children too. "Perform actions" shows a console only when the action is not set to a hidden window, as upstream. |
| Watch folders (`FileSystemWatcher`) | `notify` (ReadDirectoryChangesW); a file is known by its volume serial number and file index | NTFS gives a new file the deleted one's creation time when the name is reused within seconds, so the birth time alone is not an identity. |
| Free disk space | `GetDiskFreeSpaceExW` | |
| Special folders (`%MyPictures%`...) and environment strings | .NET's own mapping to the shell's known folders and its environment fallbacks; `ExpandEnvironmentStringsW` (names ignore case) | Paths accept both separators, drives, UNC and device paths. |

## Window and look

| Upstream | Windows port | Notes |
|---|---|---|
| Main window title bar (client area extended, caption buttons at the right) | Undecorated window; the title band draws the logo at 12 px and upstream's 46 x 34 caption buttons (minimise, maximise/restore, close with the red hover); dragging moves, double-click maximises; 5 px edges and 12 px corners resize | winit keeps the system styles, so snap layouts, Alt+F4, Alt+Space and the taskbar behave. |
| Dialogs, editor, media and tool windows | Native decorations | Overlays and notices stay undecorated. |
| Region capture overlay above the taskbar | One top-most viewport per monitor, raised with `SetWindowPos(HWND_TOPMOST)` | |
| Fonts | Inter for the UI, as on macOS; the image editor's text and emoji use the system's fonts (Segoe UI Emoji drawn as a colour font) | Snapshots of editor text differ from the Mac's for that reason. |

## Capture and recording

| Upstream | Windows port | Notes |
|---|---|---|
| Monitors, pointer, windows (`CaptureHelpers`, `WindowsList`) | `EnumDisplayMonitors`/`GetMonitorInfoW` (friendly names), `GetCursorPos`, `EnumWindows` filtered as upstream (cloaked, tool and minimised windows), DWM frame bounds | Coordinates are the app's screen points: physical pixels divided by the primary monitor's scale. With monitors at different scales, overlays on the others are sized for the primary's scale. The capture menu's monitor list ("1. 1707x960") gives sizes in these points; upstream's (`BuildMonitorMenu`, a PerMonitorV2 process) gives pixels ("1. 2560x1440"): the menu would need the monitors' pixel sizes, which `ScreenInfo` does not carry. Monitor names are read once per device name (`\\.\DISPLAY2`) while the app runs. |
| GDI capture (`Screenshot`) | `BitBlt(SRCCOPY | CAPTUREBLT)`, the pointer drawn at its hot spot | |
| Window capture, client area, shadow, transparent (`ScreenshotTransparent`) | The same: the window's current rectangle; transparency by upstream's white, black, white backdrop | |
| Active window borderless / top-most | Upstream's: styles removed and the window placed, the second press restores; `WS_EX_TOPMOST` toggled | Windows of programs running as administrator cannot be changed (UIPI); the toggle reports it. |
| Scrolling capture input (`InputHelpers`) | `SendInput` (wheel, Down, Page Down, Home) and `WM_VSCROLL` with a timeout | |
| Screen recorder input (`gdigrab`) | The region's pixels by GDI on a capture thread, piped to ffmpeg as raw BGRA (the same pipeline as macOS); the app's own windows are excluded with `WDA_EXCLUDEFROMCAPTURE` while it records | About 25 to 30 fps at 1920 x 1080 in a release build. Windows 10 before 2004 shows the app's windows as black boxes. |
| Microphones and cameras (DirectShow) | `-f dshow`, devices listed and parsed as `GetDirectShowDevices` | "Get recorder devices" (screen-capture-recorder) is not ported. |
| Encoders | Upstream's, the hardware ones included (NVENC, AMF, Quick Sync), with upstream's arguments, in the recorder and the video converter | |

## Tools

| Upstream | Windows port | Notes |
|---|---|---|
| Windows OCR (`OCRHelper`) | `Windows.Media.Ocr`; languages named by Windows; a picture larger than the engine takes is scaled down (upstream let it fail) | Only the languages installed in Windows are offered. |
| Printing (`PrintDocument`, `PrintDialog`) | GDI: the print dialog when asked, the named printer (preselected in the dialog), the picture drawn from the sheet's corner | The preview is the page as a PDF the port writes itself, opened in the default viewer. |
| Cursor annotations (`WindowsCursorBitmapRenderer`) | The system's cursors (`LoadCursorW`, `GetIconInfo`, `GetDIBits`) | Inverting pixels are drawn black. |
| Wallpaper background | The desktop picture and its layout from the system (Fill, Fit, Stretch, Tile, Center, Span) | |
| Pictures the `image` crate cannot read | WIC, refused above the app's size limits before any pixel is decoded | |
| YouTube sign-in (WebView2) | WebView2 (`webview2-com`, static loader) in a window of its own, an InPrivate profile in a temporary folder deleted afterwards, upstream's done-rule | Needs the WebView2 runtime (part of Windows 11 and updated Windows 10). Not yet tried with a real Google account. |
| Components (`ModlightComponent`) | Upstream's pins for FFmpeg 8.1 and yt-dlp, by architecture; ExifTool 13.59 win64 from exiftool.org (upstream's 13.29 download no longer exists), its `exiftool(-k).exe` renamed `exiftool.exe` | Sizes and SHA-256 in `docs/handoff/win-core.md`. Tools are found beside the exe, in `<profile>\Tools`, then on PATH. |
| App updates (`DownloaderWindow`) | The build zip from the Downloads of the server the app is signed in to, verified, unpacked, and put in place of the program's files by a helper once Modlight has quit | See "Updates and publishing" below. Upstream's installer (`/UPDATE`) is not ported. |

## Updates and publishing

As on a Mac (`MACOS.md`, "Updates and publishing": which build this is, the check, the
download), with these differences:

- **Install.** The verified zip is unpacked by the app into `%TEMP%\Modlight\Update\install-*`:
  files and folders only (a link is refused), names with `\` read as `/` (Windows
  PowerShell 5.1's `Compress-Archive` writes them so), none that leaves the folder, at most
  2 GiB written, and `Modlight.exe` at its top (or inside one folder that holds everything).
  The running copy must not be a cargo build (a `target` folder) and its folder must take a
  new file (a probe file, at once deleted); otherwise, as for any refusal, the server's
  download page is offered. A helper (`update::helper::WINDOWS_SCRIPT`, run by
  `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe -File`, without a window, in a
  process group of its own and out of the app's job when the job allows it) waits for
  Modlight to quit (at most 120 s; otherwise nothing changes), then replaces only the files
  the zip has (a file it does not have, such as `PersonalPath.cfg`, stays), keeping each
  replaced one in the update folder until all are in place and putting them back when one
  cannot be written (each file is tried for 5 s, for an antivirus that still holds it), and
  starts `Modlight.exe`. Its output is the update's `update.log`, written to the debug log
  by the next start.
- **No publisher signature.** Windows builds are not signed, so nothing like the Mac's
  signature check stands between the server and the program: the server the person signed
  in to and TLS are what vouch for a build (`PORT.md`, section 5).
- **Publish.** `build-app.ps1` writes the zip entry by entry with `/` between names, its
  description beside it, and publishes it as `build-app.sh` does (`Invoke-WebRequest`, the
  token only in the `Authorization` header, never printed; TLS 1.2 switched on for Windows
  PowerShell 5.1; no redirect followed). The token comes from `MODLIGHT_PUBLISH_SERVER` and
  `MODLIGHT_PUBLISH_TOKEN` or from `%USERPROFILE%\.config\modlight\publish.json`, which must
  be readable by no other account than yours (SYSTEM and Administrators aside, as every
  file in a profile):

```powershell
$env:MODLIGHT_PUBLISH_SERVER = 'https://files.example.com'
$env:MODLIGHT_PUBLISH_TOKEN = '<publish token>'
$env:MODLIGHT_PUBLISH_NOTE = 'What changed'        # optional
app\scripts\build-app.ps1
```

  `MODLIGHT_PUBLISH=0` builds without publishing. A failed publish leaves the build as it
  is and prints how to publish it again:
  `app\scripts\build-app.ps1 -Publish app\dist\Modlight-<version>-<build>-windows-x64.zip`.

Not yet run on Windows: the helper, the PowerShell publishing (both read against Windows
PowerShell 5.1 and PowerShell 7, and the helper's text and arguments are tested on every
system), and the install as a whole.

## Texts

On Windows the app shows upstream's own texts where the macOS port shows `Mac.resx` ones
(integration page, local folders, first run, print dialog, hotkey problem, pin options).
Texts the port needs on Windows that upstream never had are in
`assets/windows/l10n/Windows.resx` and `Windows.pl.resx` (`modlight_core::l10n::windows`).

## Tests on Windows

- `cargo test --workspace` runs on Windows. Tests that need a POSIX shell as a fake tool
  (the yt-dlp download flow and update) are Unix only, and so are the ones that run the
  Mac's update helper and `build-app.sh`'s publishing.
- The snapshots of the update windows and the Language and tray page (`update-available`,
  `update-download`, `settings-general`) changed on 2026-10-27 (the update fixture is a
  server build now, and the Updates card shows in every build): their `windows-*.png`
  pictures are to be made again on Windows (`UPDATE_SNAPSHOTS=1`).
- Snapshots: every screen is first compared with the shared picture; only screens that
  really differ on Windows (the title band, paths, editor text) have a
  `windows-<palette>-<language>.png` beside it, which macOS never reads.
- The effects' golden pictures (rendered by SkiaSharp) match on Windows as on the Mac.
- `modlight-windows` tests read the screen, enumerate windows, write and delete
  throwaway credentials and registry keys, and round-trip the single-instance pipe; tests
  that would show a window, send input, play sound or restyle another program are
  `#[ignore]` and run by hand (`cargo test -p modlight-windows -- --ignored <name>`).

## Not yet checked in a real session

A real-window pass on Windows: the region overlay on several monitors and scales, the
recorder at full size, borderless and top-most on other programs, scrolling capture, the
print dialog with a real printer, the YouTube sign-in with an account, the Explorer menu
and Send to from Explorer, and the tray on a fresh profile.
