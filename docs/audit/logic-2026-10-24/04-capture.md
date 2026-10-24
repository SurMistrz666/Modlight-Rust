# Logic review 2026-10-24, area 04: capture, hotkeys, tray, notices, menus

Branch `logic-04-capture` (from main `28916b4`). Scope: `modlight-core/src/{capture/*,
hotkeys.rs}`; `modlight-app/src/{capture_flow, capture_windows, region_overlay, scrolling,
hotkey_page, qa_capture, notification, tray, menus, commands, strip}.rs` and
`dialogs/{after_capture, auto_capture, quick_task_menu, quick_task_editor}.rs`;
`modlight-macos/src/{capture, hotkeys, keys, input, ax, windows}.rs`. Read against upstream
`ShareX/CaptureHelpers/*`, `ShareX.ScreenCaptureLib` (region capture window and light form,
`ScrollingCaptureManager`, scrolling window, `RegionCaptureTasks`), `HotkeyManager`,
`HotkeySettings*`, `TaskSettings`, `TaskHelpers`, `UploadManager.RunImageTask`,
`QuickTaskMenu*`, `AutoCapture*`, `AfterCapture*`, `Notification*`, `MainMenuBuilder`.

Every fixed finding has a test written first and seen failing on the old code.

## Findings

| Id | Severity | Where | What was wrong | How it was reproduced | Status |
|---|---|---|---|---|---|
| C1 | High | `modlight-app/src/hotkey_page.rs:1057`, called from `capture_flow.rs` `pump_hotkeys` | Typing a shortcut unregisters every hotkey and sets `ignore`. The capture ended only from the page's own frame, so closing the settings window or showing another page before a key came left every global hotkey unregistered and ignored until the app restarted. Upstream ends it on lost focus and on `Dispose` (`StopCapture`). | `leaving_the_page_while_typing_a_key_brings_the_hotkeys_back`: after `close_settings`, `ignore` stayed true and nothing was registered. | Fixed, `97fa1fc` |
| C2 | Low | `hotkey_page.rs:1095` (`stop_hotkey_capture`) | While hotkeys are disabled, ending a key capture skipped `register_all`, so the "Disable hotkeys" key itself (which `RegisterHotkey` takes even then, and the capture had unregistered) stayed unregistered: hotkeys could no longer be turned on with it. | `the_disable_hotkeys_key_stays_registered_after_typing_a_key`: 0 keys registered instead of 1. | Fixed, `97fa1fc` |
| C3 | Low | `hotkey_page.rs:1202` (Reset) | Reset while hotkeys were disabled left them disabled; `HotkeyManager.ResetHotkeys` turns them on (`TaskHelpers.ToggleHotkeys`). | `resetting_disabled_hotkeys_turns_them_on` | Fixed, `97fa1fc` |
| C4 | Medium | `modlight-app/src/tray.rs:46` (`MenuCommands`), `:524` | Menu ids were positions in the latest command list, and the menu is rebuilt whenever an entry changes (the capture menu lists every window with its title and frame, recent items, toggles). A click on the open menu resolved after a rebuild ran whatever stood at that position now: another after-capture task toggled, another recent link opened, Settings becoming Disable hotkeys. | `a_click_on_a_replaced_menu_runs_what_it_showed`: an id of the first menu looked up after a rebuild with one more entry before it gave `ToggleHotkeys` instead of `ShowSettings` with the positional lookup. | Fixed, `7b1a14b` (ids name menu and place; the last 32 menus answer) |
| C5 | Medium | `modlight-core/src/hotkeys.rs:720` (`resolve_task_settings`) | A hotkey with its own settings kept the defaults' `UseDefault...` flags after resolving (`GetSafeTaskSettings` copies the hotkey's). `OptionsHome::of`/`of_capture` and `image_settings_target` read them, so colour picker, tool options and image effect presets edited from such a hotkey were written to the default task settings. | `a_hotkeys_own_flags_survive_resolving` | Fixed, `308593f` |
| C6 | High (data loss) | `modlight-app/src/capture_flow.rs:869` (`rename_recording`) | A name typed in the after-capture window renames the recording with `rename(2)`, which silently replaced a file of that name (an earlier recording). Upstream's `File.Move` refuses, says why and keeps the recording's name. | `renaming_a_recording_never_replaces_another_file`: `demo.mp4` was overwritten. | Fixed, `afe9189` (refused with upstream's `FileHelpers_Rename_file_error` message; case-only renames are no rename) |
| C7 | Medium | `modlight-macos/src/windows.rs:42`, `capture.rs:65` | Cocoa rectangles were turned to top-left desktop points with the height of `mainScreen` (the screen with the key window; capture working areas) or of the screen itself (`visible_frame`, used for notices, dialogs and the drop window). Correct is the primary display's (`NSScreen.screens[0]`). With a display of another height, e.g. 1920x1080 beside a 900 pt MacBook, everything was 180 pt too low, notices partly off screen. | `visible_frames_are_measured_from_the_primary_display`: 205 instead of 25 for the external display. | Fixed, `0fc2a48` |
| C8 | Medium | `modlight-core/src/capture/mod.rs:147-171` | `ShowCursor` (on by default) never reached a region capture: the frozen desktop was taken without the pointer, macOS has no separate pointer picture, and Windows' was only drawn by the overlay, never into the result. Upstream inserts the pointer into the overlay's workspace (in the result), freezes the light capture with it and takes the transparent capture's area with it. | `region_captures_show_the_pointer_when_the_task_does`, `the_pointers_own_picture_goes_into_the_frozen_desktop`, `a_region_asked_for_by_a_tool_has_no_pointer` | Fixed, `557bc90`. Areas chosen for something else (tools, recording, scrolling, preconfigured region) stay pointer-free as `RegionCaptureTasks` takes them. The pointer is now part of the frozen picture (upstream: an annotation that can be moved); the port's overlay never let it be moved. |
| C9 | Low | `capture_flow.rs:504`, `capture/mod.rs:386` | A tool's region picture (OCR, QR, pin, analyze) went through `finish_region`, which records the last region, so the next "Last region" capture repeated the tool's area. | `a_tools_area_is_not_the_last_region` | Fixed, `59b5667` (`capture::region_picture`) |
| C10 | Medium | `modlight-app/src/scrolling.rs:710` | OK in the scrolling capture options changed only the open window; upstream edits `CaptureSettingsReference.ScrollingCaptureOptions`, so they were lost when the window closed. | `saved_options_are_kept_in_the_settings` (defaults and a hotkey with its own capture settings) | Fixed, `65ea7c2` |
| C11 | Medium | `capture_flow.rs:537` | Cancelling the scrolling capture's area selection (Escape, right click) left its window hidden for good: `scrolling::selection_cancelled` had no caller. Upstream shows "Selection cancelled" and restores the window. | `a_cancelled_scrolling_selection_brings_the_window_back` | Fixed, `abbd269` |
| C12 | Low | `capture_flow.rs:562` | The magnifier zoom set with the wheel (and the magnifier it showed) was dropped when the overlay closed; upstream's overlay edits `CaptureSettingsReference.SurfaceOptions` itself. | `the_magnifier_zoom_is_remembered` | Fixed, `bd9efd7` |
| C13 | Low | `modlight-core/src/capture/region.rs:439` | The light and transparent captures used the default overlay's exclusive rectangle; `RegionCaptureLightForm` uses `CaptureHelpers.CreateRectangle`, which counts both corner pixels. Results were one pixel short each way and a 6 px drag was no area. | `the_light_area_includes_both_corners` | Fixed, `a913223` (snapshot `region-capture-light` updated: right and bottom edge, 1 px) |
| C14 | Low | `capture_flow.rs:831` | Window titles in the capture menu were cut to 50 characters plus "..." (53); `Truncate(50, "...")` keeps 47. | `long_window_titles_are_cut_to_fifty_characters` | Fixed, `87db390` |
| C15 | Low | `modlight-app/src/commands.rs:215` | "Edit this menu..." opened the editor and left the quick task menu (and its picture) open until its window happened to lose focus; every upstream menu item closes the menu first, dropping the picture. | `editing_the_quick_task_menu_closes_it` | Fixed, `1ab87c5` |
| C16 | Low | `capture_flow.rs:206` | The "Disable hotkeys" job used the default task settings for its sound and notice; `ExecuteJob` passes the hotkey's safe settings. | `the_disable_hotkeys_key_follows_its_own_settings` | Fixed, `bfa1526` |
| C17 | Low | `modlight-app/src/settings.rs:649` (outside the area, one line) | Failed hotkeys were never retried (`register_failed` had no caller). Upstream's settings window builds `HotkeySettingsAvaloniaService`, which calls `RegisterFailedHotkeys`; a key another app held at start stayed red until a restart. | `opening_the_settings_tries_failed_keys_again` | Fixed, `e7ee995` |
| N1 | Medium | `capture_flow.rs:234-241` (`execute_job`) | The upload jobs ignore the hotkey's own task settings: FileUpload, FolderUpload, UploadText, UploadURL, DragDropUpload and ShortenURL run with the defaults; upstream passes `safeTaskSettings` to each. | Read against `TaskHelpers.ExecuteJob`; `pick_and_upload`, `pick_folder_and_upload`, `open_text_upload`, `open_url_upload`, `open_drop_window`, `open_shorten_url` take no settings. | Not fixed: the upload windows in `uploads.rs` (another area) have to carry task settings through. |
| N2 | Low | `modlight-app/src/uploads.rs:677` (another area) | A notice click action "Annotate image" on a file that is not a picture shows the editor's "cannot load" notice; upstream checks `FileHelpers.IsImageFile` first and does nothing. | Read against `NotificationWindow.ExecuteAction`. | Not fixed: other area, cosmetic. |
| N3 | Low, unverified | `capture_windows.rs:97` | The quick task menu closes when its viewport reports `focused == Some(false)`. egui-winit initialises `focused` from `window.has_focus()`; a window created while another app is active (a hotkey capture) may not be key in its first frame, which would close the menu at once. | Reasoned from egui-winit 0.36.2 `update_viewport_info`; needs a real window. | Not fixed: cannot be checked headlessly; listed for the real-window checks. |

## Shared files touched outside the area

- `modlight-app/src/tools/region.rs` (`tool_region`): asks `capture::region_request` instead
  of running a region capture, so tool pictures stay pointer-free after C8 (part of `557bc90`).
- `modlight-app/src/settings.rs` (`show_settings`): one call to `register_failed` (C17,
  `e7ee995`).
- `modlight-core`: `RegionRequest.cursor` is gone (replaced by `show_cursor: bool`);
  `capture::region_picture` is new; `ScrollingCaptureOptions` derives `Serialize`.
  `region_request` no longer attaches a pointer picture, so on Windows the recording and
  tool overlays no longer draw the pointer (upstream's `RegionCaptureTasks` have none).

## Checked and OK

- Region selection state machine against `RegionCaptureWindow`: press, move, release,
  pending hover, right-button cancel during creation and the suppressed release, handles,
  move and resize, fixed size, keyboard (input delay, Enter, Space, tilde, arrows with
  Shift, Delete, Ctrl+H), Escape, double click, side buttons and their actions,
  `Complete`'s rounding and its desktop rectangle at 2x and with negative origins, window
  detection front to back, `FindTopLevelWindowInfo`, magnifier normalization.
- Captures against `CaptureHelpers`: fullscreen and monitor metadata, custom region and
  window (`SearchWindow`), last region and its fallbacks per kind, `ImageEffectOnlyRegionCapture`
  and the annotation drop after a modified capture, delay and auto-hide timing, capture
  sound, `ToastWindowAutoHide`.
- macOS capture: multi-display composition at the highest scale, per-display source rects,
  frame copy bounds; hotkey key and modifier mapping to Carbon; posted scroll and key codes.
- Scrolling: `CombineImages` (ignore side, auto bottom edge, match search, best guess,
  status) and the loop's stop conditions match `ScrollingCaptureManager`.
- Auto capture: first shot after 1 s, wait-for-uploads retry every second, repeat time,
  start and stop from hotkeys, stop on close, minimize.
- Hotkeys: Newtonsoft `Keys` text round trip, numeric jobs (enum order matches), labels,
  defaults, register, unregister, toggle, move with wrap, reset, failed list,
  `DisableHotkeysOnFullscreen`, the open hotkey window following moves and removals.
- After-capture window (Copy, Continue, Cancel, destinations, file name), quick task menu
  presets and choices, the quick task editor, `RunImageTask` order (menu, window, task) and
  the after-capture job order in the worker.
- Notices: timing, fade, hover hold, placement in the area, `CanExecute`, copy-before-dismiss,
  click actions per button; tray entries, progress icon, recent items; menus; commands.

## Verification

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test -p modlight-core --lib` (379), `-p modlight-macos --lib`, `-p modlight-app --lib`
(56), `-p modlight-app --test snapshots` (every screen and look) and `-p modlight-app --test
capture_upload` against the reference server (its own uploads, deleted afterwards) all pass
on macOS. A Windows `cargo check` could not run here (C dependencies need MSVC).
