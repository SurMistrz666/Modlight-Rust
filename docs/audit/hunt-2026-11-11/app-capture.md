# Bug hunt 2026-11-11: app-capture

Area: capture, region overlay, screen recording, hotkeys, tasks and the macOS/Windows platform
glue of the desktop app (`app/`). Worktree `_wt/hunt-app-capture`; the work is the unstaged diff.
Every fix has a test that fails without it, unless said otherwise.

## 1. Files read

Docs: `app/docs/PORT.md`, `docs/audit/logic-2026-10-24/{04-capture,05-recording,10-windows}.md`,
the app parts of `docs/audit/rust-app.md` and `rust-delta-2026-10-24.md`, `app/docs/WINDOWS.md`
(capture rows).

- [x] modlight-core/src/capture/{mod, geometry, options, region, scrolling, source}.rs
- [x] modlight-core/src/recording/{mod, args, ffmpeg, options, session}.rs
- [x] modlight-core/src/tasks/{mod, actions, context, gate, host, image, info, manager, settings,
      worker, naming, paste, upload_manager}.rs
- [x] modlight-core/src/tasks/{recent, uploader}.rs (skimmed: the tray text and the upload
      transport, which app-net reviews with `upload/`)
- [x] modlight-app/src/{capture_flow, capture_windows, region_overlay, recording_ui, frame_strips,
      scrolling, hotkey_page, tray, commands, strip, menus, qa_capture}.rs
- [x] modlight-app/src/ffmpeg_window.rs (state and device logic in full; drawing skimmed)
- [x] modlight-app/src/dialogs/{after_capture, auto_capture, quick_task_menu}.rs
- [x] modlight-app/src/dialogs/quick_task_editor.rs (logic in full; drawing skimmed)
- [x] modlight-app/src/tools/{screen_picker, region}.rs
- [x] modlight-macos/src/{capture, keys, hotkeys, windows, system, input, ax}.rs
- [x] modlight-windows/src/{keys, hotkeys, windows, input, ax, duplication_plan}.rs,
      capture/{mod, desktop, gdi, winlist, frames, duplication, transparent}.rs
- [x] modlight-windows/src/system.rs (skimmed: shell, recycle bin and locale helpers, reviewed in
      full by the 2026-10-24 Windows pass; the capture area uses only `last_error_message`)
- Also read for the leads: modlight-core/src/hotkeys.rs (manager, resolving), tools/jobs.rs,
  uploads.rs (entry points), upstream `TaskHelpers.ExecuteJob`, `HotkeyForm`, `QuickTaskMenu`,
  `ScrollingCaptureWindow`, `AutoCaptureWindow`, `ScreenRecordManager`, `ScreenRecordWindow`,
  `WorkerTask`, `ExternalProgram`, `CaptureHelpers/*`, `FFmpegOptionsWindow`.

## 2. Findings

### CAP-01, medium (data loss): "Perform actions" deletes its own output on a Mac

`modlight-core/src/tasks/actions.rs:149` (`same_path`). An external program whose output
extension differs from the input's only in case (input `shot.png`, `OutputExtension` `PNG`)
writes `shot.PNG`, which on a Mac's usual case-insensitive volume is the same file. The port
compared the two paths case-insensitively only on Windows, so on a Mac the input was handed
over for deletion (`DeleteInputFile`) and `do_file_jobs` deleted it: the program's output, the
only copy of the capture, was gone, and the upload that followed read a missing file. Upstream
compares with `OrdinalIgnoreCase` on every system.
Fix: compare without case everywhere (on a case-sensitive volume the input is then kept, as
upstream keeps it).
Tests: `tasks::actions::tests::an_output_differing_only_in_case_is_never_deleted`,
`a_program_whose_output_is_its_input_keeps_the_file` (real `/bin/cp` in a temp folder).

### CAP-02, medium: the quick task menu closed in its first frame after a hotkey capture

`modlight-app/src/capture_windows.rs:146` (lead N3 of 2026-10-24, confirmed by reading winit
0.30.13 and egui-winit 0.36.2). The menu closed whenever its viewport reported
`focused == Some(false)`. egui-winit fills that every frame from `window.has_focus()`, which on a
Mac is `isKeyWindow`; winit makes a new window key with `makeKeyAndOrderFront` without activating
the app, and a window of an inactive app is never key. A capture started by a hotkey (Modlight
not in front, no overlay that would have activated it: full screen, window, monitor, last region)
with "Show quick task menu" on therefore lost its picture: the menu closed with no choice in the
first frame, before anybody saw it. Upstream activates its placement window
(`placementWindow.Activate()`) and closes on a click elsewhere.
Fix: the menu asks for the focus (`ViewportCommand::Focus`, which activates the app) in its first
frame, and only a loss of focus after it had the focus closes it (`QuickTaskMenu::focus_lost`).
Escape and Cancel close it as before. Not seen on a real screen (no windows allowed here).
Test: `dialogs::quick_task_menu::tests::the_menu_stays_until_it_had_the_focus_and_lost_it`.

### CAP-03, medium: `HotkeyRepeatLimit` was never read

`modlight-app/src/capture_flow.rs` (`pump_hotkeys`, now `system_hotkeys_pressed`). Upstream's
`HotkeyForm.CheckRepeatLimitTime` drops every `WM_HOTKEY` that comes less than
`HotkeyRepeatLimit` ms (default 500, setter floor 200) after the last one taken, whichever key.
The port shows the setting in Application settings but ignored it: a key pressed twice in a
hurry ran its job twice (two region captures, the second taking a picture of the first overlay
and replacing it; a recording hotkey started and at once stopped the recording).
Fix: `RepeatLimit` in `Captures`, applied to the system's presses before the full-screen rule
(QA's simulated presses are not system presses and are not limited).
Test: `capture_flow::tests::hotkey_presses_closer_than_the_repeat_limit_are_dropped`.

### CAP-04, medium: two hotkeys with the same job wrote their options into the first one

`modlight-app/src/tools/jobs.rs:22` (`OptionsHome`), `modlight-core/src/hotkeys.rs`
(`HotkeySettings`), `modlight-core/src/tasks/settings.rs` (`TaskSettings`). A window's changes to
a group a hotkey overrides (scrolling capture options, the magnifier zoom, tool options, the
colour picker...) went to the FIRST hotkey running the same job that overrides the group, so
with two such hotkeys (say two "Scrolling capture" keys with different capture settings) the
second one's edits changed the first one's settings and never its own. Upstream writes to the
very hotkey (`TaskSettingsReference`).
Fix: every `HotkeySettings` gets a run-time `serial` (not saved; equality ignores it), the
settings resolved from it carry it (`TaskSettings::hotkey`, `#[serde(skip)]`), and `OptionsHome`
finds the hotkey by it; settings that name no hotkey keep the old lookup by job. A window opened
before the hotkeys were read again (an import, a reset, a synced document) writes nothing, as
upstream's reference then points at a hotkey object that is no longer in the list.
Test: `scrolling::tests::options_go_back_to_the_hotkey_they_came_from`.
`tools/image_effects.rs:2317` (`image_settings_target`, app-effects) has the same first-by-job
lookup: see section 4.

### CAP-05, low: a second area selection replaced the overlay that was open

The port has one overlay at a time (`capture_with`, the recording hotkey (F12), the tools and the
custom-region picker refuse while one is open or being prepared), but three ways in did not ask:
- `modlight-app/src/scrolling.rs:543` (`open`): upstream's `StartStopAsync` starts a selection
  only `if (!_captureOperation)`. Pressing the scrolling capture key again while its area was
  being chosen took the desktop again (with the overlay on it) and replaced the overlay; pressed
  over another capture's overlay it threw that capture away.
- `modlight-app/src/capture_flow.rs` (`tick_captures`): a capture scheduled with a delay came
  due while an overlay (another capture's) was open and was taken anyway: a picture of the
  overlay, or for a region kind a second overlay in place of the first, its selection lost.
  Now it waits until the overlay closes (upstream would stack a second overlay; the port
  serialises them).
- `capture_flow.rs` (`auto_capture_action(SelectRegion)`): the same replacement.
Tests: `scrolling::tests::no_second_selection_while_the_screen_is_taken`,
`capture_flow::tests::a_capture_that_comes_due_waits_for_the_open_overlay`,
`capture_flow::tests::the_auto_capture_area_waits_for_the_screen`.

### CAP-06, low: a second screen colour pick replaced the first and left a window hidden

`modlight-app/src/tools/screen_picker.rs:375` (`start_screen_pick`). The guard looked at the
picker and the pending pick, not at `screen_pick_busy` (the desktop being taken). A pick asked in
that moment (the hotkey, another colour picker's pipette) started a second picker that replaced
the first one: the first target never heard back, so a colour picker window that had minimised
itself for its pick stayed hidden. A pick that is refused (also while one is open) now hands its
target "no colour", which brings that window back.
Test: `tools::screen_picker::tests::a_pick_asked_while_another_is_under_way_gives_the_window_back`.

### CAP-07, medium: six upload hotkeys ignored their own task settings (lead N1)

`modlight-app/src/capture_flow.rs` (`execute_job`), `uploads.rs` (app-net's file, small changes).
Upstream's `ExecuteJob` hands `safeTaskSettings` to `UploadFile`, `UploadFolder`,
`ShowTextUploadDialog`, `UploadURL`, `OpenDropWindow` and `ShowShortenURLDialog`; the port called
them without settings, so a File/Folder/Text/URL/Drag-and-drop upload or Shorten URL key whose
task has its own after-upload tasks, destinations, folder pattern... uploaded with the defaults.
Fix: `pick_and_upload_with` / `pick_folder_and_upload_with` take the settings (the old names
call them with the defaults); the four windows keep the hotkey's settings in
`Captures::upload_jobs` while they are open (`None` when opened from a menu, cleared when the
window goes) and `text_upload_finished`, `url_upload_finished`, `shorten_url_finished` and
`drop_window_outcome` use them. A window already open from a menu stays with the defaults.
Test: `capture_flow::tests::upload_hotkeys_upload_with_their_own_settings` (text window and drop
window by hotkey and from the menus). The file and folder pickers are not testable headlessly
(QA never opens a native panel); their change is the settings argument only.

### CAP-08, low (Windows): the monitor list gave sizes in points (W10-9)

`modlight-app/src/capture_flow.rs` (`monitor_entries`), `modlight-core/src/capture/source.rs`
(`ScreenInfo`), `modlight-windows/src/capture/mod.rs`. The capture menu and the tray listed
"1. 1707x960" for a 2560x1440 monitor at 150 %; upstream (per-monitor DPI aware) lists pixels.
Fix: `ScreenInfo::pixel_size` (serde default `None`), filled by Windows from the monitor's
physical bounds; the label uses it, else the points (a Mac keeps points, its own convention).
`app/docs/WINDOWS.md` updated.
Test: `capture_flow::tests::the_monitor_list_gives_pixels_where_the_platform_has_them`.

### CAP-09, low (data loss): a second quick task menu took the first one's picture

`modlight-app/src/capture_flow.rs` (`run_image_task`). The port keeps one quick task menu; a
second picture with "Show quick task menu" (another capture key pressed while the first menu
was open) replaced it, and the first picture was dropped without a word. Upstream makes a menu
per picture (`new QuickTaskMenu()` in `RunImageTask`). Now the second picture waits
(`Captures::quick_menu_waiting`) and its menu opens when the first closes.
Test: `capture_flow::tests::a_second_quick_task_menu_waits_for_the_first`.

### CAP-10, low: "URL regex replace" read `$1x` as a group named `1x`

`modlight-core/src/tasks/naming.rs:151` (`regex_replace`, the after-upload URL rewrite). The
replacement is a .NET one (`Regex.Replace`), where `$` and its digits name a group and stop
there; the regex crate reads every letter, digit and `_` after `$` as one group name. A
replacement such as `https://cdn.example/$1_thumb` gave `https://cdn.example/`: the link copied
to the clipboard and kept in the history lost its path. Now numbered references are rewritten as
`${n}` first; `$$`, `${name}` and a lone `$` keep their meaning.
Test: `tasks::naming::tests::url_replacements_read_group_numbers_as_dotnet_does`.

### CAP-11, low (Windows): recording the active window ignored "Capture client area"

`modlight-app/src/recording_ui.rs` (`RecordStartMethod::ActiveWindow`). Upstream's
`ScreenRecordManager` records `GetActiveWindowClientRectangle()` when `CaptureClientArea` is on;
the port always recorded the window's rectangle, title bar and frame included.
Fix: `CaptureSource::active_window_client_rect` (default `None`; Windows: the foreground
window's client rectangle in points, none for the app's own windows), used when the setting is
on. A Mac has no client area and records the window as before.
Test: `recording_ui::tests::the_active_window_recording_keeps_to_the_client_area`.

## 3. Not fixed

- **See-through windows render black (known, renderer not changed).** Where it bites here:
  the quick task menu (`capture_windows.rs`, `with_transparent(true)`: black corners and shadow
  area around its rounded frame) and the "transparent" region kind (`overlay_viewports`,
  `with_transparent(kind == Transparent)`), used by "Capture region (transparent)" and by
  recordings with `ScreenRecordTransparentRegion`: the overlay covers every display in black,
  so the area is chosen blind. A real fix needs a see-through surface per viewport: on a Mac a
  non-opaque `NSWindow` with a clear background and `CAMetalLayer.opaque = false` with a
  premultiplied composite alpha mode; on Windows a DirectComposition swap chain
  (`CreateSwapChainForComposition`), since a swap chain made from an `HWND` is opaque. Without
  renderer work, the transparent kind could show the frozen desktop as the light kind does and
  still take its area live after the overlay closes (`region_picture` would then capture for
  `RegionKind::Transparent` whatever `request.image` holds). Recommendation: that fallback.
- **Mac recording of an area across two displays** (`modlight-macos/src/capture.rs`,
  `MacFrames`): only the display holding most of the area is streamed, its part scaled to the
  whole area's frame size (`setScalesToFit(true)`), so the other display's part is missing and
  the picture is stretched or boxed. Upstream's default `gdigrab` records across monitors.
  A fix needs one stream per display composed into the frame (as the Windows `Duplication`
  plan does); too large for this pass. Recommendation: at least size the frame to the part.
- **A new recording can start while the last one's after-capture window is open**; upstream's
  `IsRecording` stays true until `FinishRecording`. Harmless (each recording has its own file).
- **StopScreenRecording lead: checked, no bug.** `recording_stop` acts only while recording and
  then only stops (`start_stop` from `Recording` sets `Stopped`); a waiting or paused recording
  is neither started nor resumed, and later presses do nothing. Upstream's repeated `Close`
  (a third press killing a stuck ffmpeg) is the Abort key's in the port (F1).
- **`modlight-windows` `visible_frames()` lead: checked by reading, correct**: physical
  working areas (`monitors()` enters per-monitor DPI awareness itself) turned into the app's
  points with the primary monitor's scale, edge by edge, the primary monitor first, as the Mac's.

## 4. Outside my area

Files of other areas this pass changed, because the fixes needed them (small, listed for the
merge):
- `modlight-app/src/uploads.rs` (app-net): `pick_and_upload_with`,
  `pick_folder_and_upload_with`, and the hotkey's settings read in `text_upload_finished`,
  `url_upload_finished`, `shorten_url_finished`, `drop_window_outcome` (CAP-07).
- `modlight-app/src/tools/jobs.rs` (app-tools): `OptionsHome::Hotkey` carries the hotkey's
  serial (CAP-04).
- `modlight-core/src/hotkeys.rs` (app-ui, top-level core file): `HotkeySettings::serial`, a
  hand-written `PartialEq` without it, `task_settings` sets `TaskSettings::hotkey` (CAP-04).
- `app/docs/WINDOWS.md`: the monitor list row (CAP-08).

Bugs seen, not fixed here:
- `modlight-app/src/tools/image_effects.rs:2317` (`image_settings_target`, app-effects): the
  same first-hotkey-with-this-job lookup as CAP-04, for image effect presets edited from a
  hotkey's task. With CAP-04 in, it can match `h.serial == settings.hotkey` first.

## 5. Checks run at the end

All in `app/`, with `nice -n 10` and `CARGO_BUILD_JOBS=6`:
- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings` (no-op C
  compiler of `wincc/env.sh`): clean (only the usual "resources embedded only when building on
  Windows" note). Windows code compiled, not run.
- `cargo test --workspace`: 45 test binaries, 1005 passed, 0 failed, 4 ignored (modlight-app lib
  155, modlight-core lib 438, snapshots, ui_snapshots, `reference_server` 12 and `capture_upload`
  against the local reference servers all green).
- No snapshot pictures changed; no `*.old.png` / `*.diff.png` left. No migration.
