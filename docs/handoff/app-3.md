# HANDOFF: app-3 (phase 3 of the macOS Rust port of Modlight App)

Worktree `/path/to/modlight/_wt/app-3`, branch `app-3` (from main
`dff2386`), everything under `app/`. Never pushed. Rules: `app/docs/PORT.md`.
A helper works on Components, the Download tab (yt-dlp) and the update checker in
`/path/to/modlight/_wt/app-3-media` (branch `app-3-media`); its
notes go to `docs/handoff/app-3-media.md` and are merged here when it is done.

## State (kept current while working)

Done and committed:

- Core (`modlight-core`): `capture/` (geometry, `RegionCaptureOptions` and
  `TaskSettingsCapture` with upstream defaults, the region selection state machine of
  `RegionCaptureWindow`/`RegionCaptureLightForm`, capture kinds, last region, fake
  desktop source, scrolling stitcher and loop), `recording/` (`FFmpegOptions`, argument
  VECTORS for recording/encoding/GIF palette/concat, ffmpeg process runner, the
  recording session with start/stop/pause/restart/abort and two-pass GIF), `hotkeys`
  (HotkeyType, WinForms `Keys` read/written like Newtonsoft, Mac modifier mapping and
  defaults, the hotkeys document, `HotkeyManager` over a backend), `tasks::actions`
  (ExternalProgram, "Perform actions" with argument vectors), the worker's
  AnnotateImage/PinToScreen/SaveImageToFileWithDialog/PerformActions and `file_job`.
- macOS (`modlight-macos`): `capture` (ScreenCaptureKit pictures and the recorder's
  SCStream frames, CGWindowList, displays and pointer from CoreGraphics), `hotkeys`
  (global-hotkey, Carbon registration, no permission), `integration` (SMAppService
  login item, Finder service provider `uploadFiles:userData:error:`), `input` (posted
  scroll/keys for scrolling capture, needs Accessibility), `windows::raise_overlay`.
- App: `capture_flow` (delay, hiding the main window, picture off the UI thread,
  overlay, sound, quick task menu, after-capture window, tasks; hotkeys and
  `ExecuteJob`; auto capture), `region_overlay`, `recording_ui` (frame + toolbar),
  `scrolling`, `capture_windows` (all viewports), dialogs `after_capture`,
  `auto_capture`, `quick_task_menu`, `quick_task_editor`, `controls` (numeric box,
  radio row).

Not done yet (in order): Capture tab and capture menu wiring, tray menus, QA screens and
snapshots for the new windows, settings pages, hotkey settings page, server tab/admin,
first run, about, debug log, item 8 write-up, docs (MACOS.md, STATUS.md), merge of
`app-3-media`.

## Item 8 (the "decimal point" in the Polish My space snapshot)

Investigated: every size string on My space is formatted in the Polish culture
("58,6 KB"; `cloud_size` was instrumented and every call ran with pl-PL). The second
row of cards in the 1040x720 snapshot is cut by the grid's viewport at the pager bar,
and the cut takes the comma's tail, so "58,6" reads as "58.6". Not a formatting bug.

## Gotchas

- `named_enum!`/`flags!` are `pub(crate)` macros in `tasks/settings.rs`; enums in other
  modules use them (paths inside are fully qualified).
- `TaskSettings` members a hotkey takes from the defaults are `null` in the hotkeys
  document (`Cleanup`); nested members read through `null_default`.
- ffmpeg reading frames from stdin prints no "Press [q] to stop": the first frame
  written marks the recording as started.
- QA and headless runs use `FakeSource` (`<profile>/qa-capture/{desktop.png,layout.json}`
  or the built-in sample) and a fake hotkey backend; tests press hotkeys through
  `state.captures.simulated`.
