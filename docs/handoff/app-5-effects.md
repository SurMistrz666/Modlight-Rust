# HANDOFF: app-5-effects (phase 5 of the macOS Rust port: image effects)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-5-effects`, branch `app-5-effects`
(from the editor's `c7cadd6` plus main). Never pushed. Rules: `app/docs/PORT.md`.

Upstream: `ShareX.ImageEditor/Core/ImageEffects/**` (the editor's effect browser) and
`ShareX.ImageEffectsLib/**` (the classic preset effects of the task settings).

## State (kept current while working)

- [x] `modlight-effects` crate: Skia pieces (`sk`: colours, premultiplied storage, colour
  matrix/table/blend filters, matrix convolution, morphology, Skia's CPU blur and mask
  blur, blend modes, canvas over tiny-skia with highp image sampling), upstream helpers.
- [x] Golden harness (`app/scripts/effects-golden`, Docker only) and goldens in
  `crates/modlight-effects/tests/golden` (230 effects, 677 cases); test `tests/golden.rs`.
- [x] Adjustments (32 of 32): metadata and pixels match upstream.
- [x] Manipulations (32 of 32, Rotate3D dead code excluded).
- [ ] Filters: 117 of 149 ported (files `fx/filters_a.rs` .. `filters_k.rs`, through
  Rope border); next batches are the upstream sources from Rotate onwards (scratch
  `fb11..fb14.txt`).
- [ ] Drawings (16).
- [ ] `ShareX.ImageEffectsLib`: classic effects, presets JSON, packager, windows,
  "Add image effects" after capture.
- [ ] Snapshots of the windows; STATUS.md / MACOS.md.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo test -p modlight-effects                     # unit tests + metadata + goldens
# one effect, with the port's outputs and a report written out:
MODLIGHT_GOLDEN_ONLY=lomo MODLIGHT_GOLDEN_OUT=/tmp/fx cargo test -p modlight-effects --test golden
# regenerate the goldens (Docker, colima profile modlight):
scripts/effects-golden/run.sh            # all effects
```

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
  trait extension was needed so far. `modlight_effects::register()` adds them to the
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
  model inference belongs to the Background remover tool (`ShareX.Tools`), which the
  tools agent ports: `modlight_core::background_removal::remove_background(image,
  options)` exists as a stub returning "model not installed" for that agent to fill in.
