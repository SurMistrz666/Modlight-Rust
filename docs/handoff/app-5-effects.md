# HANDOFF: app-5-effects (phase 5 of the macOS Rust port: image effects)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-5-effects`, branch `app-5-effects`
(from the editor's `c7cadd6` plus main). Never pushed. Rules: `app/docs/PORT.md`.

Upstream: `ShareX.ImageEditor/Core/ImageEffects/**` (the editor's effect browser) and
`ShareX.ImageEffectsLib/**` (the classic preset effects of the task settings).

## State (kept current while working)

- [x] `modlight-effects` crate: Skia pieces (`sk`: colours, premultiplied storage, colour
  matrix/table/blend filters, matrix convolution, morphology, Skia's CPU blur and mask
  blur, blend modes, canvas over tiny-skia with highp image sampling, hairlines,
  `rect_aa` for SkScan's rectangle anti-aliasing), upstream helpers.
- [x] Golden harness (`app/scripts/effects-golden`, Docker only) and goldens in
  `crates/modlight-effects/tests/golden` (230 effects, 677 cases); `tests/golden.rs`
  requires every effect (`REQUIRE_ALL`).
- [x] All 230 editor effects: Adjustments 32, Manipulations 33, Filters 149, Drawings 16.
- [x] `ShareX.ImageEffectsLib` (`modlight_effects::classic`): 51 classic classes
  (`model` metadata/JSON/converters, `gdi` operations, `apply`), presets JSON in
  upstream's Newtonsoft shape, `.sxie` packager (`package`), the sample picture.
- [x] App: `tools/image_effects.rs` (window in presets/tool/editor modes, options panel,
  gradient flyout, packager window, clear confirmation), Image effects hotkey / Tools
  menu / Tools tab card, task settings' "Image effects configuration...", the tray's
  preset submenu, `TaskQuestion::ImageEffects` (after-capture "Add image effects"),
  `.sxie` drop on the window; `modlight_effects::register()` at start.
- [x] Snapshots `image-effects*` (8 screens) and `image-editor-effects`,
  `image-editor-effect-dialog` (the editor with the real library), every look.
- [x] STATUS.md ("Image effects"), MACOS.md ("Image effects (phase 5)").

## Left for later

- `.sxie` file association and a command line (`-ImageEffect`): `AppState::import_image_effect`
  is ready; needs document types in `Info.plist` and an open-files handler (packaging).
- Real-window checks (no window was opened from automation): the Image effects window's
  popups (add-effect menu, colour and gradient flyouts), drag and drop of pictures and
  `.sxie` files, the tray submenu, Save/Upload in tool mode, the after-capture window
  (Continue and Cancel both apply the selected preset, as upstream).
- Classic effects have no goldens (System.Drawing is Windows-only in .NET 10); a
  Windows-side render of `contact_sheet`'s inputs would allow a comparison.
- The `mcr.microsoft.com/dotnet/sdk:10.0` image pulled for the goldens is still on the
  colima profile (no container left); remove it if the space is needed.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo test -p modlight-effects                     # unit tests + metadata + goldens
cargo test -p modlight-app --test snapshots        # QA_SCREEN=image-effects to narrow
# every classic effect with its defaults on one sheet:
CLASSIC_SHEET=/tmp/sheet.png cargo test -p modlight-effects contact_sheet -- --ignored
# one effect, with the port's outputs and a report written out:
MODLIGHT_GOLDEN_ONLY=lomo MODLIGHT_GOLDEN_OUT=/tmp/fx cargo test -p modlight-effects --test golden
# regenerate the goldens (Docker, colima profile modlight):
scripts/effects-golden/run.sh            # all effects
# the small pictures and parameter corners of edge.txt into tests/golden-edge:
EDGE=1 scripts/effects-golden/run.sh
# every effect at its parameters' ends on 1x1, one-row, one-column and odd pictures,
# and every classic class at the options panel's ends (no panic, sizes):
cargo test -p modlight-effects --test edge_inputs --test classic_edge
```

Logic review 2026-10-24 (`docs/audit/logic-2026-10-24/07-effects.md`): a one-off run of the
harness over every parameter's ends and six small pictures (7667 cases, not kept: 91 MB)
found the Skia differences fixed then; the cases that showed them are in `edge.txt`.

## Decisions so far

- Images from outside the process (map files of Displacement map etc., later presets
  and packages) are decoded with `modlight_core::image_decode` (from main's security
  fixes), never `image::open`; preset/package import rejects entries with paths.
- Skia raster details reproduced because upstream's pixels show them: anti-aliased
  strokes at most one device pixel wide are hairlines (`SkDrawTreatAsHairline`, rect
  frames excepted), walked segment by segment like `hair_path` (cap extension, closing
  segment between the pushed-out points), drawn through tiny-skia's hairline rasteriser
  with a padded copy near the top/left edge (tiny-skia clamps negative rows to 0); a
  bitmap drawn unscaled with paint alpha uses the legacy sprite blend (alpha+1, >>8).
- `Array.Sort(array, comparison)` is .NET's unstable introsort (`dotnet::sort_by`).

- Effects are `def::Def` values (id, name, category, icon, description, mode, parameter
  builder, run function) implementing the editor's `ImageEffect` trait unchanged; no
  trait extension was needed. `modlight_effects::register()` adds them to the
  editor's registry; ids equal upstream's, so they replace the editor's sample effects.
- Bitmaps keep straight colours but apply Skia's premultiply/unpremultiply rounding
  where upstream's Bgra8888 premultiplied bitmap would (`Bitmap::from_pixels`,
  `pixel_op` for `ApplyPixelOperation`, which sees premultiplied memory).
- `SKColors.Transparent` is `#00FFFFFF` (white, zero alpha): gradients to it pass through
  grey (Lomo's vignette). `C::ZERO` is empty memory.
- `MathF.Round` is half-to-even (`round_ties_even`) everywhere.
- Gamma: upstream's `SKColorFilter.CreateTable(null, t, t, t)` throws
  `ArgumentNullException` in SkiaSharp 3.119.4, so the effect fails on Windows. The port
  applies the table as intended (alpha untouched); the harness renders the golden the
  same way. Listed in MACOS.md as a deliberate difference.
- `Rotate3DImageEffect` and `Rotate3DBoxImageEffect` have no parameterless constructor,
  so upstream's browser never lists them (dead code): not ported.
- RemoveBackgroundImageEffect is a classic algorithm upstream (border colour prototypes,
  flood fill, centre protection, feathering), not the ONNX model: ported as such. The
  model inference belongs to the Background remover tool (`ShareX.Tools`): main's
  `modlight_core::background_removal` (the tools agent's ONNX implementation, taken
  as is in the merge) is not called by the effect.
- Classic library: presets stay where upstream keeps them
  (`ImageSettings.ImageEffectPresets` of the defaults or of a hotkey that overrides its
  image settings: `AppState::image_settings_target`); the window writes edits back at
  most once a second and when it closes, and the selected index on close (upstream's
  `selectedPresetChanged`). After capture the app answers `TaskQuestion::ImageEffects`
  with the preset applied off the UI thread; `None` (a crop outside the picture) ends the
  task like upstream's null.
- `GraphicsPath.AddArc` joins arcs to the figure (`gdi::figure_arc`); Skia's `addArc`
  starts a new contour (`sk::canvas::add_arc`). Rounded rectangles of the classic
  effects use the former.
- `TryGetSafeLocalPath` refuses symbolic links on the way except macOS's `/tmp`, `/var`,
  `/etc` (system links into `/private`).
