# HANDOFF: app-4 (phase 4 of the macOS Rust port: the image editor)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-4`, branch `app-4` (from main
`5e8e4bc`, which carries phase 3). Never pushed. Rules: `app/docs/PORT.md`.

Upstream: `upstream/Modlight-App/ShareX.ImageEditor` (Avalonia + SkiaSharp).

## State (kept current while working)

- [x] `modlight-editor` crate: model (`Core/Annotations`), `EditorCore` + history
  (`Core/Editor`, `Core/History`), image operations, text layout (fontdb + rustybuzz,
  Inter as Avalonia's default family), raster helpers (Skia-like resampling and blur).
- [x] Rendering of every annotation (tiny-skia), effect bitmaps, spotlight layer, emoji
  (Apple Color Emoji `sbix`), drop shadows, the preview frame and `GetSnapshot` export.
- [ ] Editor state machine (input and selection controllers), egui view, toolbars,
  pickers, dialogs.
- [ ] Effects registry, browser panel, schema-driven dialog, preview flow.
- [ ] App integration (selector, annotate after capture, notice button, hotkey).
- [ ] Tests: snapshots, scripted interaction tests with golden PNGs.
- [ ] Docs: `app/docs/EDITOR.md`, STATUS.md, MACOS.md.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo test -p modlight-editor
cargo run -p modlight-editor --example gallery -- /tmp/gallery.png   # every annotation once
```

## Decisions so far

- One annotation list (core and canvas are the same list). Highlights are inserted
  below the first non-effect annotation, where upstream's canvas puts their visuals.
- Effect bitmaps use the interaction-cache path upstream's view uses (whole image
  processed once, then cut): a pixelate grid is aligned with the image.
- Inter Bold is not bundled: bold Inter text uses the SemiBold cut; italics are
  simulated with a 0.2 skew (Avalonia's oblique simulation).
- `CombinedGeometry` unions (balloon and step tails) go through `i_overlay`.
