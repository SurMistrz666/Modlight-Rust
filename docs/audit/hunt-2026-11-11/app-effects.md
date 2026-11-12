# Bug hunt 2026-11-11: app-effects (image effects)

Area: `app/crates/modlight-effects/**` (the editor's effects `fx`, Skia arithmetic `sk`,
the classic `ShareX.ImageEffectsLib` port `classic`, helpers) and
`app/crates/modlight-app/src/tools/image_effects.rs` (the Image effects window).
Earlier review of this area: `docs/audit/logic-2026-10-24/07-effects.md` (E1-E14, N1-N3).

## 1. Files read

Read top to bottom:

- [x] `Cargo.toml`, `src/lib.rs`, `src/def.rs`, `src/helpers.rs`, `src/dotnet.rs`
- [x] `src/sk/mod.rs`, `color.rs`, `bitmap.rs`, `blend.rs`, `filters.rs`, `blur.rs`,
      `raster.rs`, `rect_aa.rs`, `text.rs`, `canvas.rs`
- [x] `src/classic/mod.rs`, `model.rs`, `apply.rs`, `gdi.rs`, `package.rs`, `sample.rs`
- [x] `src/fx/mod.rs`, `manipulations.rs`, `adjustments.rs`, `remove_background.rs`,
      `drawings.rs` (the effect bodies and helpers)
- [x] `modlight-app/src/tools/image_effects.rs`
- [x] `tests/edge_inputs.rs`, `tests/classic_edge.rs` (structure and value sets),
      `tests/golden.rs` (what it compares and skips)

Read in part, the rest swept for risk patterns:

- [~] `src/fx/filters_a.rs` .. `filters_o.rs` (about 11 000 lines): read `filters_a`
      bodies up to `block_glitch`, Matrix digital rain, Star filter, Starfield border,
      macOS and Windows 11 windows, Polaroid caption, Pixel sort, Motion blur, the
      blurred-paint call sites of every file; every file swept for `unwrap`/`expect`,
      integer `/` and `%` by parameters, `clamp` with variable bounds, string slicing,
      file and environment access, randomness and sorting. These effects are also covered
      by the golden comparison with upstream (677 cases, plus the 2026-10-24 edge
      goldens) and by the two dynamic checks below.

Dynamic checks made for this hunt (temporary harnesses, not kept):

- Every editor effect with 25 random mid-range value sets (0, middle, min+1, max-1 and
  random values of every slider and box, random options, checks, colours and texts) on
  1x1, 2x1, 1x2, 3x3, 2x7, 40x30 and 64x3 pictures in a debug build: no panic.
- Every editor effect timed with its defaults on a 1280x800 picture: found FX-02.
- Many-draw effects at their largest counts on 1920x1080 (Tile shuffle 20x20, Tile
  repeat 10x10, Draw particles 1000, classic Draw particles 1000): all under a second.

## 2. Findings

| Id | Severity | Where | What goes wrong | Fix | Test |
|---|---|---|---|---|---|
| FX-01 | Medium | `classic/apply.rs` `draw_particles` (the `let Some(r) = rect else` after the placement loop) | Draw particles with No overlap: when one particle found no free place in 1000 tries, the port returned the canvas at once. Upstream's `DrawParticles.DrawImage` only gives up on that particle and goes on with the next. With Background on, the early return also skipped drawing the picture over the particles: the screenshot was replaced by the particles alone (an after-capture preset wiped the capture). | `continue` with the next particle, as upstream. | `classic::apply::tests::a_particle_without_room_keeps_the_picture_over_the_background` (failed: the 8x8 picture came back as the blue particle) |
| FX-02 | High | `sk/canvas.rs` `draw_path_impl` (the layer for `mask_blur` / `image_blur`) | Every shape drawn with a blurred paint got a layer the size of the canvas (plus the blur margin), blurred and composited whole. Effects that draw thousands of small glowing shapes paid a full-canvas blur per shape: Matrix digital rain took 28 s on a 1280x800 picture with its defaults (84 s on 1600x1000, minutes on a 4K screenshot), Stained glass border 5.5 s, Circuit board border 4.4 s, Snowflakes 2.4 s, Starfield border 1.2 s. The editor runs the preview on the UI thread on every slider change, so the window froze. Skia blurs a mask the size of the shape. | When the layer's transparent pixels cannot change the canvas (no colour filter; SrcOver, Plus or Screen), the layer is only the shapes' device bounds plus the blur's reach (at least 3 sigma + 2, at least 9 px for the small-sigma kernel), inside the same canvas-plus-margin as before, and only that part is composited (`composite_rect`; the clip mask is no longer cloned per draw). Pixels are the same (goldens unchanged and passing). After: Matrix digital rain 0.1 s, the others below 0.1 s. | `edge_inputs.rs::many_small_blurred_shapes_do_not_blur_the_whole_canvas_each` (failed: 84 s; passes in 0.15 s) |
| FX-03 | Medium (Windows) | `tools/image_effects.rs` `PackagerWindow::create` | "Package preset" checks that the assets folder is inside the image effects folder with `starts_with("<folder>/")`. Upstream checks `_effectsFolder + Path.DirectorySeparatorChar`. On Windows the default assets folder (`effects_folder.join(name)`) and any folder picked with Browse are written with backslashes, so the check always failed: no package with assets could be made on Windows ("The assets folder must be inside..."). | `is_below(path, folder, MAIN_SEPARATOR)`: the system's separator, case ignored, as upstream. | `tools::image_effects::tests::the_assets_folder_check_follows_the_system_separator` (the old check is `is_below(.., '/')`, which refuses the Windows paths the test accepts) |
| FX-04 | Medium | `fx/drawings.rs` `replace_ci` (used by `expand_text_variables`, Text watermark) | The `%width`, `%y`, `%h`... tokens were searched in `text.to_lowercase()` and the offsets used to cut the original text. Lower case does not keep byte lengths ("İ" grows by a byte, "ẞ", the Kelvin and Ångström signs shrink): with both kinds in one text the cut fell inside a character and the editor's preview panicked (no `catch_unwind` there: the editor crashes), e.g. "İ%widthẞ". When the total length changed (any Turkish "İ"), it fell back to a case-sensitive replace, so "%WIDTH" stayed unreplaced although upstream replaces `OrdinalIgnoreCase`. | The ASCII tokens are matched byte by byte with `eq_ignore_ascii_case` in the text itself (a match starts and ends on ASCII bytes, always char boundaries). | `fx::drawings::tests::text_variables_expand_next_to_letters_that_change_length_in_lower_case` (panicked: byte index 9 is not a char boundary) |

## 3. Not fixed

| Id | Severity | Where | What | Why not fixed / recommendation |
|---|---|---|---|---|
| NF-01 | Low | `fx/manipulations.rs` `skew` (and any effect whose output grows with its parameters) | Skew with both angles at 89 (the sliders' ends) makes a canvas about 58 times the picture on each side: a 1920x1080 screenshot asks for about 7 gigapixels (28 GB) and the preview runs on the UI thread. Upstream computes the same size and lets `new SKBitmap` fail. | Same size math as upstream; the right guard (refuse outputs above a pixel budget with an error, like the classic `refused_by_gdi`) is a product decision for the editor as a whole. Recommend a shared limit in `Def::apply` (for example 2^28 pixels) returning `Err`. |
| NF-02 | Low | `fx/adjustments.rs` `lomo` (`w * w + h * h`), `classic/model.rs` `Padding::horizontal`, `classic/apply.rs` positions and offsets | i32 arithmetic on picture sizes past 46 340 px or on preset values near `i32::MAX` (only from hand-edited JSON) overflows: a panic in debug builds, wrapping in release, exactly as upstream's unchecked C#. | Release builds behave as upstream; widening would change upstream's results for those inputs. |
| NF-03 | Low | `classic/gdi.rs` `rotate`, `refused_by_gdi` | A preset with an angle beyond the float range (`"Angle": 1e39` in a hand-made package) gives a NaN size: the port returns a 1x1 picture (6x1 after a following Skew) where upstream's `new Bitmap` throws "Parameter is not valid." and the task stops. The options panel cannot enter such a value (it stops at a million). | Only reachable with a crafted file, and caught (`apply_guarded`); recommend refusing non-finite sizes in `refused_by_gdi` if packages from strangers become common. |
| NF-04 | Info | `sk/canvas.rs` `draw_image_clipped`, `fill_with_image` | Bitmap draws still allocate a canvas-sized coverage mask and layer per draw. Measured: 1000 particles on 1920x1080 take 0.2 s (the zeroed memory is mostly never touched), so not pathological. | Not a bug at the sizes used; the same bounding as FX-02 would apply if an effect ever draws many bitmaps with blur. |

## 4. Outside my area

- For merge: `tools/image_effects.rs` `AppState::image_settings_target` picks "the first
  hotkey with this job" whose image settings are its own, so with two hotkeys of the same
  job the second one's image effects presets are read from and written to the first one.
  Another branch adds the hotkey's serial to `TaskSettings::hotkey`
  (`HotkeySettings::serial`); the lookup should use it once that is merged. Not fixed here
  (reported by another area; the field is not on this branch).

## 5. Checks

Earlier fixes of `logic-2026-10-24/07-effects.md` (E1-E13) still hold: their tests
(`golden.rs::edge_pixels_match_upstream`, `edge_inputs.rs`, `classic_edge.rs`, the
`tools::image_effects` tests) pass unchanged.

Run at the end (app workspace, macOS host):

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (wincc environment): clean.
- `cargo test --workspace`: 45 test binaries, 995 passed, 0 failed, 4 ignored (the
  reference-server tests reached the local servers). `modlight-effects` alone: 38 unit,
  3 edge-input, 2 classic-edge, 3 golden tests (pixels and edge pixels against upstream
  renders, metadata) all pass after FX-02.

No snapshots changed, no migrations. New tests: 4 (one per finding).
