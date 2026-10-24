# Logic review 2026-10-24, agent 10: Windows

Area: the Windows platform crate (`app/crates/modlight-windows/src/**`), `app/crates/modlight-app/src/win32.rs`,
`app/crates/modlight-app/build.rs`, `app/scripts/build-app.ps1`, and the Windows branches
(`cfg(windows)`, `cfg!(windows)`, `target_os`) of the shared crates. Specification: upstream
`Modlight-App` (`ShareX/**`, `ShareX.HelpersLib/**`, read only).

Branch `logic-10-windows` from `main` at `28916b4`. Never pushed.

## How it was checked

- Every file of `modlight-windows` read against the upstream code it ports (`CaptureHelpers`,
  `WindowsList`, `CursorData`, `Screenshot*`, `InputHelpers`/`InputManager`, `HotkeyInfo`,
  `StartupManager`, `ShortcutHelpers`, `IntegrationHelpers`, `SingleInstanceManager`,
  `MutexManager`, `CLIManager`/`ShareXCLIManager`, `OCRHelper`, `BorderlessWindowManager`,
  `DownloaderWindow`/`UpdateChecker`, `HotkeyProblemViewModel`, `WinFormsTrayIconService`,
  `ClipboardUpload`/`ClipboardUploadWindow`), and the FFI against the Win32 contracts (struct
  sizes, buffer lengths, ownership and freeing, last-error reads, overlapped I/O, COM apartments,
  thread affinity of windows and hooks, DPI awareness).
- The Windows target was compiled on this Mac: `rustup target add x86_64-pc-windows-msvc`, then
  `cargo clippy --target x86_64-pc-windows-msvc --workspace --exclude modlight-macos --all-targets
  -- -D warnings`. The C build scripts (bundled SQLite, aws-lc) cannot find the Windows SDK
  headers here, so they were given a no-op compiler and archiver
  (`CC_x86_64_pc_windows_msvc` / `AR_x86_64_pc_windows_msvc` pointing at two small scripts that
  only create their output files): enough for `check`/`clippy`, which never link. Nothing
  Windows was run.
- macOS: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p modlight-app` (lib, bin, integration and snapshot tests).
- Tests were written before each fix. Pure logic is tested on every system; tests of Windows-only
  code live in `modlight-windows` or `cfg(windows)` modules and were only compiled here.

## Findings

| Id | Severity | File:line (at `28916b4`) | What is wrong | How it was found / why it is certain | Status |
|---|---|---|---|---|---|
| W10-1 | Medium | `app/crates/modlight-app/src/main.rs:110-129` | Windows command line: every word without a dash was a file to upload. Upstream's `CLIManager.ParseCommands` binds the word right after a `-command` to that command (`-ImageEffect <file.sxie>`, `-task <name>`, `-workflow <name>`, also `-silent <word>`). Upstream writes the `.sxie` association (`"<exe>" -ImageEffect "%1"`) on every start, so a port installed at the same path uploaded the double-clicked effect package to the account's server. | Read upstream `CLIManager.cs`, `ShareXCLIManager.cs`, `IntegrationHelpers.cs`, `Program.RegisterExtensions`; tests `the_word_after_a_command_is_its_parameter_not_a_file`, `free_standing_words_are_files` (run on macOS). | fixed `07062ea` |
| W10-2 | Medium | `app/crates/modlight-app/src/update_window.rs:368`, `:392-424`, `:492`, `:610` | Windows update: every verified release went to `start_installer` (ShellExecute "open" + `/UPDATE`), but a Windows release may be a `.zip` (`update.rs` accepts `.exe` and `.zip`; `build-app.ps1` makes only the `.zip`): the archive was opened with a stray `/UPDATE`, the window said the installer started and closed, nothing was installed. And after an installer did start the app kept running, while upstream's `DownloadUpdateAsync` quits ("making way for it"): a running `Modlight.exe` cannot be replaced. `WINDOWS.md` still said updates are never run (stale since `d569597`). | Read upstream `DownloaderWindow.StartInstaller` and `UpdateChecker.DownloadUpdateAsync`; test `only_an_installer_is_started` (macOS). | fixed `7c99dbb` (`.exe`: start, then quit; `.zip`: portable question, revealed in Explorer; docs) |
| W10-3 | Low-Medium | `app/crates/modlight-windows/src/integration.rs:632-652` | A later start whose mutex says another copy runs, but whose pipe gets no answer within 5 s, returned `false` and started a full second copy: no mutex, no pipe, the same settings, upload queue (duplicate uploads) and history, its hotkeys refused. Upstream never runs a later instance (`RedirectArgumentsToFirstInstance` swallows the failure, `Main` runs only in the first). | Code reading against `SingleInstanceManager`/`MutexManager`; test `a_later_start_never_runs_beside_the_first` (Windows only, compiled). A start that cannot open the mutex at all still runs when nobody answers. | fixed `509b4ea` |
| W10-4 | Low | `app/crates/modlight-windows/src/hotkeys.rs:225-239`, `app/crates/modlight-app/src/win32.rs:168-183` | Hotkeys on keys `global-hotkey` has no `Code` for (Oem8, the ABNT keys) are registered by virtual-key code on the crate's own window and queued in `RAW_PRESSED`; the app's wake-up thread polled only `global-hotkey`'s channel, so such a press waited for the next repaint (idle window, or hidden in the tray: indefinitely). | Code reading of the wake-up path and of eframe's repaint rules; test `a_key_registered_by_its_code_wakes_the_window` (Windows only, compiled). | fixed `0c90712` (`SystemHotkeys::waiting()`; the app's direct `global-hotkey` dependency removed, `Cargo.lock` one line) |
| W10-5 | Low | `app/crates/modlight-app/src/uploads.rs:1011-1037` | `read_clipboard` asked for the file list first on every system: the Mac's rule (its comment says so). Upstream (`ClipboardUpload`, `ClipboardUploadWindow`) asks for a picture, then text, then files, so on Windows a program that offers a picture or text with a file list got the other upload. | Read upstream `UploadManager.ClipboardUpload` and `ClipboardUploadWindow.axaml.cs`; test `the_clipboard_is_read_in_upstreams_order_except_on_a_mac`. | fixed `8143111` (macOS unchanged) |
| W10-6 | Low | `app/crates/modlight-app/src/win32.rs:115-129` | `sharex_running()` answered `false` for any `OpenMutexW` error; upstream's `IsShareXRunning` answers `true` on `UnauthorizedAccessException` (the mutex exists, e.g. an elevated ShareX), so the hotkey notice blamed the Snipping Tool while ShareX held the keys. | Read `HotkeyProblemViewModel.cs`; test `a_mutex_this_process_may_not_open_still_says_sharex_runs` (Windows only, compiled). | fixed `da9ea9f` |
| W10-7 | Low | `app/crates/modlight-app/src/main.rs:75` | `std::env::args()` panics on an argument that is not Unicode. Windows allows a lone surrogate in a file name and Explorer passes it on (the single-instance pipe already carries such names), so "Upload with Modlight" or Send to on such a file killed the start before any window or message. | Rust's documented `env::args` behaviour; test `a_word_that_is_not_unicode_does_not_stop_the_start` (macOS, with a non-UTF-8 word). | fixed `6accd15` (`args_os`; the port's own options still read text) |
| W10-8 | Low | `app/crates/modlight-app/build.rs:13-16` | The icon, version information and manifest (long paths, per-monitor DPI v2, common controls 6, `asInvoker`) come from `winresource`, a build dependency of Windows hosts only; building for Windows elsewhere left them out without a word. | Observed: `cargo check --target x86_64-pc-windows-msvc` on macOS built no `winresource` and printed nothing. | fixed `9464e29` (cargo warning) |
| W10-9 | Info | `app/crates/modlight-app/src/capture_flow.rs:783-797` | The capture menu's monitor list shows sizes in the app's points ("1. 1707x960" on a 2560x1440 monitor at 150 %); upstream (PerMonitorV2) shows pixels. Labels only; captures are right. | Read `MainMenuBuilder.BuildMonitorMenu` and `ShareX.csproj` (`ApplicationHighDpiMode`). | not fixed: exact pixel sizes are not in `ScreenInfo` (points are rounded, so points times scale can be off by one); recorded in `WINDOWS.md` |
| W10-10 | Info | `app/crates/modlight-app/src/main.rs` (Windows command line) | A URL on the command line is not downloaded and uploaded (upstream's `DownloadAndUploadFile`); it was, and still is, dropped as a path that does not exist. | Read `ShareXCLIManager.UseCommandLineArgs`. | not fixed (the single-instance pipe carries only existing local paths by design); recorded in `WINDOWS.md` |

## Checked and OK

`modlight-windows`:
- `wide.rs`: NUL termination, lossy decoding only for display text.
- `credentials.rs`: `CREDENTIALW` filled with live buffers, `CRED_MAX_CREDENTIAL_BLOB_SIZE` checked
  before the call, `CredFree` once, `ERROR_NOT_FOUND` on delete is success, UTF-16 values of
  other programs read.
- `system.rs`: `FormatMessageW` with `ALLOCATE_BUFFER` freed by `LocalFree`; `ShellExecuteExW` on an
  STA thread with `SEE_MASK_NOASYNC`; URL quoting against `"%1"` injection; folder targets get
  upstream's trailing `\`; Recycle Bin list double-NUL terminated, verbatim prefix stripped,
  wildcards refused, `DE_*` codes mapped; locale and UI language buffers; NetBIOS name;
  `SPI_GETCLIENTAREAANIMATION`.
- `network.rs`: `GetAdaptersAddresses` growth loop and alignment (difference from .NET, a
  unicast address required, already documented).
- `sound.rs`: the `SND_MEMORY` buffer is kept alive until a later sound stopped it.
- `capture/*`: per-thread DPI context entered and restored; edge-by-edge point/pixel conversion;
  DIB size and stride; `BitBlt` clipped to the virtual screen; cursor hot spot, size multiplier
  and draw size as `CursorData` (upstream's extra `ScreenToClient` offset in `DrawCursor` is an
  upstream bug on monitors left of or above the primary; the port draws at the right place);
  `CaptureWindowTransparent` (backdrop z-order, white/black/white, `CreateTransparentImage`,
  `AutoCropImage`, `TrimShadow` thresholds, no `u32` underflow possible in the crop);
  `IsActiveWindowFullscreen`; `WindowsList.IsValidWindow` filters; `HWND` truncation and sign
  extension; process names; recorder frames (exclusion affinity set only on `WDA_NONE` windows and
  restored, off-desktop parts black, rate loop).
- `keys.rs`: thread `WH_KEYBOARD` hook, `HC_ACTION` only, release and repeat bits, Print Screen on
  its key-up, AltGr excluded from Ctrl+V.
- `input.rs`: `SendInput` with `size_of::<INPUT>()`, wheel delta sign, `WM_VSCROLL` with a timeout,
  `AttachThreadInput` undone.
- `hotkeys.rs`: every `code_for` value checked against `global-hotkey`'s `key_to_vk`; Ctrl+Pause as
  `VK_CANCEL`; `global-hotkey` ids are `(mods << 16) | code`, so they never meet `RAW_IDS`;
  unregister and drop order.
- `integration.rs`: registry values against `IntegrationHelpers` (command `"<exe>" "%1"`, icon
  `"<exe>",0`, both keys checked), `StartupManager` state and `StartupApproved` byte, shortcut COM
  on its own STA thread; single-instance protocol (owner-only SDDL, `FIRST_PIPE_INSTANCE`,
  `REJECT_REMOTE_CLIENTS`, owner check by the client, `SECURITY_IDENTIFICATION`, header and
  payload bounds, overlapped I/O cancelled and awaited before the `OVERLAPPED` goes, `ERROR_PIPE_CONNECTED`
  and `ERROR_NO_DATA` handled).
- `ocr.rs` (`OCRInternal` joining rules, size limit, premultiplied BGRA), `print.rs` (`DEVNAMES`
  layout, `PrintDlgEx` with the classic fallback, `StretchDIBits`, `PHYSICALOFFSET`, PDF xref
  offsets), `editor.rs` (cursor masks, wallpaper style mapping, `TranscodedImageCache` scan, WIC
  size check before decoding, apartment handling), `ax.rs` (styles and placement as
  `BorderlessWindowManager`, failure restores), `webview*` (main-thread checks, InPrivate folder
  lifetime and sweep, outcome reported once), `windows.rs` (winit class filter, top-most overlay).

App and shared Windows branches:
- `win32.rs`: console attach only when not redirected, `ITaskbarList3` creation and states,
  double-click time clamp, installer start with `ERROR_CANCELLED`.
- `build-app.ps1`: version line, cargo exit code under PowerShell 5.1, `DirectML.dll` lookup, zip
  and SHA-256.
- `modlight-core`: `names/file_helpers.rs` Windows `path` module compared with .NET's
  `PathInternal.GetRootLength`, `IsDevice`/`IsExtended`/`IsDeviceUNC`, `GetDirectoryName`,
  `NormalizeDirectorySeparators`, `GetFileName`, `GetExtension`, `Combine`; special folders
  against .NET's `Environment.Windows.cs` mapping; `ExpandEnvironmentStringsW` loop;
  `name_parser.rs` user, domain and machine names; `hotkeys.rs` defaults and labels against
  `HotkeyManager`/`HotkeyInfo`; `components/process.rs` job objects; `archive.rs` device names
  and colons; free space; `download/cookies.rs` owner-only ACL; `watch.rs` file identity and
  `FileShare.None` probe; `tasks/actions.rs` hidden window and case-insensitive output test;
  `recording/args.rs` dshow inputs and device parsing; `personal_folder.rs` (app).
- `modlight-effects` `safe_local_path` (Windows), `modlight-tools` indexer hidden attribute,
  tray clicks against `WinFormsTrayIconService`, the hidden-window frame path of eframe 0.36
  (an invisible window still runs full frames on Windows, so hotkeys and Explorer files are
  taken while the app sits in the tray).

## Not verifiable here (needs Windows)

Every test of `modlight-windows` and the `cfg(windows)` tests of the app were compiled only:
`a_key_registered_by_its_code_wakes_the_window`, `a_later_start_never_runs_beside_the_first`,
`a_mutex_this_process_may_not_open_still_says_sharex_runs`, and the crate's existing ones. Also
not seen: a real `WM_HOTKEY` for Oem8 waking the tray app, the installer start and the app's
exit after it, a `.zip` release revealed in Explorer, the `.sxie` association calling the port,
Explorer passing a non-Unicode file name, `SetWindowDisplayAffinity` from the frames thread, and
the C build scripts with a real MSVC toolchain.

## Shared files touched outside the Windows crate

- `app/crates/modlight-app/src/main.rs` (Windows command line and `args_os`; macOS gets the same
  `args_os` reading, its options unchanged).
- `app/crates/modlight-app/src/update_window.rs` (Windows install flow; macOS unchanged).
- `app/crates/modlight-app/src/uploads.rs` (clipboard order; macOS unchanged).
- `app/crates/modlight-app/Cargo.toml` and `app/Cargo.lock` (the app's direct `global-hotkey`
  dependency removed; the package stays through `modlight-windows` and `modlight-macos`).
- `app/docs/WINDOWS.md`.

## Verification at the end of the branch

- macOS: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings`
  clean; `cargo test -p modlight-app`: 45 lib, 3 bin, and all integration and snapshot tests
  pass.
- Windows target (on macOS, no-op C compiler for the build scripts): `cargo clippy --target
  x86_64-pc-windows-msvc --workspace --exclude modlight-macos --all-targets -- -D warnings`
  clean (the build script's new warning about the missing resources is the only message).
