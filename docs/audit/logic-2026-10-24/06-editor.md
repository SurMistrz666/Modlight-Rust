# Logic review 2026-10-24, area 06: the image editor

Scope: `app/crates/modlight-editor/src/**` except `effects/builtin.rs` (and the effect
registry, untouched), `modlight-app/src/{editor_window.rs, qa_editor.rs,
tools/editor_hooks.rs}`, `modlight-macos/src/editor.rs`. Read against upstream
`ShareX.ImageEditor` (`Core/Editor`, `Core/History`, `Core/Annotations`,
`Presentation/Controllers`, `ViewModels`, `Views`) and `ShareX/TaskHelpers.cs`
(`AnnotateImageModern`). Branch `logic-06-editor` from main `28916b4`.

Method: each suspected bug was reproduced by a test before the fix (unit tests in the
crate and in `modlight-app`, which drive the editor and its window host headlessly;
the app tests build an `AppState` with `qa::state`, nothing opens a window). Upstream
bugs that are fixed here are listed in `app/docs/MACOS.md`, "Behaviour that differs on
purpose" ("Image editor operations and annotations").

## Findings

| Id | Severity | Where | What was wrong | Reproduced by | Status |
|---|---|---|---|---|---|
| E1 | High | `editor/mod.rs:475` (`queue_auto_copy`), `editor_window.rs` `Request::AutoCopyImage` | "Auto copy image to clipboard" pushed the Copy command's request. The host then showed "Image copied to clipboard" after every change and, with "Close editor after task" on, closed the editor as soon as it opened or after the first edit (a capture's "Annotate image" step ended cancelled). Upstream only calls the host's copy handler. | `editor_window::tests::auto_copy_copies_without_the_copy_commands_notice_or_closing` (task result became Cancel) | Fixed `0a31551`; also `editor::input::tests::auto_copy_asks_for_one_plain_copy_per_change` (one copy per frame) |
| E2 | High | `editor/background.rs:302,322` | `BackgroundSettings::state` cloned the settings to build the brush, so the decoded picture was cached in the clone and dropped: an editor opened with a remembered Image or Wallpaper background decoded that file several times per frame (a HEIC wallpaper through NSImage and PNG), even with the Background panel closed. | `editor::background::tests::a_remembered_background_picture_is_read_once` (3 reads while hidden) | Fixed `b93ecca` (cache shared by clones, brush only while shown) |
| E3 | Medium | `core.rs:125` (`load_image`) | Loading a picture raised no image change, so the smart-padding border found on the previous picture stayed valid. With the Background panel open, opening another picture cropped it by the old border; a smaller one vanished from the frame and the export. | `editor::background::tests::a_new_picture_gets_its_own_smart_padding` | Fixed `b454e61` |
| E4 | Medium | `core.rs:352,520,597` | A speech balloon with its default tail (every balloon drawn at least 50 x 30 and never moved) had the tail taken after the body moved and moved again: after a crop, rotate, flip, resize, canvas size or cut-out the tail jumped by the offset a second time. Upstream's order is the same; its `EnsureTailPointInitialized` on selection hides it. | `core::tests::a_default_balloon_tail_moves_once_with_the_image` | Fixed `067e5f6` |
| E5 | Medium | `core.rs:352,520,597` | A line or arrow bent with its middle handle kept its curve point at the old place through crop, cut-out, rotate, flip, resize and canvas size, so its shape changed (upstream bug). | `core::tests::a_curved_line_keeps_its_shape_through_image_operations` | Fixed `067e5f6` |
| E6 | Medium | `core.rs:520` (`adjust_for_cut`) | A cut-out across an annotation moved both of its ends back by the strip width, so the part before the strip moved too (a rectangle 20..80 cut at 30..50 became 0..60 instead of 20..60); upstream bug. | `core::tests::a_cut_through_an_annotation_shortens_it` | Fixed `067e5f6` |
| E7 | Medium | `editor_window.rs` Save (`let written`), `editor_requests:269` | A failed save was only logged: no message, and the exit confirmation's "Yes" (save, then close) closed the editor over the unsaved picture. A picture that encoded to nothing was written as an empty file and reported saved. | `editor_window::tests::a_failed_save_says_so_and_keeps_the_editor_open` | Fixed `c0bee79` (error box as `ImageHelpers.SaveImage`; the Close of the same batch is skipped) |
| E8 | Medium | `editor/mod.rs:372`, `editor/input.rs:690,976,1070`, `editor/tools.rs:905` | Only undoable steps marked the picture dirty (as upstream), so after a save, moving, resizing, rotating, restyling or retyping an annotation and closing dropped those edits without the exit confirmation. | `editor::input::tests::edits_without_an_undo_step_ask_before_closing` | Fixed `06fa029` (selection sync does not mark it) |
| E9 | Medium | `editor_window.rs:445` and Pin/Upload arms, `:618` | Print, Pin to screen and Upload showed no notice (upstream: "Image printed", "Image pinned to screen", "Image is uploading"), and Print and Pin never closed the editor with "Close editor after task" on. | `editor_window::tests::print_and_pin_show_their_notice_and_close_when_asked` | Fixed `1ae8481` |
| E10 | Medium | `editor_window.rs:484`, `editor/input.rs` `remove_recent_image_file` | Opening a recent file that no longer exists kept it in the list and, outside the start screen, said nothing. Upstream removes it from the options and the start screen and shows "File no longer exists" on the start screen. | `editor_window::tests::a_recent_file_that_is_gone_leaves_the_list` | Fixed `1ae8481` |
| E11 | Medium | `editor_window.rs:645` (`close_editor`) | Each editor window wrote its whole copy of the options back on close, so with two windows open the one closed last undid the other's changes (tool colours and sizes, recent files, toolbar), and a window open while the settings changed undid those. Upstream's windows share one live object. | `editor_window::tests::two_editor_windows_keep_each_others_option_changes` | Fixed `6f1085b` (only the members the window changed are written) |
| E12 | Low | `core.rs:173,723` | The core skipped annotation and reorder mementos while the Crop tool was active (upstream, for the crop annotation its core drew). The port's crop never enters the document, so a picture, emoji, paste or duplicate added with the Crop tool active had no undo step: Undo removed it together with the step before. | `editor::input::tests::additions_with_the_crop_tool_active_can_be_undone` | Fixed `da74860` (core test `crop_tool_skips_annotation_mementos` replaced) |
| E13 | Low | `editor/mod.rs:1287` (`offset_clone`) | Duplicate (Cmd+D) and paste moved only the ends and freehand points; a bent line's curve point and a step's or balloon's tail stayed where the original's were (upstream bug). | `editor::input::tests::duplicates_and_pastes_keep_curves_and_tails` | Fixed `4ff1bd8` |
| E14 | Low | `editor/mod.rs:690` (`sync_core_selection`) | After an undo the core kept the selection stored in the memento while the view had cleared it; Home, End, Page Up, Page Down then reordered that invisible selection and dropped the redo step (upstream's keys do the same). | `editor::input::tests::z_order_keys_need_a_selection` | Fixed `4ff1bd8` |
| E15 | Low | `core.rs:597` (`crop`) | A crop area reaching past the image's left or top edge (Shift-square quick crop towards the corner) kept its full width and height from the edge and cropped pixels it never covered (upstream bug). | `core::tests::a_crop_area_over_the_edge_crops_what_it_covers` | Fixed `83e357f` |
| E16 | Low | `ui/style.rs:1195,1262` (`numeric_box`) | Numeric boxes parsed with Rust's `f64` parse, which reads "nan" and "inf"; a NaN passed the clamp into the effect dialog's values and the New image size. Upstream's decimal parse takes neither. | `ui::style::tests::numeric_boxes_take_numbers_only` (tests the parse the box now uses) | Fixed `9ac80c9` |
| R1 | Low | `ui/view.rs` keyboard, `ui/canvas.rs` text box | Escape in a text box commits it on the key press; if a frame is drawn before the release (Escape held past a cursor-blink repaint), the release reaches `key_up` and clears the selection of the committed text. Upstream keeps it selected. Timing dependent, cosmetic. | Reasoned from egui's focus timing; not reproducible headlessly (kittest delivers press and release in one frame) | Not fixed: cosmetic, needs key-press/release pairing state |
| R2 | Low | `options.rs` `from_value` | One member of `ImageEditorOptions` with an unexpected JSON type resets all editor options to defaults (serde is strict; Newtonsoft converts leniently). Documents written by upstream or by the port parse; NaN can no longer reach them (E16). | Reasoned | Not fixed: robustness, no reachable trigger found |
| R3 | Low | `editor/effects_flow.rs` | With an effect dialog open, Undo changes the document under the preview; Apply then replaces the undone state with the preview, as upstream does (its commit resets the core to the pre-effect picture). What you see is what is applied. | Reading upstream `CommitEffectAndCleanup` | Not changed: same as upstream |

## Shared files touched outside the area

None outside the listed files. Inside the area, `docs/EDITOR.md` (host table: `AutoCopyImage`,
the task-action notices, missing recent files, options write-back) and `docs/MACOS.md`
("Image editor operations and annotations" in "Behaviour that differs on purpose") were
updated. `Request` gained the variant `AutoCopyImage`; the region capture overlay drops
the editor's requests, so it needs no change.

## Checked and OK

- History: 50 mementos of which 5 with a picture, newest first, everything older than the
  first dropped goes too, redo cleared by a new memento, redo pushed without limits; undo
  and redo of add, delete (with step renumbering), clear, z-order, crop, cut-out, image
  operations, effects and flatten; selection restore filtered to existing ids.
- Step numbers: counter from `max + 1` or the start number after every history change,
  renumbering on delete, start number clamped to 1..10.
- Drawing: minimum size 5, Shift squares and 45-degree snapping, Cmd moving the shape
  while drawing, highlight insertion under the first non-effect annotation, smart eraser
  clamped and sampled, text box created on release and dropped when empty.
- Hit testing: rotated rectangles, ellipses, effects and text unrotated around their
  centre; lines and freehand with tolerance 5 plus half the pen; tails of steps and
  balloons; spotlight only under the Spotlight tool; handles 15 image pixels scaled by
  the zoom, as upstream's `OverlayCanvas` with its `ScaleTransform`.
- Resize and rotate through handles (rotated frames, emoji kept square, balloon tail
  scaled with the body), crop handles with the 16-pixel minimum, cut-out direction after
  15 pixels.
- Zoom: levels, Cmd+wheel and pinch around the pointer with the 150 ms throttle,
  zoom to fit (24-point margin, DPI, clamp, centring), zoom in/out rounding.
- Export (`snapshot`): 1:1 size rounded half away from zero, the frame only while the
  Background panel shows, clipping to the viewport, transparency kept; flatten uses the
  same picture as upstream's `GetSnapshot`.
- Text boxes: Enter and Shift+Enter finish, Cmd+Enter breaks the line, Escape commits;
  on macOS winit does not deliver keys the IME consumed, so confirming a composition does
  not finish the box; a click on a toolbar button surrenders the box's focus (and commits
  it) before the button acts.
- Options document: upstream member names, enums as names or numbers, unknown members
  kept; toolbar items in stored order with missing ones appended, an empty hotkey kept
  empty (cleared by the user) as upstream.
- Effects flow: preview on the original, Apply as one undo step, Cancel and closing
  restore, operations through the core with their notices.
- Host: Save/Save As paths and the auto-increment counter, paste priority (files, then a
  picture, then the copied annotation), URL downloads capped at 256 MiB and decoded under
  the limits, Finder drops, window close through the exit confirmation, task result
  handed back on close; screen colour picks set the same values as the pickers.
- macOS services: cursor pictures trimmed at 2x, desktop picture and its scaling, NSImage
  reads measured against the decode limits before drawing; none needs a permission.
