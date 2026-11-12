# Bug hunt 2026-11-11: app-tools (the ShareX tools of the desktop app)

Scope: `app/crates/modlight-tools/**` and `app/crates/modlight-app/src/tools/**` without
`screen_picker.rs`, `region.rs` (app-capture), `image_effects.rs` (app-effects) and
`editor_hooks.rs` (app-editor). Spec: upstream `ShareX.Tools`, `ShareX.HistoryLib`,
`ShareX.HelpersLib` (ImageHelpers, Presentation, Printer), the tool parts of
`ShareX/TaskHelpers.cs`. Earlier review of the same area: `docs/audit/logic-2026-10-24/08-tools.md`
(T1-T13, N1-N4).

Status: done (2026-11-12).

## 1. Files read

modlight-tools (`app/crates/modlight-tools/`):

- [x] Cargo.toml, src/lib.rs
- [x] src/analyze.rs
- [x] src/background.rs
- [x] src/borderless.rs
- [x] src/color.rs
- [x] src/color_format.rs
- [x] src/color_picker.rs
- [x] src/combine.rs
- [x] src/dotnet.rs
- [x] src/history.rs
- [x] src/images.rs
- [x] src/indexer.rs
- [x] src/metadata.rs
- [x] src/ocr.rs
- [x] src/pin.rs
- [x] src/print.rs
- [x] src/qr.rs
- [x] src/settings.rs
- [x] src/video.rs

modlight-app (`app/crates/modlight-app/src/tools/`):

- [x] mod.rs
- [x] boxes.rs
- [x] files.rs
- [x] jobs.rs
- [x] kit.rs
- [x] qa.rs
- [x] pin.rs
- [x] ocr.rs
- [x] qr.rs
- [x] history.rs
- [x] history_list.rs
- [x] image_history.rs
- [x] image_viewer.rs
- [x] combiner.rs
- [x] color_picker.rs
- [x] video.rs
- [x] print.rs
- [x] metadata.rs
- [x] analyze.rs
- [x] background.rs

Also read for context (not in the area, not changed unless said): `controls.rs`
(`number_box_at`), `tools/screen_picker.rs` (`start_screen_pick`, `tick_screen_pick`,
`screen_pick_finished`), `tools/image_effects.rs` (`apply_guarded`, one line changed for
APPTOOLS-02), `uploads.rs` (`sync_upload_settings`), `modlight-core/src/tasks/context.rs`,
egui 0.36 `ViewportBuilder::patch`.

## 2. Findings

| Id | Severity | Where | What goes wrong | Fix | Test |
|---|---|---|---|---|---|
| APPTOOLS-01 | Medium | `modlight-tools/src/images.rs:17,30,48` (`ROTATE_BY_EXIF`, `load`, `decode`), `modlight-app/src/tools/jobs.rs:356` (`apply_app_options`) | Advanced settings, "Rotate image by EXIF orientation data" (`RotateImageByExifOrientationData`) did nothing: every tool load (`files::load_image`: pin from a file or the clipboard's files, OCR of a file, the image viewer, the history preview and thumbnails, the combiner, analyze image) turned a photo by its EXIF orientation even with the option off. Upstream's `ImageHelpers.LoadImage` turns it only while `HelpersOptions.RotateImageByExifOrientationData` (copied from the setting by `MainForm`) is on. | A process-wide switch in `images` (on by default, as upstream), `load_turned(path, rotate)`; the app copies the setting into it every frame (`jobs::tick` -> `apply_app_options`). | `images::tests::exif_orientation_follows_the_setting` (a hand-built JPEG with EXIF orientation 6), `tools::jobs::tests::the_exif_setting_reaches_the_tools` |
| APPTOOLS-02 | Medium | `modlight-app/src/tools/boxes.rs:456` (`caught`), `:471` (`install_panic_hook`); caller `tools/image_effects.rs:446` (`apply_guarded`) | The panic hook queued every panic off the main thread for the error window, also one the code caught on purpose. `apply_guarded` turns an image effect's panic into the effects window's error line, but it runs on tokio's blocking threads (the preview, the after-capture "Add image effects" preset), so the same panic also opened the "unhandled exception" error window with Continue / Quit. Upstream's `ThreadException` fires only for exceptions nobody caught. | `boxes::caught(f)`: `catch_unwind` with a thread-local depth the hook reads; inside it the hook only logs a warning. `apply_guarded` uses it (one line in `image_effects.rs`, app-effects' file, needed by this fix). | `tools::boxes::tests::only_panics_nobody_caught_reach_the_error_window` (fails with plain `catch_unwind`, as `apply_guarded` had it) |
| APPTOOLS-03 | Low | `modlight-app/src/tools/color_picker.rs:214` (`pick_from_screen`, the pipette) | The pipette minimised the colour picker and asked for a screen pick; `start_screen_pick` silently turns a request down while another pick is waiting or open (the screen colour picker hotkey, the editor's pipette, a second colour picker). Nothing then answered the window, and since it re-sends `Minimized(true)` every frame while `minimized` is set, it could never be brought back (clicking it in the Dock minimised it again). | The pipette does nothing while a pick is waiting, taking the desktop or open. | `tools::color_picker::tests::a_pick_that_cannot_start_leaves_the_window_up` |
| APPTOOLS-04 | Low | `modlight-app/src/tools/history.rs:1575` (`remembered_spec`), `:1597` (`keep_on_screen`), `history_list.rs` / `image_history.rs` (`spec`, `open_history`, `open_image_history`), `tools/mod.rs` (`WindowSpec.maximized`) | The history and image history windows (lead of 2026-10-24, N2): a window closed maximised opened normal (`IsMaximized` saved, never restored), and a place remembered on a display that is gone (unplugged, rearranged) was used as it was, so the window opened out of reach. Upstream's `ApplySavedWindowState` restores the maximised state and uses the place only when `Screens.ScreenFromPoint` finds a display. Also, the spec was rebuilt every frame from the place and size tracked every frame, so after each move or resize egui-winit sent `OuterPosition` / `InnerSize` with the values of the frame before (`ViewportBuilder::patch` acts on every change), working against a drag in progress. | `WindowSpec.maximized` (`with_maximized`); the windows keep the state remembered when they opened (`opened`) for their spec and track the live one only to save it; `keep_on_screen` drops a place no display shows, with the displays of the capture source (desktop points). | `tools::history::tests::a_remembered_window_opens_where_it_can_be_seen` |
| APPTOOLS-05 | Low | `modlight-tools/src/pin.rs:39` (`change_scale`), `:50` (`change_opacity`) | `ScaleStep` / `OpacityStep` come from the settings file unchecked (the options window keeps them to 1..100 / 1..90; a hand-edited or imported file need not). A huge step overflowed `scale + delta`: a debug build panicked on the UI thread at the first wheel step over a pinned picture; a release build wrapped, so a step up shrank the picture to 20 % (or faded it to 10 %). | Saturating addition before the clamp. | `pin::tests::a_huge_step_goes_to_the_limit` (overflow panic before) |
| APPTOOLS-06 | Low | `modlight-tools/src/history.rs:659` (`fetch_preview`) | The history window's preview of an uploaded picture claims a 32 MB ceiling, but checked it only against `Content-Length`, then read the whole body (`bytes()`) before looking at its size. An answer without a length (chunked) was read whole whatever its size, and one that never ended held the preview until the 30 s timeout with memory growing all along. | The body is read chunk by chunk and given up as soon as it passes the limit. | `history::tests::a_preview_without_a_length_stops_at_the_limit` (a chunked stub on 127.0.0.1; before the fix it ran into the 30 s timeout) |
| APPTOOLS-07 | Low | `modlight-app/src/tools/image_history.rs:480` (`refilter`) | Thumbnails decode on at most four workers (`WORKERS`, upstream's `ImageHistoryThumbnailLoader` semaphore). Filtering again (every search keystroke after 220 ms, a favourite under the favourites filter, the settings dialog) reset the count of running decodes to 0 while they still ran: four more started at once, and the old ones counting down as they landed let more start again. Typing in the search box over a folder of large photos decoded many full-size pictures at once. | The count keeps the decodes that still run; their pictures are dropped by the version check as before. | `tools::image_history::tests::filtering_again_keeps_counting_the_running_decodes` |
| APPTOOLS-08 | Low | `modlight-tools/src/history.rs:359` (`has_extension`), `:632` (`renamed`) | `Path.HasExtension("name.")` is false in .NET; Rust's `extension()` is `Some("")`. A history file named `shot.` renamed to `x` became `x.` (upstream: `x`), and the list window's "Rename file" and the file path / name / folder copy entries stayed on for it (upstream: off). The video converter had the same and was fixed on 2026-10-24 (T13). | An empty extension counts as none in both. | `history::tests::a_name_ending_in_a_dot_has_no_extension` |

## 3. Not fixed

| Id | Severity | Where | What | Why not, and the recommendation |
|---|---|---|---|---|
| APPTOOLS-N1 | Low | `modlight-app/src/tools/ocr.rs:886` (`ocr_image`, silent mode) | The OCR window keeps `ScaleFactor` to 1..4; silent mode takes it from the settings file as it is (only `max(1)`, as upstream's `OCRHelper.OCR`). A hand-edited 50 scales a 2560 x 1440 screenshot to 128000 x 72000: upstream's GDI+ throws and says so, the port's `imageops::resize` allocates tens of GB (swap, or the allocator aborts the app). | Same input handling as upstream; only reachable through a hand-edited or imported file. Recommendation: clamp to the window's 1..4 in `modlight_tools::ocr::scale`. |
| APPTOOLS-N2 | Low | `modlight-tools/src/combine.rs:10` (`layout`), `modlight-app/src/tools/combiner.rs:607` (`combine_images`) | The combiner window keeps `Space` and `WrapAfter` to 0..1000, but "combine" without the window (`combine_images`) reads them from the settings file as they are. A huge `Space` makes a union that does not fit memory (`RgbaImage::new` aborts the app; upstream's GDI+ throws), and `i64::MAX` overflows `x += w + space` (a debug build panics). | Hand-edited file only; upstream allows negative spaces from the file (overlap), so a clamp changes its semantics. Recommendation: refuse a union wider or taller than `u32` / the decoder's `MAX_SIDE`, keep negative spaces. |
| APPTOOLS-N3 | Info | `modlight-app/src/tools/metadata.rs:422`, `background.rs:456` | "{0} tags" and "No image selected" are English in both languages. | Upstream hard-codes them too (`MetadataWindow.axaml` `StringFormat={}{0} tags`, `TargetNullValue`); the port is 1:1. A translation would be a new resx key on both sides. |
| APPTOOLS-N4 | Low | `modlight-app/src/tools/ocr.rs:588` (`OcrWindow::closed`), `:903` (silent mode); `modlight-tools/src/ocr.rs:156` (`text_file_for`) | OCR of a picture file writes the text to the file with `.txt` as its extension, replacing whatever is there without asking: OCR of `notes.png` overwrites the person's own `notes.txt`. In the window, the text written is the window's last result, also when the person switched to another picture or a region with "Select image" / "Select region" (the text of picture B lands beside picture A). | Upstream does exactly this (`TaskHelpers.OCRImage`: `File.WriteAllText(Path.ChangeExtension(filePath, "txt"), result)`, also after `ReplaceSourceAsync`); the port is 1:1. Owner's decision. Recommendation: write only when the result is still the file's (the source was not replaced), and not over an existing `.txt` that was not written by OCR (or pick a free name). |
| 2026-10-24 N1, N3 | Low | `tools/jobs.rs:98` (`save_task_member`), `tools/print.rs:49` (`page_layout`) | Still as described in `logic-2026-10-24/08-tools.md` (edits land in the first overriding hotkey of a job; the paper changed in the print panel is not followed). | Unchanged; same reasons as then. N2 of that review is APPTOOLS-04 here; N4 (speed) unchanged. |

## 4. Outside my area

- `modlight-app/src/tools/screen_picker.rs:375` (`start_screen_pick`, app-capture): it refuses a
  request while `screen_picker` or `pending_pick` is set, but not while `screen_pick_busy` (the
  desktop is being taken off the UI thread). A second request in that moment is accepted;
  when the first capture lands it opens the picker for the first target, and when the second
  lands it replaces it, so the first target never hears back (a colour picker window stays
  minimised, an editor's pipette never answers). Recommendation: also refuse while
  `screen_pick_busy`, and return whether the pick started. APPTOOLS-03 guards the colour
  picker's side.
- `modlight-app/src/uploads.rs:164` (app-uploads): `TaskContext.strip_png_color_space` is
  created `false` and never set from `PNGStripColorSpaceInformation`; the settings switch is
  shown but not read. (`modlight-core/src/tasks/image.rs:50` notes the PNG encoder writes no
  colour-space chunks anyway, so the effect may be nil; worth a look by its owner.)

## 5. Checks

Run at the end, in `app/`, on the final diff (2026-11-12):

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings` (macOS): clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (with `wincc/env.sh`): clean (only the build script's note that the exe resources are
  embedded on Windows only).
- `cargo test --workspace`: 1000 passed, 0 failed, 4 ignored. Among them `modlight-tools`
  82 unit tests (5 new), `modlight-app` 149 unit tests (5 new), the snapshot suite
  (`tests/snapshots.rs`, every screen in every look, unchanged), `reference_server` 12 and
  `reference_app` 2 against the local reference servers, `tools_video` with Homebrew's
  ffmpeg.
- Seen failing against the old code (put back for the run, then the fix restored):
  `only_panics_nobody_caught_reach_the_error_window`, `a_pick_that_cannot_start_leaves_the_window_up`,
  `a_huge_step_goes_to_the_limit` (overflow panic), `a_preview_without_a_length_stops_at_the_limit`
  (30 s timeout). Not run against the old code because they test what the fix adds or the
  old line is plain: `exif_orientation_follows_the_setting` and `the_exif_setting_reaches_the_tools`
  (no way to turn EXIF off before; the JPEG fixture loads turned, as the old `load` always did),
  `a_remembered_window_opens_where_it_can_be_seen` (no maximised state before),
  `filtering_again_keeps_counting_the_running_decodes` (the old line set the count to 0),
  `a_name_ending_in_a_dot_has_no_extension` (`extension()` is `Some("")` for `shot.`).
- No snapshot pictures changed, no migrations.
