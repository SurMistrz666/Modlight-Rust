# HANDOFF: app-4 (phase 4 of the macOS Rust port: the image editor)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-4`, branch `app-4` (fast-forwarded
to main `a42c49f`, which carries phase 3). Never pushed. Rules: `app/docs/PORT.md`.
Upstream: `upstream/Modlight-App/ShareX.ImageEditor` (Avalonia + SkiaSharp).

The crate map, the effect registry contract and what a host provides are in
`app/docs/EDITOR.md`; per-feature state in `app/docs/STATUS.md` ("Image editor");
macOS substitutions in `app/docs/MACOS.md` ("Image editor").

## State

- [x] `modlight-editor` crate: model, `EditorCore` + history, image operations, text
  layout (fontdb + rustybuzz, bundled Inter), raster helpers.
- [x] Rendering of every annotation with tiny-skia, effect bitmaps, spotlight, emoji
  (Apple Color Emoji), shadows, the Background frame and `GetSnapshot` export.
- [x] Editor state machine (`editor/`): input and selection controllers, tool options,
  toolbar items and hotkeys, Background settings, options document, task result.
- [x] egui view (`ui/`): canvas (zoom levels, Cmd+wheel, pinch, middle-button pan,
  scroll, overlay chrome, text boxes), annotation toolbar with every picker and the
  shadow popup, bottom bar, sidebars (Background, effect dialog, options), effect
  browser, context and file menus, toast, the six dialogs.
- [x] Effects registry with the editor operations (crops, resizes, rotations, flips)
  and sample effects (brightness, grayscale, invert, Gaussian blur, flip); browser,
  schema-driven dialog, preview and apply.
- [x] App integration (`modlight-app/src/editor_window.rs`): Image editor hotkey and
  the selector (both choices open this editor), after-capture "Annotate image" (task
  mode), "Beautify image" (task mode, Background panel open; new
  `TaskQuestion::Beautify` in core), the notice's "Annotate image", Finder drops; copy,
  save, save as, upload, open, recent, insert, replace, background image, effect file,
  clipboard, paste, URL; the result back to the task or `RunImageTask`; options in
  `DefaultTaskSettings.ToolsSettings.ImageEditorOptions`.
- [x] Region capture annotations (`region_overlay.rs`): the editor as the overlay's
  workspace, `ANNOTATION_TOOLS_AVAILABLE = true`.
- [x] macOS services (`modlight-macos/src/editor.rs`): NSCursor pictures, the desktop
  picture through NSWorkspace, NSImage for HEIC; real app only.
- [x] Tests: 67 unit tests in the crate; `tests/ui_snapshots.rs` (31 scenes × 4 looks);
  `tests/interaction.rs` (scripted pointer and keys through the view, exported goldens);
  `tests/render.rs` (every annotation, exported golden); the app's snapshots
  `image-editor`, `image-editor-start`, `image-editor-annotate`,
  `region-capture-annotate` (and `region-capture*` updated for the tool strip); an app
  unit test that annotations go with the region picture.
- [x] Docs: `app/docs/EDITOR.md`, STATUS.md, MACOS.md.
- [ ] Optional Docker .NET golden harness: skipped (cost); the goldens are this port's
  own renders, checked by eye against upstream's code.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p modlight-editor --example gallery -- /tmp/gallery.png   # every annotation
EDITOR_SHOTS_DIR=/some/dir cargo test -p modlight-editor --test ui_snapshots
UPDATE_SNAPSHOTS=1 cargo test -p modlight-editor --test ui_snapshots   # accept changes
EDITOR_SCENE=editor-crop cargo test -p modlight-editor --test ui_snapshots
./target/debug/Modlight --qa-shot image-editor /tmp/editor.png      # the app's window
cargo run -p modlight-macos --example editor_probe                  # cursors, wallpaper
```

The editor's snapshot and render tests use this Mac's fonts (the font picker, Apple
Color Emoji); the interaction tests switch system fonts off.

## Decisions

- One annotation list (core and canvas are the same list). Highlights go below the
  first non-effect annotation, where upstream's canvas puts their visuals.
- Effect bitmaps use upstream's interaction-cache path (whole image processed once,
  then cut): a pixelate grid is aligned with the image.
- Inter Bold is not bundled: bold Inter text uses SemiBold; italics are simulated.
- `CombinedGeometry` unions (balloon and step tails) go through `i_overlay`.
- The canvas draws the image as a GPU texture (pre-scaled below 100 %) and rasterises
  annotations, spotlight and (with the Background panel open) the whole frame with
  tiny-skia for the visible part at the window's pixel density; the chrome is egui
  shapes in overlay units scaled by the zoom, as upstream's `OverlayCanvas`.
- Pasted and duplicated annotations get new ids (selection is by id; upstream kept the
  copied id).
- egui reports Cmd+V only with text on the clipboard: the main window's AppKit monitor
  hands a press to the focused editor window (`windows.editors.paste`).
- A press on the canvas commits an open text box first (upstream's LostFocus); the box
  a press opens keeps its focus through the release; the context menu opens on release.
- `ColorView` is approximated (three tabs, the Fluent palette with upstream's three
  replacements, the previewer with accent shades).
- The legacy (Greenshot) editor is not ported: the selector's choice is stored, and
  this editor opens either way.
- Print, pin to screen and the screen colour picker push the "later" notice (phase 5).

## Real-window checks for the owner

Nothing here was seen in a real window; QA renders and tests only. Please check:

1. Image editor hotkey (or Tools → Image editor): the selector the first time, then a
   maximised editor with the start screen; New image, Open, Load from clipboard,
   Load from URL, a recent file.
2. Draw each tool; select, move, resize, rotate; Cmd+Z / Cmd+Y; Backspace deletes;
   arrow keys nudge; Cmd+wheel and trackpad pinch zoom around the pointer; two-finger
   scroll and middle-button pan; the zoom picker and "Zoom to fit".
3. Text tool: click, type, Enter (Cmd+Enter breaks the line), Escape; edit an existing
   text by double click; the speech balloon's box.
4. Every picker opens below its button and closes on a click outside; the colour
   picker's tabs; the shadow popup opens on a right click of the shadow button.
5. Background panel (gradient, colour, image, wallpaper with your desktop picture),
   effect browser (right click toggles favourites), the grayscale dialog's live preview,
   Apply and Cancel.
6. Save (into the screenshots folder), Save as (save panel), Copy (paste into Preview),
   Upload; closing with unsaved changes asks; "Remember window state".
7. After-capture "Annotate image": Continue (the task goes on with the edited picture)
   and Cancel (the task stops); "Beautify image" opens with the Background panel.
8. Region capture with annotations allowed: the tool strip beside the region tool, draw
   on the desktop, back to the region tool, select an area, the picture carries the
   drawings; on two displays the strip follows the pointer.
9. Cursor annotation (the macOS cursor pictures), the emoji picker, drag a picture file
   from Finder onto the editor.
10. Cmd+V pastes a copied screenshot into the open editor (and not into My space).
