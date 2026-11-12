# Bug hunt 2026-11-11: final (cross-area leftovers on the merged code, app)

Worktree `_wt/hunt-final`. The index holds every merged fix of the hunt; this pass is the
unstaged diff on top of it. Scope: the leftovers named in the brief (1 full-size
textures, 2 image effect presets of a hotkey, 3 the question queue, 4 the effects' output
size, 5 the backup import race) and the three added later by the coordinator (6 tool
windows placed every frame, 7 a second screen colour pick, 8 the PNG colour space
setting), each from the reports of the other areas.

Status: done (2026-11-12). 9 findings fixed (FIN-01 to FIN-09: 1 high, 4 medium, 4 low),
brief item 7 found already fixed (CAP-06), 4 left with a recommendation (section 3).

## 1. Files read

- [x] `docs/audit/hunt-2026-11-11/*.md` (every report of the hunt)
- [x] `modlight-app/src/media.rs`, `notification.rs` (UI-01), `dialogs/after_upload.rs`,
  `dialogs/after_capture.rs`, `dialogs/before_upload.rs`, `dialogs/clipboard_upload.rs`,
  `scrolling.rs`, `region_overlay.rs` (texture parts), `myspace/actions.rs`
  (`ask_confirm`, `confirm_finished`), `capture_flow.rs` (`screen_recording_refused`),
  `app_pages.rs` (backup import and reset), `tools/jobs.rs` (`OptionsHome`, CAP-04),
  `tools/image_effects.rs` (`image_settings_target` and its callers), `task_pages.rs`
  (`TaskTarget`)
- [x] `modlight-app/src/tools/mod.rs` (window hosting, `builder`, `draw_windows`),
  `tools/screen_picker.rs` (`start_screen_pick`, `tick_screen_pick`, CAP-06's test),
  `uploads.rs` (`Uploads::new`, `sync_upload_settings`), `settings.rs` (where a settings
  change syncs)
- [x] `modlight-core/src/backup.rs`, `hotkeys.rs` (`HotkeySettings::serial`),
  `image_decode.rs`, `tasks/image.rs`, `tasks/context.rs`
- [x] upstream `TaskHelpers.SaveImageAsStream`, `ImageHelpers.PNGStripChunks`,
  `CropImageEffect`, `ImageHelpers.Crop`
- [x] `modlight-effects/src/def.rs`, `sk/bitmap.rs`, `sk/canvas.rs` (allocation and
  `scale_pixels`), `fx/manipulations.rs`; the size math of every effect in `fx/filters_*.rs`
  that makes a canvas of another size than its source
- [x] `modlight-editor/src/ui/canvas.rs`, `ui/style.rs` (`paint_image`),
  `editor/effects_flow.rs`, `core.rs` (operations)
- [x] Every `load_texture` of `modlight-app` (outside `tools/`) and `modlight-editor`

## 2. Findings

| id | severity | file:line | what goes wrong | fix | test |
|---|---|---|---|---|---|
| FIN-01 | high | `modlight-app/src/dialogs/after_upload.rs:415`, `dialogs/after_capture.rs:513`, `dialogs/before_upload.rs:291`, `dialogs/clipboard_upload.rs:122`, `scrolling.rs:242` | Same crash as UI-01, in five more windows: each uploaded its picture whole as one texture. The app's GPU device takes at most 8192 px a side (egui-wgpu asks wgpu for exactly that); a larger texture fails wgpu's validation and its default handler panics, so the app ends as the window opens. Scenarios: the after-upload window over a 61 MP photo (9504 x 6336); the after-capture or before-upload window over a capture of three monitors; a clipboard upload of a large picture; the scrolling capture window with its result (up to 32768 px tall), right when the capture ends. | The four scaled previews use `media::texture_color_image` (the whole picture when one texture holds it, else shrunk to `max_texture_side`; nothing changes for pictures up to 8192 px). The scrolling preview shows the result at full size, so it is uploaded in parts of at most one texture each (`media::tiled_textures`) and drawn part by part: nothing is lost to scaling. The full picture is kept for Copy, Upload and the size badge. | `dialogs::{after_upload,after_capture,before_upload,clipboard_upload}::tests::a_picture_larger_than_a_texture_still_shows`, `scrolling::tests::a_result_taller_than_a_texture_shows_in_parts` (all five failed before on egui's assertion "Texture ... has size 9000x40, but the maximum texture side is 2048"; headless egui's limit is 2048) |
| FIN-02 | medium | `modlight-app/src/media.rs:79` (`decode_still`'s `shrink`), `media.rs:195` (`decode_animation`) | The search for other unbounded textures: My space's card thumbnails, the details drawer's preview and the account picture are decoded shrunk to a WIDTH only (288, 1024, 256 px); the moving previews follow `motion::plan`, which also bounds the width only. The server sends the original file when it cannot make a thumbnail (`render_thumbnail` returns `None` on a timeout, a decode error or a picture past `MAX_THUMBNAIL_PIXELS`), and a picture narrower than the asked width stays as tall as it is: a 200 x 10000 strip (or a narrow GIF) whose thumbnail timed out became a 200 x 10000 texture, and the main window ended when My space listed it. | Stills and animation frames are also fitted to one texture's height (`TEXTURE_SIDE`, 8192: decoded off the UI thread, where the context's own limit cannot be read). Pictures within the limit decode as before. | `media::tests::a_narrow_picture_taller_than_a_texture_is_fitted_to_one` (failed before: `[100, 10000]` instead of `[82, 8192]`) |
| FIN-03 | medium | `modlight-app/src/tools/image_effects.rs:2324` (`image_settings_target`) | The image effects presets of a hotkey's task were looked up as "the first hotkey with this job that overrides its image settings". With two such hotkeys (say two Print Screen keys with different presets), the second one's task applied the FIRST one's selected preset, its Image effects window (after capture, "Show image effects window") showed the first one's presets, and every edit there was written into the first hotkey: the second never got its own. Upstream's `ImageSettingsReference` is the very hotkey's settings. Reported by app-effects and app-capture (CAP-04 added the serial). | The hotkey is found by the serial its settings carry (`TaskSettings::hotkey` = `HotkeySettings::serial`, as `OptionsHome` does since CAP-04); settings that name no hotkey, or one no longer in the list, keep the old lookup by job. | `tools::image_effects::tests::presets_go_back_to_the_hotkey_they_came_from` (failed before: `Hotkey(0)` instead of `Hotkey(1)`) |
| FIN-04 | low | `modlight-app/src/capture_flow.rs:576` (`screen_recording_refused`), `qa.rs:932` | The last two `windows.confirm = Some(..)` outside tests. Both run only while no question is on screen (the Screen Recording question says itself in a message box when another question is up, as NET-07 kept it; the QA scene starts empty), and `confirm_finished` always brings the next waiting question up, so neither could drop one; they now go through `ask_confirm` like every other question, so a later change cannot lose one. | `ask_confirm(..)`. No behaviour changes. | None possible (no behaviour change); `capture_flow::tests` of the Screen Recording question still pass. The remaining assignments are in tests that set up "another question is on screen" on purpose. |
| FIN-05 | medium | `modlight-effects/src/fx/manipulations.rs` (`skew`, `scale`, `resize_image`, `resize_canvas`), `sk/bitmap.rs` (`check_size`, new), `modlight-editor/src/ops.rs:111` (`resize_canvas`) | app-effects NF-01 and app-editor's notes. The editor previews an effect on the UI thread at every change of a value, and some effects make a picture whose size grows with their values without bound: Skew at 89 by 89 degrees on a 1920 x 1080 screenshot asks for about 7 gigapixels (28 GB, 63795 x 111077); Scale at 500 % on a 4K screenshot 829 MB; Resize image with the aspect ratio kept on a long narrow picture a side in the millions (10 x 4000 at width 10000: 4 million px tall); Resize canvas up to 10000 px on each side, 1.8 GB on a plain screenshot. The largest fail to allocate (the app aborts) or make the Mac swap while the window hangs; the others make, on the UI thread, pictures the app itself could not open again. Apply of Resize canvas, done by the editor's core, allocated the same. | One shared limit: an effect may make a picture the app could open (`image_decode::fits`: 32768 px a side, 512 MiB), the limit EDIT-03 gave the editor's resize. `sk::check_size` refuses a larger one with an error before anything is allocated, called where the four effects allocate their result (the editor logs it and keeps its last preview); the editor's Resize canvas operation refuses it too (as its Resize image does). Effects that grow by a bounded border (frames, shadows, at most a few hundred px) and Rotate (at most twice the pixels) are left alone. | `fx::manipulations::tests::effects_refuse_a_result_larger_than_a_picture_may_be` (failed before: Skew of a 1 x 580 column at 89 degrees gave 33230 x 580, Scale 33000 x 5, Resize canvas 32800 x 1; the 7-gigapixel case is only run with the fix), `ops::tests::a_canvas_larger_than_a_picture_may_be_is_refused` (failed before: a 32800 px wide canvas was made) |
| FIN-06 | low | `modlight-effects/src/fx/manipulations.rs:566` (`crop_rect`) | `ImageHelpers.Crop` of an area with no part inside the image returned a picture of nothing (0 x 0), which the editor's canvas cannot show (EDIT-12: a texture of nothing is a wgpu validation error) and a task cannot upload. Its two callers (Crop image, which clamps to the picture as upstream's `CropImageEffect` does, and Auto crop) never pass such an area today, so nothing reached it; it is `pub`. | An empty area is an error (`Result`); the callers pass it on. | `fx::manipulations::tests::a_crop_outside_the_image_is_an_error` (failed before: `Ok` with a 0 x 0 picture) |
| FIN-07 | low | `modlight-app/src/app_pages.rs:2331` (`import_backup`), `modlight-core/src/backup.rs:175` (`import`) | app-ui's "backup import can lose to a save in the same frame": the worker wrote `ApplicationConfig.json` / `HotkeysConfig.json` and the settings were loaded again only when its completion ran on the UI thread. A save in between (the main window's bounds, saved a second after they stop changing and before the frame runs completions; a settings sync finishing in the same drain) wrote the old settings over the imported file, the load read them back, and the import was lost while the status line said "Backup imported". | `backup::import` is split into `read_import` (the archive read and every file parsed, on the worker; nothing written) and `write_import` (the auto-run checks against the settings as they are now, then the writes; two small documents), which the completion runs on the UI thread after saving the settings on screen and right before `reload_settings_files`. `import` is both in a row, as before. | `app_pages::tests::a_save_while_a_backup_imports_does_not_undo_it` (failed with the old flow: `UploadLimit` 3, the value from before the import, instead of 7); the backup tests of `modlight-core` (allow-list, byte order marks, auto-run checks) run through `import` unchanged |
| FIN-08 | medium | `modlight-app/src/tools/mod.rs:189` (`builder`) | Brief item 6, the tools' twin of UI-07: every tool window (image effects, image viewer, combiner, colour picker, OCR, QR, metadata, video tools, history...) was given its position in every frame: centred on `visible_frame()` (on a Mac the screen of the KEY window), or at its remembered place. egui's `ViewportBuilder::patch` sends `OuterPosition` whenever that position changes, so on a Mac with two screens a tool window dragged to the other screen jumped to the middle of it as soon as it took the key, and one with a remembered place was put back whenever that place was written while it moved. | A window is placed only while it is not open yet (`app::is_open`, as `app::builder` does for the dialogs since UI-07); the full-screen overlays (`spec.screen`) keep covering their display. | `tools::tests::a_tool_window_is_placed_once_as_it_opens` (failed before: the open window was given its position again) |
| FIN-09 | low | `modlight-app/src/uploads.rs:164` (`TaskContext::strip_png_color_space`), `modlight-core/src/tasks/image.rs:20` (`prepare_image`) | Brief item 8: "Strip color space information from PNG images" (`PNGStripColorSpaceInformation`, Settings, Advanced) did nothing: the flag the tasks read was created `false` and never set from the setting, and `prepare_image` ignored it anyway (its comment: this encoder writes no colour chunks). Upstream's `SaveImageAsStream` strips gAMA, cHRM, sRGB and iCCP from every PNG a task makes when it is on. | `sync_upload_settings` (run at start, after every settings change, after an import, a reset and a settings sync) sets the flag from the setting; `prepare_image` strips the four chunks from a PNG when it is set, before the size is weighed for the automatic JPEG, as upstream does (`strip_png_color_space_chunks`, `PNGStripChunks`). The `image` crate's encoder writes none of these chunks today, so the pictures do not change; the setting now holds whatever the encoder writes. | `uploads::tests::the_png_colour_space_setting_reaches_the_tasks` (failed before: the flag stayed off), `tasks::image::tests::a_png_loses_its_colour_space_chunks_when_asked` |

Brief item 7, `tools/screen_picker.rs` `start_screen_pick` while `screen_pick_busy`:
already fixed by CAP-06 (in the index). The guard refuses a pick while the picker is open,
a pick is pending or the desktop is being taken, and hands the refused target "no colour"
so a window that stepped aside comes back; `pending_pick` is set nowhere else (the QA scene
sets the picker directly). Its test,
`tools::screen_picker::tests::a_pick_asked_while_another_is_under_way_gives_the_window_back`,
passes. Nothing changed.

Every other `load_texture` of `modlight-app` (outside `tools/`) and `modlight-editor` was
checked: the region overlay (`region_overlay.rs:207`, fitted to `max_texture_side`), its
magnifier, the editor's canvas (`ui/canvas.rs:512`, fitted), its overlay (the visible part
of the view), the editor's small pictures (`ui/style.rs` `paint_image`: emoji, cursor,
arrow and font previews of a few dozen pixels), the about and title logos, the checkered
pattern and QA-only pictures are bounded.

## 3. Not fixed

- **NF-1 (low) An open Image effects window keeps its hotkey by position.** FIN-03 picks
  the right hotkey when the window opens, but the window then holds
  `TaskTarget::Hotkey(index)`. If a hotkey before it is removed or moved in Settings while
  it is open (the hotkey settings window follows such changes, `hotkey_window_follows`;
  tool windows do not), or the hotkeys are read again (an import, a reset, a settings
  sync), its edits are written into whichever hotkey now has that place. Upstream holds a
  reference (edits to a removed hotkey go nowhere). Recommendation: carry the serial in
  `TaskTarget::Hotkey` (it is also held by the FFmpeg options window and the custom region
  picker) and write nothing when that hotkey is gone, as `OptionsHome` does since CAP-04.
  Not done here: `TaskTarget` is used by the settings pages, the hotkey window and the
  region picker, more than this pass should touch.
- **NF-2 (low) FIN-03 falls back to the job when the serial is gone.** Settings whose
  hotkey is no longer in the list (read again between the task's start and its image
  effects step) keep the old lookup by job, as before; CAP-04's `OptionsHome` writes
  nothing then. Same recommendation as NF-1.
- **NF-3 (info) Effects that grow by a bounded border are not checked.** Frames, borders
  and shadows add at most a few hundred pixels (their sliders stop there), and Rotate makes
  at most twice the pixels of its source; on a picture already at the limit their result
  can pass it by that much. The classic effects (after-capture presets) keep upstream's own
  limit, GDI+'s 2 GiB (`refused_by_gdi`), which is larger than FIN-05's 512 MiB: a preset
  can still make a picture the app could not open again. Recommendation: if that matters,
  call `sk::check_size` in `refused_by_gdi`'s size math too (it would refuse presets
  upstream runs).
- **NF-4 (low) The editor applies its last good preview after a refused one.** With an
  effect dialog open, a value the effect refuses (FIN-05, or any effect error) leaves the
  last good preview on the canvas (EDIT-12), and Apply then applies that preview, not the
  values in the dialog. What is applied is what is shown, so it is left as it is;
  recommendation: disable Apply while the last preview failed.

## 4. Outside my area

- `tools/mod.rs` (FIN-08) and `tools/image_effects.rs` (FIN-03) are files of the tools
  area: the change in `builder` is a few lines and keeps the function's shape (the tools
  branch adds a `maximized` field to `WindowSpec`; no `WindowSpec` literal was added, the
  test sets `position` on `WindowSpec::fixed`). Merge note: `builder` now takes the
  context and the viewport id.
- `modlight-core/src/tasks/image.rs` (FIN-09) and `modlight-core/src/backup.rs` (FIN-07)
  belong to app-net and app-ui; `modlight-editor/src/ops.rs` (FIN-05) to app-editor.

## 5. Checks

Run at the end in `app/` on the whole diff (`CARGO_BUILD_JOBS=6`, `nice -n 10`):

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings` (macOS): clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (with `wincc/env.sh`): clean (only the build script's note that `Modlight.exe`'s
  resources are embedded on Windows builds only). No Windows-only code was changed.
- `cargo test --workspace --no-fail-fast`: 45 suites, 1057 passed, 0 failed, 4 ignored (the
  existing ones). Per crate: `modlight-app` lib 179, `modlight-core` lib 446,
  `modlight-editor` lib 86, `modlight-effects` lib 40 plus its golden (3), edge-input (3)
  and classic-edge (2) suites, `modlight-api` lib 56; the tests against the local
  reference servers (`reference_server` 12, `reference_app` 2 and the others) reached
  them and passed; the snapshot suites (`snapshots`, `ui_snapshots`) passed.
- Every new test was run against the code without its fix (the fix taken out for the
  run, then put back and checked) and failed there, with the reasons given in section 2;
  except FIN-04 (no behaviour change) and the new `strip_png_color_space_chunks` of
  FIN-09 (a new function; the wiring test of FIN-09 did fail). The 7-gigapixel Skew of
  FIN-05 is only run with the fix in place (without it the test would allocate 28 GB).
- Snapshot pictures: none changed; no `*.old.png`, `*.new.png` or `*.diff.png` left.
- No database migrations; no server or web files touched.
