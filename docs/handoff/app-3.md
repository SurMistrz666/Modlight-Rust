# HANDOFF: app-3 (phase 3 of the macOS Rust port of Modlight App)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-3`, branch `app-3` (from main
`dff2386`), everything under `app/`. Never pushed. Rules: `app/docs/PORT.md`.
A helper works on Components, the Download tab (yt-dlp) and the update checker in
`/Users/marcinasdfg/Desktop/modlight/_wt/app-3-media` (branch `app-3-media`); its
notes go to `docs/handoff/app-3-media.md` and are merged here when it is done.

## State (kept current while working)

Done and committed (newest last):

- Core (`modlight-core`): `capture/`, `recording/` (args incl. the custom-command
  template `ffmpeg_args_text`, avfoundation `list_devices`), `hotkeys` (Mac `gesture`,
  `set_default_settings`, `cleanup` on save), `tasks::actions`, `backup` (export,
  import of plain json/xml names only, reset).
- macOS (`modlight-macos`): capture, hotkeys, integration, input, windows.
- App: capture flow, region overlay, recording UI, scrolling, auto capture, quick task
  menu and editor, Capture tab, tray menus.
- Merged `app-3-media` (components, Download tab, updates, YouTube sign-in); recording's
  CheckFFmpeg offers the FFmpeg component and records when it arrives.
- Settings: `settings_kit` (Binder by JSON path), `task_pages` (every task page, plus a
  hotkey's Task/General/Upload/Modlight folder pages), `task_editors` (action, watch
  folder, notification buttons), `ffmpeg_window` (FFmpegOptionsWindow), `app_pages`
  (general, integration, paths, backup, main window, copy formats + editor, upload,
  history, print, proxy, advanced), `hotkey_page` (hotkeys list, capture, reset) and the
  hotkey settings window (`windows.hotkey_task`, drawn by swapping it into the settings
  slot for the frame, `OVERRIDE_GROUPS`).
- QA screens for all of the above in `qa_settings.rs` (rendered and looked at; no
  snapshots committed for them yet).

Not done yet (in order): server tab/admin view, first run, About, Debug log, image
editor selector stub, Finder Services in Info.plist (`scripts/build-app.sh`),
integration test (fake capture -> 8420 -> delete), snapshots of every new QA screen
(4 looks), docs (MACOS.md, STATUS.md), final report.

Decisions to carry into MACOS.md: screen source stored as upstream's `gdigrab`
(ScreenCaptureKit; ddagrab/screen-capture-recorder/"GDI grab" read as the screen);
F13/F14/F15 labels for Print Screen/Scroll Lock/Pause; hotkeys unregistered while one
is typed; personal folder fixed; backup without UploadersConfig (token in Keychain);
Lucide arrow for the missing right arrow of Inter; settings texts that name Windows or
a PC replaced from `assets/mac/l10n/Mac.resx`; no "Get recorder devices"; NVENC/AMF/QSV
not offered (VideoToolbox, bitrate only); printing and image effects are NotYet.

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
