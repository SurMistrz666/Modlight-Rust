# HANDOFF: app-3 (phase 3 of the macOS Rust port of Modlight App)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-3`, branch `app-3` (from main
`dff2386`), everything under `app/`. Never pushed. Rules: `app/docs/PORT.md`; the
Windows-to-macOS decisions are in `app/docs/MACOS.md`, the per-screen state in
`app/docs/STATUS.md`. The media slice (components, Download tab, updates, YouTube
sign-in) was built on `app-3-media` and merged; its own notes are in
`docs/handoff/app-3-media.md`.

## State

Phase 3 is complete and committed. `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo test --workspace` are green (with the reference
server on 8420 running; its tests skip themselves when it is not).

What exists, by crate:

- `modlight-core`: `capture/` (geometry, region selection state machine, capture kinds,
  last region, fake desktop, scrolling stitcher), `recording/` (FFmpegOptions, argument
  vectors incl. the custom-command template, ffmpeg runner, avfoundation device list,
  recording session with pause and two-pass GIF), `hotkeys` (HotkeyType, WinForms Keys,
  Mac gesture and labels, defaults, manager, SetDefaultSettings/Cleanup), `admin` (the
  Server tab's words), `backup` (.sxb export/import/reset), `debug_log` (log backend,
  monthly file, cleanup), `components/`, `download/`, `update` (from app-3-media),
  `tasks::actions` (external programs).
- `modlight-macos`: `capture` (ScreenCaptureKit pictures, SCStream recorder frames,
  CGWindowList, displays), `hotkeys` (global-hotkey), `integration` (SMAppService login
  item, Finder service provider), `input` (posted scroll/keys), `windows`, `webview`.
- `modlight-app`: capture flow, region overlay, recording UI, scrolling capture, auto
  capture, after-capture, quick task menu and editor, Capture tab, tray menus; settings
  (`settings_kit`, `task_pages`, `task_editors`, `app_pages`, `hotkey_page`, the hotkey
  settings window), `ffmpeg_window`, `admin_tab`, `about_window` (About + Debug log),
  `first_run` (first-time configuration + image editor selector), plus the media
  modules.

Tests: unit tests in every new module; `modlight-core/tests/recording.rs` (real
Homebrew ffmpeg: mp4, GIF two-pass, stop/abort, pause+concat); `modlight-app/tests/
capture_upload.rs` (fake desktop -> simulated Print Screen and region hotkeys -> saved
and uploaded to 8420 -> downloaded back and compared byte for byte -> deleted; an
`#[ignore]` test removes leftovers of an interrupted run); snapshots of every QA screen
in four looks (444 pictures, all passing).

## Check in a real window with the owner

None of these was done by automation (no window was opened on the owner's screen and no
permission prompt was triggered). Each needs a built `Modlight.app`
(`app/scripts/build-app.sh`), ideally in `/Applications`.

1. **Captures** (Screen Recording, asked by macOS on the first capture): full screen,
   active monitor, monitor and window lists, active window, region (selection,
   magnifier, crosshair, keyboard nudging, window and control detection), last region,
   custom region and window. Check the overlay covers the menu bar, the Dock and
   full-screen spaces on every display, with two displays at different scales.
2. **Global hotkeys** (no permission): the defaults Cmd+Shift+1/2/7/8/9 fire from any
   app; typing a new shortcut on Settings -> Keyboard shortcuts; a combination another
   app holds shows the red state and the Capture tab notice.
3. **Screen recording** (Screen Recording; Microphone and Camera only if those sources
   are chosen in the FFmpeg options): region and full-screen video and GIF, start delay,
   pause, stop, abort, the red frame not being in the video (own app excluded), the
   toolbar. FFmpeg options: device list (opening the window lists devices through
   ffmpeg; no prompt is expected), refresh, browse for ffmpeg.
4. **Scrolling capture** (Accessibility, System Settings -> Privacy & Security ->
   Accessibility): the target window scrolls and the stitch is right.
5. **Auto capture**: repeat timer, region choice, minimise.
6. **Login item** (System Settings -> General -> Login Items, a notification "Background
   Items Added" may appear): Settings -> macOS integration and the first-run window
   (shown on the first launch of the bundle, i.e. with no ApplicationConfig.json).
7. **Finder service** (no permission; may need logging out and in, or
   `/System/Library/CoreServices/pbs -update`): right-click files -> Services -> Upload
   with Modlight (Polish: Prześlij za pomocą Modlight).
8. **Save and open panels** ("Save image to file as...", backup export/import, browse
   buttons): NSSavePanel/NSOpenPanel open and no prompt appears.
9. **Sounds and notices** after captures and recordings.
10. The media slice's own checks (Download tab with a real yt-dlp, components installs,
    YouTube sign-in in WKWebView, update with a build-time host): see
    `docs/handoff/app-3-media.md`.

## Remaining differences from upstream

All listed in `app/docs/MACOS.md`; the main ones:

- Hotkeys: Mac modifier mapping and defaults; F13/F14/F15 for Print Screen/Scroll
  Lock/Pause (labelled F13...); every hotkey unregistered while one is being typed.
- Recording: ScreenCaptureKit frames piped to ffmpeg; avfoundation devices; no NVENC,
  AMF, Quick Sync (VideoToolbox, bitrate only); no "Get recorder devices"; the screen
  source is stored as upstream's `gdigrab`.
- Integration: login item instead of the Run key; Finder Services instead of Explorer
  menus (always present, cannot be switched from the app); first-run window on the
  first launch of the bundle instead of after the installer.
- Personal folder fixed; backup without UploadersConfig.json (Keychain).
- About without getsharex.com and the ShareX store/social links; Debug log reached from
  About (upstream's menus no longer reach it); no "Loaded assemblies"; no logo clicker.
- Proxy settings stored, not applied to requests. Print settings stored; printing,
  image effects configuration and OCR languages wait for their phases ("later").
- Custom time zones other than UTC fall back to local time (core, phase 2).
- History and image history windows (`ShareX.HistoryLib` views) were not part of the
  phase 3 brief and are not done.
- Server gap from the media slice: `/api/releases/app` publishes one release for all
  platforms.

## For phase 4 (image editor)

- `capture::ANNOTATION_TOOLS_AVAILABLE` (false) gates the region overlay's annotation
  toolbar; `RegionSession` already has the tool-switch action (`SwapToolType`) and the
  toolbar row where upstream's drawing tools go (`region_overlay.rs`, `toolbar`).
- `TaskQuestion::Annotate` (worker `AnnotateImage`) is answered by
  `uploads.rs::annotate_image`, today a "later" notice that returns the picture
  unchanged; replace it with the editor and return the edited image.
- `TaskEffect::PinToScreen` and the notice actions "Annotate image"/"Pin to screen" push
  `Command::NotYet`.
- `HotkeyType::ImageEditor` runs `first_run::open_image_editor`: the selector
  (`ImageEditorSelectorWindow`) stores `ToolsSettings.UseLegacyImageEditor` and
  `ShowImageEditorSelector = false` in the default task settings, then pushes NotYet;
  hook the editor in `editor_selector_finished` / `open_image_editor`.
- `ToolsSettings.UseLegacyImageEditor` is edited on Settings -> Tools; screen colour
  picker options are there too.
- Recording's after-capture window already passes video files (`after-capture-recording`).

## Gotchas

- `named_enum!`/`flags!` are `pub(crate)` macros in `tasks/settings.rs`.
- `TaskSettings` groups a hotkey takes from the defaults are `null` in the hotkeys
  document (`hotkeys::cleanup` on save); opening a hotkey's settings copies the defaults
  in (`set_default_settings`).
- The hotkey settings window is drawn by swapping it into `windows.settings` for the
  frame (`capture_windows.rs`); the settings pages always read `windows.settings`.
- ffmpeg reading frames from stdin prints no "Press [q] to stop": the first frame written
  marks the recording as started.
- QA renders seed the random name codes (`names::helpers::set_thread_seed`); a screen
  that draws random text each frame would still flicker, so previews are computed on
  change.
- Region captures over a detected window are named with the window pattern
  (`NameFormatPatternActiveWindow`), as upstream; tests that look an upload up by name
  must set both patterns.

## Item 8 (the "decimal point" in the Polish My space snapshot)

Every size string on My space is formatted in the Polish culture ("58,6 KB";
`cloud_size` was instrumented and every call ran with pl-PL). In the 1040x720 snapshot
the second row of cards is cut by the grid's viewport at the pager bar, and the cut
takes the comma's tail, so "58,6" reads as "58.6". Not a formatting bug. The QA screen
`my-space-wide` renders My space at upstream's 1486x943, where every card is whole.
