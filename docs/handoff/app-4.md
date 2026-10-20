# HANDOFF: app-4 (phase 4 of the macOS Rust port: the image editor)

Worktree `/path/to/modlight/_wt/app-4`, branch `app-4` (from main
`5e8e4bc`, which carries phase 3). Never pushed. Rules: `app/docs/PORT.md`.

Upstream: `upstream/Modlight-App/ShareX.ImageEditor` (Avalonia + SkiaSharp).

## State (kept current while working)

- [x] `modlight-editor` crate: model (`Core/Annotations`), `EditorCore` + history
  (`Core/Editor`, `Core/History`), image operations, text layout (fontdb + rustybuzz,
  Inter as Avalonia's default family), raster helpers (Skia-like resampling and blur).
- [x] Rendering of every annotation (tiny-skia), effect bitmaps, spotlight layer, emoji
  (Apple Color Emoji `sbix`), drop shadows, the preview frame and `GetSnapshot` export.
- [x] Editor state machine (`editor/`: input and selection controllers, tool options,
  toolbar items and hotkeys, Background settings, options document, task flow).
- [x] egui view (`ui/`): canvas (zoom, Cmd+wheel, pinch, middle-button pan, overlay
  chrome), annotation toolbar with every picker and the shadow popup, bottom bar,
  sidebars, effect browser, context and file menus, toast, the six dialogs.
- [x] Effects registry, browser panel, schema-driven dialog, preview flow.
- [x] QA scenes (`qa.rs`, 30 scenes) and snapshot tests (`tests/ui_snapshots.rs`,
  120 pictures: dark/light, EN/PL). Last commit with them: `edb464c`.
- [ ] App integration (selector, annotate after capture, notice button, hotkey).
- [ ] Scripted interaction tests with golden PNG exports.
- [ ] Docs: `app/docs/EDITOR.md`, STATUS.md, MACOS.md.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo test -p modlight-editor
cargo run -p modlight-editor --example gallery -- /tmp/gallery.png   # every annotation once
# the editor window, every QA scene in four looks (writes the PNGs to look at):
EDITOR_SHOTS_DIR=../_scratch/app-4/shots cargo test -p modlight-editor --test ui_snapshots
UPDATE_SNAPSHOTS=1 cargo test -p modlight-editor --test ui_snapshots   # accept changes
EDITOR_SCENE=editor-crop cargo test -p modlight-editor --test ui_snapshots  # one scene
```

The snapshot test renders with this Mac's fonts (font picker, Apple Color Emoji).

## Decisions so far

- One annotation list (core and canvas are the same list). Highlights are inserted
  below the first non-effect annotation, where upstream's canvas puts their visuals.
- Effect bitmaps use the interaction-cache path upstream's view uses (whole image
  processed once, then cut): a pixelate grid is aligned with the image.
- Inter Bold is not bundled: bold Inter text uses the SemiBold cut; italics are
  simulated with a 0.2 skew (Avalonia's oblique simulation).
- `CombinedGeometry` unions (balloon and step tails) go through `i_overlay`.
- The canvas draws the image as a GPU texture (pre-scaled with a triangle filter
  below 100 %, like Skia's mipmaps) and rasterises the annotations, the spotlight layer
  and, while the Background panel is open, the whole frame with tiny-skia for the
  visible part at the window's pixel density. The chrome (handles, outlines, crop) is
  egui shapes in overlay units scaled by the zoom, as upstream's `OverlayCanvas`.
- egui reports Cmd+V only when the clipboard holds text: the host sets
  `EditorView::host_watches_paste` and calls `paste_shortcut` from its AppKit key
  monitor (the main window already has one in `modlight_macos::keys`).
- Avalonia's `ColorView` is approximated: the three tabs (spectrum, palette with
  `FluentColorPalette` and the three replacements, components), the previewer with its
  accent shades, then "Pick colour from screen".
- Font names in the font picker are drawn in their own family with the editor's text
  engine (egui only knows its own fonts); the text box over a text annotation uses Inter
  at the annotation's size (SemiBold when bold).
