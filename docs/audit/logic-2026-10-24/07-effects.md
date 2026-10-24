# Logic review 2026-10-24, area 07: image effects

Branch `logic-07-effects` (from main `28916b4`). Area: `modlight-effects` (`fx`, `classic`,
`sk`, helpers), the editor's built-in sample effects, and the Image effects window
(`modlight-app/src/tools/image_effects.rs`).

## Method

- **Every editor effect on edge inputs** (`tests/edge_inputs.rs`, kept): all 230 effects
  with their defaults, each parameter at both ends (every enum option, toggled checks,
  transparent/white/black colours, empty and long texts, missing files), all at their
  minimum and all at their maximum, on 1x1 (opaque and transparent), 1x9, 9x1, 2x2,
  7x5 (opaque and transparent) and 33x17 pictures, in a debug build (overflow checks on).
  No panic, no empty output; sizes kept by every effect that keeps them upstream.
- **Upstream on the same corners**: the golden harness, extended for this review, ran
  upstream's own classes in Docker (colima `modlight`) on every parameter's ends and six
  small pictures: 7667 cases (91 MB, not kept). `pixels_match_upstream` against them gave
  97 cases over the thresholds; the real causes are fixed below, the cases that showed
  them are kept in `tests/golden-edge` (`scripts/effects-golden/edge.txt`, `EDGE=1`).
- **Metadata**: ids, names, categories, icons, parameter kinds, ranges, defaults, formats
  and enum options are already compared field by field with upstream's manifest
  (`metadata_matches_upstream`, unchanged, passes).
- **Classic library** (no goldens possible: GDI+ is Windows-only): every class with its
  properties at the options panel's ends (`tests/classic_edge.rs`, kept), the JSON
  converters against Newtonsoft's and the TypeConverters' rules, the setters' limits,
  `ImageEffectPreset`/`ImageEffectPackager`, and the GDI+ size limits.
- **Window**: the view model's commands against `ImageEffectsViewModel`, the options panel
  against `ImageEffectOptionsPanel`, the gradient flyout against `GradientOptionsPanel`
  (one kittest interaction test), drops, packager and task flow.

## Findings

| Id | Severity | Where | What was wrong | How reproduced | Status |
|---|---|---|---|---|---|
| E1 | Medium | `sk/canvas.rs:434` `draw_rect`; `fx/filters_b.rs:325` `border` | A stroked empty rect (zero height or width) was framed (`AntiFrameRect`), which fills the whole frame; Skia's `easy_rect_join` refuses an empty rect and strokes it as a path. Border with type Inside on a picture no taller than the stroke painted the whole picture. An inside border wider than half the picture inverts its rect; `rect()` collapsed it, which only worked by the accident above. | Upstream render `border-inside-5.p7x5`: 85.7 % of pixels differ (port all black). | Fixed `960f0be`: empty rects go to the path stroker, `border` passes the corners to `draw_rect_ltrb` (sorted, as `DrawRect`). Tests: `golden.rs::edge_pixels_match_upstream` (`border-inside-*` in `tests/golden-edge`). |
| E2 | Medium | `sk/canvas.rs:615` (`draw_path_impl`) | Strokes under a pixel wide were always drawn as hairlines with their width as coverage. Skia does so only when that coverage is one or the blend mode can carry coverage as alpha (`SkBlendMode_SupportsCoverageAsAlpha`); with Screen, Multiply etc. it strokes them at their width. Glare's rays (Screen) came out at near full strength on small pictures or with a small radius. | `glare.default.p7x5` 31 % of pixels differ, `glare.min-radius.photo` 2.95 %; 22 extended cases. | Fixed `960f0be`. Tests: `glare.*` in `tests/golden-edge`. |
| E3 | Low | `sk/canvas.rs:673` (`draw_path_impl` layer) | A mask or image blur was computed on a canvas-sized layer, so a shape reaching past the canvas lost its glow along the edge; Skia renders the mask over the clip bounds outset by the blur's margin (at most 128 px). Neon glow border at its smallest border or widest tube. | `neon_glow_border.max-tube_width.photo` 23 % differ, `min-border_size.p7x5` 28 %. | Fixed `960f0be` (padded layer). Tests: `neon_glow_border.*` in `tests/golden-edge`. |
| E4 | Low | `sk/canvas.rs:508` `draw_round_rect` | A round rect without both radii was stroked as a path (a 1 px stroke becomes a hairline); `SkCanvas::drawRoundRect`/`onDrawRRect` draws it as a rect (framed). macOS and Windows 11 window with corner radius 0: wrong border coverage on small pictures. | `macos_window.allmin.p1x1` 92 % differ (195 vs 202); 12 extended cases. | Fixed `960f0be`. Tests: `macos_window.allmin.p1x1`, `windows11_window.allmin.p7x5` in `tests/golden-edge`. |
| E5 | Medium | `fx/filters_h.rs:785` `minecraft_grass` | With an overlap larger than the border and the picture together the canvas height is negative; `Canvas::new` clamps it to one row, so applying the effect replaced the picture with a 1-pixel strip. Upstream's `new SKBitmap` fails and the editor keeps the picture. | `edge_inputs` (temporary assertion in `Canvas::new`: only this effect asks for a non-positive size); upstream harness: "Unable to allocate pixels". | Fixed `b70305a` (returns the error). Tests: `edge_inputs.rs::minecraft_grass_fails_when_the_overlap_eats_the_picture`, `minecraft_grass.max-overlap.p7x5` (upstream error case) in `tests/golden-edge`. |
| E6 | Low | `fx/filters_k.rs:530` `reflection` | A reflection under one row (1 % of 64 rows, 20 % of a 1-row picture) gave the picture plus the offset's empty rows; upstream throws (empty `SKBitmap`), so the editor keeps the picture. | Upstream harness: 12 error cases; port drew them. | Fixed `59d0c61`. Tests: `reflection.default.p1x1`, `reflection.min-percentage.photo` in `tests/golden-edge`. |
| E7 | High | `classic/model.rs:791` `FontSpec::to_converter_string`, `:1688` `fmt_float` | A preset's font went into the settings and `.sxie` packages with the interface language's decimal separator: with the app in Polish the default watermark font was saved as "Arial, 11,25pt" and read back, here and on Windows, as an 11 pt font. Newtonsoft writes TypeConverter strings invariant. The options panel's font box used the same string and a "," list separator, so editing it cut the size at the decimal comma. `fmt_float` (summaries) followed the language instead of the region. | `fonts_are_written_invariant_in_every_language_and_region` failed ("Arial, 11,25pt, style=Bold"); `summaries_follow_the_region_not_the_language` failed ("1.5" for pl-PL). | Fixed `3479918`: JSON invariant; the panel shows and reads the font as `ConvertToString`/`ConvertFromString` with `CurrentCulture` (pl-PL "Arial; 11,25pt"); summaries use `modlight_core::culture`. Tests: the two above and `fonts_in_the_panel_follow_the_region`. |
| E8 | Medium | `tools/image_effects.rs:187` `select_preset` | Switching presets kept the first effect selected by index, and the text boxes' buffers and the validation line were cleared only when that index changed: between two presets starting with the same class the box showed the previous preset's text, and typing in it wrote that text into the selected preset. Upstream rebuilds the editors (`SetEffect`) on every preset change. | `switching_presets_rebuilds_the_text_editors` failed. | Fixed `2073169`. |
| E9 | Medium | `tools/image_effects.rs:1837` `gradient_panel_inner`, `:1392` | Each stop's location box was keyed by the stop's index and the stops were sorted after every keystroke: typing a location past another stop's sent the next keystrokes to the other stop (0/20/100, typing 501 into the first left it at 50 and set the second to 100). Upstream's `ReorderStops` keeps each row bound to its stop. | kittest `a_stop_location_typed_past_another_stays_with_its_stop` failed. | Fixed `313b39f`: rows drawn in location order, stops keep their indices while the flyout is open and are sorted when it closes (`flyout.Closed += gradient.Sort()`). |
| E10 | Medium | `classic/apply.rs:24` `try_apply`, `:57` `refused_by_gdi`; `tools/image_effects.rs:445` | The options panel accepts numbers up to a million and a package any `int`. Canvas margins, Resize, Scale, Skew, Shadow, Glow, an outside border, the checkerboard or pixelate tile, DrawImage or particle sizes of that order reserved tens of gigabytes (calloc, lazily) and overflowed the i32 pixel indexes: a panic in release builds, caught by the window but shown by the app's panic hook as an unhandled error for the preview thread; just under the overflow, a real multi-gigabyte allocation. Upstream's `new Bitmap` throws "Parameter is not valid." and the preview shows it. | Release probe (not kept): Canvas margin 100000 on 4x4: "index out of bounds: the len is 40001600016" at `sk/canvas.rs:1195`; Resize, Shadow, Skew alike. | Fixed `fb98c2b`: before each effect the largest bitmap it will make is checked against GDI+'s 2 GiB and the effect fails with GDI+'s message (the window shows it, a task ends with no picture as before). Test: `classic_edge.rs::bitmaps_larger_than_gdi_allows_are_refused`. |
| E11 | Low | `classic/gdi.rs:940` `rotate` | Rotate with Upsize ("larger than the input and no clipping") drew into a bitmap of the input's size, cutting the corners: an upstream bug (`RotateImage` uses `bmp.CreateEmptyBitmap()`), ported as is. | `rotate_with_upsize_grows_instead_of_clipping` failed (40x10 turned 90 degrees stayed 40x10). | Fixed `c1449f5`; listed in `MACOS.md` "Behaviour that differs on purpose". |
| E12 | Low | `classic/model.rs:707` `GdiColor::to_converter_string` | `Color.Empty` (read from "") was written back as "Empty", no colour name for Windows' `ColorConverter`; reading failed there and kept the default. | `the_empty_colour_round_trips_as_an_empty_string` failed. | Fixed `a09dc16`. |
| E13 | Low (tooling) | `scripts/effects-golden/run.sh` | The default upstream path (`$APP/../..`) pointed outside the repository from both the main checkout and a worktree, and the work folder in `$TMPDIR` is not shared with the colima VM (the container saw an empty folder). | Running the harness from this worktree. | Fixed `960f0be` (upstream from the main checkout via `git --git-common-dir`, work folder under `target/`); `EDGE=1` mode added. |
| E14 | Medium | `tools/image_effects.rs` `ConfirmWindow` | "Clear effects": Yes in the confirmation did nothing, because the parent window was out of the window list while the confirmation drew. | Reported by area 08. | Fixed by area 08 (`63b923c` on `logic-08-tools`, `tools/mod.rs`); not changed here. |

### Not fixed

| Id | Severity | Where | What | Why not fixed |
|---|---|---|---|---|
| N1 | Low | `classic/apply.rs` `Preset::try_apply` | The picture goes through Skia's premultiplied storage (`store`) before the classic effects, while GDI+ keeps 32bppArgb straight: semi-transparent pixels of a picture opened in the tool window lose colour precision before pixel-exact effects (Replace colour with threshold 0). | Captures are opaque, where it changes nothing; without Windows renders a change of the classic pipeline's storage cannot be checked against upstream. |
| N2 | Low | `classic/model.rs` `GdiColor::parse`, `value_from_json` | Hand-written JSON corners differ from .NET: "#RGB" is read with alpha 255 (.NET: 0), ints from JSON floats round half away from zero (.NET: to even), "255, 255, 255" stays numeric where .NET's `PossibleKnownColor` turns it into "White". | Windows never writes these forms; both sides read what the other writes. |
| N3 | Low | `sk/*` | 60 of the 7667 extended cases stay over the thresholds: anti-aliasing of many small or complex shapes on 1 to 35 pixel pictures (Color halftone, Dot matrix, Embroidery, Engraving, Pointillism, Ornate scroll, Chain link, Circuit board, Draw shape corners), one-pixel trig rounding (Twirl), self-intersecting extreme Wave edge, the many overlapping mask-blurred panels of Stained glass at its smallest panels (Skia truncates the A8 blit, the port rounds: up to 16 levels), and two random-seeded effects on 1-pixel pictures (statistics meaningless). | Rasteriser precision, not logic: the documented class of difference (`MACOS.md`, "Image effects"). |

## Checked and OK

- No editor effect panics on any of the edge inputs and value sets above (debug build,
  overflow checks on); every output is at least 1x1; every effect outside Manipulations
  keeps its size unless it is one of the 34 that upstream also resizes, and every size in
  the 7667 upstream cases matches.
- Parameter schemas against upstream (`metadata_matches_upstream`); the editor's built-in
  sample effects are all replaced by the library's at start (every id exists upstream).
- Classic: every class with its properties at the panel's ends, every enum member, empty
  flags, empty and one-stop gradients, tiny and huge fonts, odd texts: no panic, no empty
  picture. Setter limits (`constrain`) and defaults against each class's `DefaultValue`s
  and setters; Glow's and DrawTextEx's random default gradient (`RandomFast.Next(0, 2)`
  is inclusive: three choices); MatrixColor's row layout; Crop returning null outside the
  picture; AutoCrop's padding only after a crop; Rotate's angle 0 shortcut;
  ReplaceColor, SelectiveColor (Aggregate tie rule), ColorDepth, PerceivedBrightness
  against `ImageHelpers`/`ColorHelpers`.
- Presets JSON: `$type` long and short forms, unknown classes skipped, `Enabled` and
  `Name` (omitted when empty), enums by name or number (case-insensitive), flags as
  "Top, Bottom, Left, Right" / "None", Point/Size/Padding/Color strings, gradients
  (`Type`, `Colors`, clamped `Location`), BOM, missing list giving the default preset,
  empty list kept empty.
- `.sxie` packager: `Config.json` without BOM, entries named from the assets folder's
  parent, pictures only, the first root `Config.json`, 100 MB ceiling, refused escaping
  paths and planted links (existing tests).
- Window: new/remove/duplicate/move presets and effects, add after the selection, remove
  selecting the next, Delete and F5 keys (not while typing), preview versioning and busy
  state, null result keeping the previous preview, import with picture paths kept only
  inside the effects folder, drops (pictures in tool mode, `.sxie` anywhere), packager
  checks (assets inside the effects folder, path, overwrite), number editors' ranges and
  the byte overflow message, write-back at most once a second and on close, selected
  preset saved to the task's image settings reference, task flow (editor window first
  when asked; Continue and Cancel both apply, as upstream; random preset over the whole
  list; invalid index leaves the picture), tray preset menu.
- Snapshots `image-effects*`, `image-editor-effects`, `image-editor-effect-dialog`
  unchanged.

## Files outside the area

None in code. `app/docs/MACOS.md` (the GDI+ row and one "differs on purpose" line),
`docs/handoff/app-5-effects.md` (how to run the new tests and the edge goldens),
`app/scripts/effects-golden/` (harness of this area).
