# Bug hunt 2026-11-11, area app-editor: the image editor

Scope: `app/crates/modlight-editor/**` and, in `modlight-app`, `editor_window.rs`,
`qa_editor.rs`, `tools/editor_hooks.rs`. Read against upstream `ShareX.ImageEditor`
(`Core/Editor`, `Presentation/Views/EditorView.axaml.cs`,
`Presentation/Rendering/AnnotationVisuals`, `Presentation/Emoji`, `Integration`) and
`ShareX.HelpersLib/Helpers/ImageHelpers.cs`. Earlier findings of
`docs/audit/logic-2026-10-24/06-editor.md` (E1 to E16) were checked and still hold;
R2 of that review is fixed here (EDIT-04).

Status: done. 12 findings fixed (1 high, 4 medium, 7 low), 7 left with a recommendation
(section 3), 2 notes for the effects library (section 4).

## 1. Files read

- [x] `app/docs/EDITOR.md`, the editor rows of `app/docs/MACOS.md` and `WINDOWS.md`
- [x] `docs/audit/logic-2026-10-24/06-editor.md`, the editor parts of `docs/audit/rust-app.md`
- [x] `modlight-editor/Cargo.toml`, `src/lib.rs`, `examples/gallery.rs`
- [x] `src/model.rs`, `src/geom.rs`, `src/color.rs`, `src/history.rs`, `src/core.rs`, `src/ops.rs`
- [x] `src/text.rs`, `src/render.rs`, `src/annot_effects.rs`, `src/emoji.rs`, `src/colr.rs`,
  `src/raster.rs`, `src/cursor.rs`, `src/frame.rs`, `src/options.rs`, `src/qa.rs`
- [x] `src/editor/mod.rs`, `input.rs`, `tools.rs`, `toolbar.rs`, `background.rs`, `effects_flow.rs`
- [x] `src/effects/mod.rs`, `src/effects/builtin.rs`
- [x] `src/ui/mod.rs`, `view.rs`, `canvas.rs`, `toolbars.rs`, `pickers.rs`, `sidebar.rs`,
  `dialogs.rs`, `effects.rs`, `menu.rs`, `style.rs`, `icons.rs`
- [x] `tests/common/mod.rs`, `tests/interaction.rs`, `tests/ui_snapshots.rs`, `tests/render.rs`,
  `tests/emoji.rs`
- [x] `modlight-app/src/editor_window.rs`, `qa_editor.rs`, `tools/editor_hooks.rs`

## 2. Findings

### EDIT-07 (high) Turning the image moved a blur off what it hid

- Where: `modlight-editor/src/core.rs` `rotate_annotations_orthogonal`,
  `rotate_annotations_arbitrary` (Rotate 90 clockwise / counter-clockwise, Rotate with a
  custom angle; 180 degrees and the flips were right).
- What went wrong: both moved the two corner points of every annotation through the
  image's rotation and then added the angle to the annotation's own rotation. For the
  kinds drawn as a box turned about its centre (rectangle, ellipse, text, balloon,
  picture, emoji, cursor, blur, pixelate, highlight, magnify) that turns them twice: at
  90 degrees the box swaps width and height and is then turned again, so it lies across
  the spot it covered. Scenario: pixelate a password in a 200 by 20 band of a phone
  screenshot, rotate the picture 90 degrees, upload: the pixelation is a horizontal band
  beside the now vertical password, which is in the clear in the upload. A frame drawn
  round a button frames the wrong spot, a text box wraps into a 1-word column, an inserted
  picture shrinks to a quarter. At 180 degrees boxes were right, but a balloon's tail
  (kept in the balloon's own frame) was mapped through the turn and then turned again, so
  it kept pointing the old way. Upstream's `EditorCore` has the same code.
- Fix: those kinds move as a whole (the centre through the image's mapping, the size
  kept, the angle added); points-based kinds (lines, arrows, freehand, steps, spotlight,
  smart eraser) still have their points mapped. Written up in `app/docs/MACOS.md`
  ("Behaviour that differs on purpose", image editor operations).
- Tests: `editor::effects_flow::tests::an_effect_stays_on_its_band_when_the_image_turns`
  (failed before: the exported highlight was not over its band after Rotate 90 and
  Rotate 90 by custom angle); `core::tests::rotation_moves_annotations_with_the_image`
  (now checks the drawn box, clockwise and counter-clockwise; it had asserted the old
  corner positions) and `a_default_balloon_tail_moves_once_with_the_image` (now checks the
  drawn tail tip, which stays where it was drawn before).

### EDIT-01 (medium) Cmd+V in an editor text field also pasted onto the canvas

- Where: `modlight-editor/src/ui/view.rs` `EditorView::paste_shortcut` (called by
  `modlight-app/src/editor_window.rs` `editor_viewports` for the AppKit / Windows key
  monitor's Cmd+V or Ctrl+V; both platforms install the monitor).
- What went wrong: the host's paste was turned into `Request::Paste` whenever no modal and
  no annotation text box was open, without asking whether one of the editor's own text
  fields had the keyboard (the effect browser's search box, an effect dialog's numeric or
  text box, the colour panel's hex box). Scenario: copy an annotation (Cmd+Shift+C), open
  the image effects, paste a word into the search box with Cmd+V: the word lands in the
  box and a copy of the annotation appears on the picture (with a picture on the
  clipboard, the insert dialog opens). Upstream's `OnKeyDown` returns early while a
  `TextBox` is focused; the view's own egui paste path already checked this, the host's
  path did not.
- Fix: the view remembers at the end of each frame whether egui wants the keyboard and
  `paste_shortcut` leaves the paste to the field then.
- Test: `tests/interaction.rs` `the_hosts_paste_leaves_a_focused_text_field_alone`
  (failed before: "the paste went to the canvas too").

### EDIT-02 (medium) A drop shadow's layer covered the whole annotation, not the view

- Where: `modlight-editor/src/render.rs` `draw_annotation` (the shadow layer).
- What went wrong: the canvas renders annotations into a target the size of the visible
  part of the view, but the shadow's layer took the annotation's whole extent on the
  device, clipped only on the left and the top. Zoomed in on a large shadowed annotation
  (a rectangle round a Retina screenshot, an arrow across a wide capture) the layer, its
  copy for the shadow and the blur's float planes took gigabytes on every redraw (a
  4100 by 2000 rectangle at 300 % on a Retina screen: about 1.8 GB per frame, seconds per
  frame while dragging), and past 16384 device pixels the shadow was dropped altogether
  (the fallback draws the annotation without it).
- Fix: the layer is clipped on every side to the target grown by the shadow's offset and
  blur reach (all that can reach a visible pixel); an annotation wholly outside the
  target draws nothing. Exports draw at 1:1 into a target of the picture's size and are
  unchanged.
- Test: `render::tests::a_shadow_shows_when_the_annotation_is_far_larger_than_the_target`
  (failed before: no shadow under a rectangle running past the target's right edge at
  4x). Every snapshot and the gallery golden still match.

### EDIT-03 (medium) Resize image with the aspect ratio kept could crash the app

- Where: `modlight-editor/src/editor/effects_flow.rs` `preview_effect_dialog`,
  `src/core.rs` `resize_image`, `src/raster.rs` `resize`.
- What went wrong: with "Maintain aspect ratio" on, the side not typed is worked out from
  the picture's ratio with no upper bound (`resize_target`, and the effect library's
  `resolve_target_size`). On a long narrow picture (a scrolling capture 10 px by 4000,
  or 100 by 30000) typing 10000 into Width asks for a height in the millions: the live
  preview (run on every keystroke) and Apply allocate tens of gigabytes, and in the
  resampler `width * height` overflowed `u32` (a panic in debug builds, a wrapped size
  and an out-of-bounds index in release). The app crashed.
- Fix: a resize the app could not open as a picture (`image_decode::fits`: 32768 px a
  side, 512 MiB) is neither previewed nor applied (the core refuses it and logs it; the
  preview keeps the last good one); the resampler sizes its buffer in `usize`.
- Test: `editor::effects_flow::tests::a_resize_far_past_the_picture_limits_is_refused`
  (panicked before: "attempt to multiply with overflow" in `raster.rs`); the same test
  checks that a resize within the limits keeps the ratio.

### EDIT-04 (low) One unreadable member reset every editor option

- Where: `modlight-editor/src/options.rs` `ImageEditorOptions::from_value` (read by
  `editor_window.rs` when a window opens and when one closes and writes back).
- What went wrong: serde reads the document all or nothing, so one member of an
  unexpected type (`"BackgroundImagePath": null`, `"CornerRadius": 4.5`, a hand edit, a
  NaN the port itself would write as `null`) returned the defaults for everything: tool
  colours and sizes, the customised toolbar, recent files, favourite effects. Closing the
  editor then wrote those defaults back over the stored options. Newtonsoft (upstream)
  converts such values or keeps the rest. Reported as R2 in the 2026-10-24 review and
  named as a lead for this hunt.
- Fix: when the whole document does not read, every member is tried on its own and only
  the ones that do not read fall back to their defaults; unknown members are still kept.
- Test: `options::tests::one_unreadable_member_keeps_the_others` (failed before: the
  thickness 9 came back as the default 4).

### EDIT-05 (low) Resizing an emoji kept hundreds of megabytes of stickers

- Where: `modlight-editor/src/emoji.rs` `render_cached` (the process-wide sticker and
  picker cache).
- What went wrong: an emoji annotation is drawn from a sticker rendered at its exact size
  (`render.rs`, the annotation's own bitmap is never set), so dragging an emoji's handle
  renders and caches a sticker for every size the drag passes through. The cache was
  cleared only after 512 entries, whatever they took: one drag from 160 to 800 pixels
  left about 400 MB behind, a drag to 2000 pixels gigabytes, for the rest of the session
  (the cache is global, every editor window adds to it). Upstream renders stepped
  "interactive" sizes while dragging and keeps at most 256 entries.
- Fix: the cache also keeps under 64 MiB of pictures (cleared when the next one would
  pass it); the entry limit stays. The emoji picker's cells (36 KB each) never reach it.
- Test: `emoji::tests::the_sticker_cache_keeps_under_its_memory_budget` (failed before:
  64 MB held at size 588 and growing).

### EDIT-06 (low) Saving to a `.jfif` or `.jpe` file wrote a PNG under that name

- Where: `modlight-app/src/editor_window.rs` `encode_for_path`.
- What went wrong: the editor's Save picks the format from the file's extension and knew
  only `jpg` and `jpeg` as JPEG; upstream's `ImageHelpers.GetImageFormat` (which the
  editor's save callback uses) and the app's own tools (`modlight-tools/src/images.rs`,
  `tools/files.rs`) also take `jpe` and `jfif`. Opening `photo.jfif` (the name browsers
  give saved JPEGs on Windows) through "Annotate image" and pressing Save rewrote it as a
  PNG with a `.jfif` name.
- Fix: `jpe` and `jfif` encode as JPEG.
- Test: `editor_window::tests::jfif_and_jpe_names_are_saved_as_jpeg` (failed before: PNG
  bytes for `photo.jfif`).

### EDIT-08 (low) A slow URL download replaced the work done meanwhile

- Where: `modlight-app/src/editor_window.rs` `Request::LoadFromUrl` (now
  `editor_url_loaded`).
- What went wrong: "Load image from URL" downloads in the background and, when it ends,
  closed whatever modal was up and loaded the picture. While it runs the start screen's
  other buttons work: a person can close the start screen or open a file and start
  drawing. When the slow download then ended, it replaced that picture, its annotations
  and its history without a question (no undo; `load_image` clears the history), or, if
  it failed, brought the start screen back over the work. Upstream's
  `OnLoadFromUrlRequested` has the same race.
- Fix: the result is used only while the start screen is still up and waiting for it
  (`url_loading`); otherwise it is dropped (logged). Written up in `app/docs/MACOS.md`.
- Test: `editor_window::tests::a_late_url_download_leaves_the_work_alone` (failed before:
  the 40 by 30 picture with its rectangle was replaced by the download).

### EDIT-09 (low) Editing a very large text zoomed in crashed the app

- Where: `modlight-editor/src/ui/canvas.rs` `Canvas::text_box`.
- What went wrong: the box over a text annotation or balloon being written is an egui
  text box at the annotation's font size times the zoom. egui (`epaint` texture atlas)
  asserts that every glyph fits its font atlas, which is as wide as the largest texture
  (at most 16384 on the GPU, 2048 headless): a text of a few thousand pixels, which comes
  from Resize image (font sizes scale with the picture: 288 pt times 15 when a small
  picture is enlarged) or from a hand-edited `TextFontSize`, edited at 400 % panics
  ("Tried to allocate a 3365 wide glyph in a 2048 wide texture atlas").
- Fix: the box's font is capped at a quarter of the atlas width in physical pixels; the
  annotation itself is drawn by the editor's renderer as before.
- Test: `tests/interaction.rs` `editing_a_huge_text_zoomed_in_does_not_crash` (panicked
  before in `epaint`).

### EDIT-10 (low, Windows) Chinese, Japanese, Korean and other scripts drew as boxes

- Where: `modlight-editor/src/text.rs` `FontSystem::resolve` (the fallback faces).
- What went wrong: a character that neither the annotation's family nor the bundled
  Inter (Latin and Latin Extended only) has is looked up in fallback families, and those
  were the Mac's (`Helvetica`, `Arial Unicode MS`), which Windows does not have. On
  Windows, text, balloon and font-picker text in Chinese, Japanese, Korean, Thai or
  Devanagari, and emoji typed into a text, drew the missing-glyph box. Upstream's
  Avalonia falls back to the system's fonts and draws them.
- Fix: on Windows the fallbacks are Segoe UI, Segoe UI Symbol, Microsoft YaHei, Yu
  Gothic, Malgun Gothic, Nirmala UI, Leelawadee UI, Ebrima and Segoe UI Emoji (families
  that are not installed are skipped); the Mac's list is unchanged.
- Test: `tests/emoji.rs` (Windows only, the binary that keeps system fonts on)
  `text_beyond_latin_finds_a_windows_font`; compiled here by the Windows clippy check,
  not run (no Windows machine): needs a run on Windows.

### EDIT-11 (low) Resizing the editor window by hand fought the pointer

- Where: `modlight-app/src/editor_window.rs` `builder` (the editor window's
  `ViewportBuilder`, rebuilt every frame).
- What went wrong: with "Remember window state" on (the default) `editor_viewports`
  stores the window's inner size into the options every frame, and the next frame's
  builder took its size from those options. egui's `ViewportBuilder::patch` (egui 0.36.2,
  `viewport.rs`; applied by eframe's `initialize_or_update_viewport`) turns a builder
  whose size changed into `ViewportCommand::InnerSize`, so while the window was being
  resized by hand every frame asked for the size of the frame before: the window snaps
  back towards the old size against the pointer. Mechanism confirmed in the egui and
  eframe sources; not watched on screen (no windows during the hunt).
- Fix: the builder uses the size and maximised state the window opened with (kept in the
  session); the options still follow the window and are written back on close.
- Test: `editor_window::tests::the_window_builder_keeps_the_size_it_opened_with`
  (failed before: the builder's size followed the stored 900 by 500).

### EDIT-12 (medium) An effect preview of an empty picture crashed the app

- Where: `modlight-editor/src/editor/effects_flow.rs` `preview_effect_dialog`.
- What went wrong: the live preview took whatever the effect returned as the picture to
  show. The effect library's crop (`crop_rect` in `modlight-effects`) returns a 0 by 0
  picture for an area outside the image, and the Crop image dialog's X and Y go up to
  100000: typing an X past the picture's width made the canvas upload a texture of
  nothing, which wgpu rejects ("Validation Error: Dimension X is zero") with a panic.
  Apply already refused empty results; the preview did not.
- Fix: an empty result is a failed preview (logged); the last good preview stays.
  `app/docs/EDITOR.md` says so in the effect contract.
- Test: `tests/interaction.rs` `an_empty_effect_preview_keeps_the_picture` (registers an
  effect returning 0 by 0, renders through wgpu; panicked before with the wgpu
  validation error). The interaction harness gained `harness_on(builder)` for it.

## 3. Not fixed

- **N1 (low) Smart eraser after Rotate 90.** Its two corners are mapped (the box lands
  right, it is drawn unturned) but, as upstream, its rotation is turned too (only its hit
  test reads it, so it is hit as a turned box), and its stretched edge (a column or a
  row sampled when it was drawn) keeps its old direction, so a horizontal stretch shows
  as stripes the wrong way. Recommendation: after a turn by 90 or 270, keep its rotation
  at 0 and sample the fill again from the turned picture (`configure_smart_eraser`).
- **N2 (low) Resize image scales only plain text.** `scale_annotations` multiplies the
  font size of text annotations but not of balloons or steps, so after halving a picture
  a balloon's text overflows its body. Upstream's `ScaleAnnotations` does the same; an
  owner's call whether to scale them too.
- **N3 (low) Emoji resized by hand renders every size.** Upstream renders stepped
  "interactive" sizes while dragging (`GetInteractiveStickerSize`) on a background task;
  the port renders the exact size on the UI thread every frame. EDIT-05 bounds the memory;
  the CPU per frame for a large emoji (tens of milliseconds) remains. Recommendation: use
  `emoji::interactive_sticker_size` while `pending_emoji_exact` is set.
- **N4 (low) Copy or save while an effect dialog previews** exports the document without
  the preview; upstream's `GetSnapshot` renders the visual, which shows it. Rare; left as
  is (the preview is not applied yet).
- **N5 (low) Saving a picture opened from `.webp` (or `.heic`)** writes PNG bytes under
  that name, as upstream's `GetImageFormat` (anything unknown is PNG). Recommendation: an
  owner's decision whether Save should go to Save As for formats it cannot write.
- **N6 (low, macOS) Emoji typed into a text annotation** draw Inter's missing-glyph box:
  Apple Color Emoji has bitmaps only, no outlines for the outline renderer. Upstream's
  `BuildGeometry` draws nothing for them. Recommendation: draw colour glyphs as
  images in `render::draw_outlined_text` (larger change).
- **N7 (low) Loading another picture with an effect dialog open** leaves a dialog whose
  Apply does nothing (the preview is cancelled, the dialog stays). Recommendation: close
  the effects panel in `Editor::load_image`.
- R1 of the 2026-10-24 review (Escape timing in a text box) is unchanged.

## 4. Outside my area

- `modlight-effects/src/fx/manipulations.rs` `resize_image` / `resolve_target_size`
  (area of the effects library): the same unbounded side as EDIT-03, and `scale_pixels`
  allocates whatever it is asked. The editor no longer previews or applies such a resize
  through the dialog (EDIT-03), but the function itself has no limit; anything else that
  calls it should check `image_decode::fits` first.
- `modlight-effects/src/fx/manipulations.rs` `crop_rect` returns a 0 by 0 picture for an
  area outside the image. The editor now treats that as a failed preview (EDIT-12);
  other callers that show or upload the result should do the same, or `crop_rect`
  could return an error.

## 5. Checks

Run at the end in `app/` (`CARGO_BUILD_JOBS=6`, `nice -n 10`):

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (`wincc/env.sh`): clean; this compiles the Windows-only `tests/emoji.rs` test of
  EDIT-10, which has not been run.
- `cargo test --workspace --no-fail-fast`: 1002 passed, 0 failed, 4 ignored. Of these,
  `modlight-editor`: 85 unit tests, 7 interaction tests, the 30-scene by 4-look
  snapshot test and the gallery golden; `modlight-app` library: 147. A first run without
  `--no-fail-fast` had 11 of `modlight-api`'s `reference_server` tests fail in 0.00 s
  while the local servers were not answering; on their own, and in the full rerun, they
  pass (environment, not code).
- Snapshot pictures: none changed; no `*.old.png`, `*.new.png` or `*.diff.png` left.
- No database migrations; no server or web code touched.
