# The image editor (`modlight-editor`)

A port of upstream's `ShareX.ImageEditor` (Avalonia + SkiaSharp) to Rust: the document
and history are plain Rust, the annotations are rasterised with tiny-skia, the window is
egui. This page is the map of the crate, the contract an image effect implements, and
what a host (the app) has to provide. The per-feature state is in `STATUS.md`, the
macOS substitutions in `MACOS.md`.

## Map of the crate

| Module | Upstream | What it is |
|---|---|---|
| `model`, `geom`, `color` | `Core/Annotations` | Every annotation kind with its geometry, hit testing, bounds, curves, tails. |
| `core`, `history` | `Core/Editor/EditorCore`, `Core/History` | The document: source image, annotations, selection, z-order, canvas operations, mementos (50, of which 5 canvas). |
| `ops` | `Core/ImageEffects/Manipulations` used by the core | Crop, cut-out, resize, canvas size, rotate, flip, auto crop. |
| `text` | Avalonia's `FormattedText` | Font matching (fontdb, bundled Inter as `FontFamily.Default`), shaping (rustybuzz), wrapping and alignment, glyph outlines. |
| `render`, `annot_effects`, `emoji`, `cursor`, `raster` | the annotation controls, `SpotlightOverlayControl`, `EmojiPreviewImage`, Skia sampling | One function per annotation kind drawing into a `tiny_skia::Pixmap`; effect bitmaps (blur, pixelate, highlight, magnify); Apple Color Emoji stickers. |
| `frame` | `EditorView`'s `PreviewFrame` and `GetSnapshot` | The Background panel's frame (margin, padding, smart padding, rounded card, shadow, ratio) and the exported picture. |
| `editor` | `MainViewModel`, `EditorInputController`, `EditorSelectionController`, `EditorZoomController` | The editor without a window: tool options and their write-back, pointer and key input in image pixels, toolbar items and hotkeys, the Background settings, effects flow, notifications, task result. |
| `effects` | `Core/ImageEffects` (registry, parameters), `EffectBrowserPanel`, `SchemaDrivenEffectDialog` | The registry (below), the parameter schema and its values, the dialog state, recent and favourite lists. |
| `options` | `ImageEditorOptions` | The options document, read and written as upstream's JSON (PascalCase, enums as names, unknown members kept). |
| `ui` | `Presentation/Views`, `Presentation/Controls`, `Theming` | The egui window: canvas, toolbars, pickers, sidebars, effect browser, menus, toast, dialogs. |
| `qa` | | Fixtures for the snapshot tests and the app's `--qa-shot`. |

The editor never touches files, the clipboard or the network itself: it asks the host
through `Editor::take_requests` (next section but one).

## The effect registry contract

The image effects library (ported separately) plugs into `modlight_editor::effects`.
An effect is a value implementing `ImageEffect`; the library hands its effects over once
at start:

```rust
modlight_editor::effects::register_effects(vec![
    std::sync::Arc::new(GaussianBlur) as std::sync::Arc<dyn ImageEffect>,
    // ...
]);
```

`register_effects` adds to the process-wide registry; an effect whose id is already
there (the built-in samples included) replaces the earlier one. `registry()` returns a
snapshot the editor reads on every use, so effects registered later still show up.

### `ImageEffect`

| Method | Meaning | Upstream |
|---|---|---|
| `id(&self) -> &str` | Stable id, lower case with underscores (`"gaussian_blur"`). Stored in `ImageEditorOptions.RecentEffects` / `FavoriteEffects`, so it must not change; ids are compared case-insensitively. Old ids go through `resolve_alias`. | `ImageEffect.Id` |
| `name(&self) -> &str` | English name. The browser and the dialog title show `EffectBrowserPanel_Effect_<id>` from the string tables when it exists (`display_name`). | `Name` |
| `category(&self) -> Category` | `Manipulations`, `Adjustments`, `Filters` or `Drawings`: the browser's group, shown in this order. | `Category` |
| `icon(&self) -> char` | A Lucide glyph from `modlight_core::icons`. | `IconKey` |
| `description(&self) -> &str` | The browser's tooltip and search text (default empty). | `Description` |
| `parameters(&self) -> Vec<ParamDef>` | The schema the dialog is built from (default none). | `GetParameters` |
| `execution_mode(&self) -> ExecutionMode` | `Previewable` (default): a click opens the dialog in the right sidebar with a live preview. `Immediate`: a click applies it at once with default values. The browser labels previewable effects with "...". | `ExecutionMode` |
| `apply(&self, source, values, context) -> Result<RgbaImage, String>` | The work. | `Apply` |

`apply` must be pure: read `source` (straight, not premultiplied, RGBA8), read
`values`, return a new image of any size. It runs on the UI thread today (as upstream's
preview did on the dispatcher), once per change of a dialog value while previewing
(`context.is_preview == true`) and once more on Apply on the image as it was before the
preview started (`is_preview == false`). It must not panic; an `Err` is logged and the
image stays as it was. Expensive effects should keep previews fast (upstream's effects
do the same work in both modes; a cheaper preview is allowed when `is_preview` is set).

### Parameters (`ParamDef`, `ParamKind`)

Build them with the constructors, which carry upstream's defaults and formats:

| Constructor | Dialog control | Value read with |
|---|---|---|
| `ParamDef::int_slider(key, label, min, max, default)` | `EffectSlider` with a capsule, whole numbers | `values.int(key)` (rounds half to even, as `IntSlider`) |
| `ParamDef::float_slider(key, label, min, max, default)` | slider, `{0:0.##}` | `values.number(key)` |
| `ParamDef::checkbox(key, label, default)` | `LabeledToggleSwitch` | `values.boolean(key)` |
| `ParamDef::enumeration(key, label, options, default_index)` | `ComboBox`; options are `(label, value)` pairs | `values.choice(key)` (the value string) |
| `ParamDef::color(key, label, default)` | `ColorPickerDropdown` | `values.color(key)` |
| `ParamDef::int_numeric(key, label, min, max, default)` | `NumericUpDown` (empty allowed) | `values.int(key)` (rounds away from zero; empty gives the default) |
| `ParamDef::text(key, label, default)` | `TextBox` | `values.text(key)` |
| `ParamDef::file_path(key, label, default, filter)` | `TextBox` + Browse (the host's open panel) | `values.text(key)` |

`description` on a `ParamDef` is optional. Labels are shown as given (localise them in
the library). `values.sequence` records which numeric boxes changed last (the resize
dialogs use it to keep the aspect ratio from the box the person typed in).

### What the editor does with an effect

- **Browser**: Recent (last 10 used, newest first), Favourites, then the four
  categories sorted by name; search matches the label and the description. A right
  click removes a recent entry or toggles a favourite; both lists are written to the
  options.
- **Run**: `Editor::run_effect(id)` records it as recent, then: an id that maps to an
  editor operation (`operation_for`: crop, resize image or canvas, rotations, flips,
  auto crop) goes through `EditorCore` so annotations follow the image; an `Immediate`
  effect is applied as one undo step; otherwise the dialog opens with a preview.
- **Preview and apply**: the preview replaces the displayed image only; Apply puts the
  result into the document as one history step (`ApplyEffect`) and shows the
  "applied" notification; Cancel, Escape or closing the panel restores the original.

### Tests an effect library should keep

The editor's tests do not know the library's effects. The library should test each
`apply` against reference pictures; the editor side is covered by
`effects::tests`, `editor::effects_flow::tests` and the `editor-effect-browser` /
`editor-effect-dialog` snapshots.

## What a host provides

`Editor::take_requests()` after every frame. The app's handling (`modlight-app`,
`editor_window.rs`) is the reference:

| Request | Host does |
|---|---|
| `CopyImage` | `Editor::snapshot()` to the clipboard, then `show_task_action_notification`, `close_after_task_action`. |
| `AutoCopyImage` | `Editor::snapshot()` to the clipboard and nothing else ("Auto copy image to clipboard" after each change; at most one per frame). |
| `Save`, `SaveAs` | Write `snapshot()` to `image_file_path`, or a name from the task settings in the screenshots folder (Save as: a save panel first); then set `image_file_path`, clear `is_dirty`, `saved_notification`. |
| `Upload`, `Print`, `PinToScreen` | Upload, print or pin the snapshot, then `show_task_action_notification` (uploading, printed, pinned) and `close_after_task_action`. |
| `OpenImage`, `OpenRecent(path)`, `InsertImage`, `ReplaceImage(id)`, `BrowseBackgroundImage`, `BrowseEffectFile(key)` | Open panel (not for `OpenRecent`), then `load_bitmap` / `insert_external_image` / `replace_image` / `set_background_image` / `set_effect_value`. A recent file that is gone: `remove_recent_image_file` and `start_screen_status`. |
| `LoadFromClipboard`, `Paste` | A picture or picture file from the clipboard: load it (start screen) or insert it (paste); nothing outside: `paste_annotation` (paste) or `start_screen_status` (start screen). |
| `LoadFromUrl(url)` | Download in the background, then `load_bitmap` or `start_screen_status`. |
| `ClearClipboard` | Clear the system clipboard (the editor copied an annotation). |
| `PickScreenColor(target)` | The screen colour picker (a later phase). |
| `Close` | Close the window; `task_result` says what to hand back: `Continue` → `snapshot()`, `ContinueNoSave` → `source_image()`, otherwise nothing. |

Window close buttons call `Editor::request_close(true)` (the exit confirmation may
answer instead). Cmd+V: egui reports a paste only when the clipboard holds text, so the
host watches the key itself and calls `EditorView::paste_shortcut`
(`host_watches_paste = true`).

System hooks, all optional (without them the feature is simply absent, as upstream on a
platform without the service):

- `cursor::set_provider` — the picture of a `CursorType` (cursor annotations).
- `editor::background::set_wallpaper_service` — the desktop picture and its layout
  (the Wallpaper background mode exists only with a service).
- `editor::background::set_image_loader` — a reader for formats the `image` crate
  lacks (HEIC).

Options: `ImageEditorOptions::from_value` / `to_value` on
`DefaultTaskSettings.ToolsSettings.ImageEditorOptions`; the app writes back the members
a window changed when it closes (each window works on a copy; upstream's share one).

## Testing

```sh
cargo test -p modlight-editor                     # unit tests, interaction, snapshots
EDITOR_SHOTS_DIR=/some/dir cargo test -p modlight-editor --test ui_snapshots   # look at the renders
UPDATE_SNAPSHOTS=1 cargo test -p modlight-editor --test ui_snapshots          # accept changes
cargo run -p modlight-editor --example gallery -- /tmp/gallery.png             # every annotation
Modlight --qa-shot image-editor out.png           # the app's window, headless
```

- `tests/ui_snapshots.rs`: 30 scenes (`qa::SCENES`) × dark/light × EN/PL.
- `tests/interaction.rs`: pointer and key events through the egui view (draw, select,
  move, resize, undo, redo, delete, arrow, text box, crop, Cmd+wheel zoom, context
  menu), each ending in an exported picture compared with `tests/snapshots/export/`.
- The renders use this Mac's fonts (the font picker lists them, emoji come from Apple
  Color Emoji); the interaction tests switch system fonts off.
