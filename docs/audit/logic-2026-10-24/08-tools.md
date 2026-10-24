# Logic review 2026-10-24, area 08: the ShareX tools and the history windows

Branch `logic-08-tools` (from main `28916b4`). Scope: `app/crates/modlight-tools/src/**`,
`app/crates/modlight-app/src/tools/**` without `image_effects.rs` and `editor_hooks.rs`,
`app/crates/modlight-macos/src/{ocr,print,pasteboard,sound,system,network}.rs`. Spec:
upstream `ShareX.Tools`, `ShareX.Avalonia/Windows` (screen colour picker),
`ShareX.HelpersLib/Presentation` and `Printer`, `ShareX.HistoryLib`, and the parts of
`ShareX/TaskHelpers.cs` that drive them.

Method: every file of the area read against its upstream counterpart; each suspected bug
reproduced by a test that failed before the fix and passes after it (the test is kept).
No window was opened, no permission asked, no network request sent (stubs and stand-in
scripts only).

## Findings

| Id | Severity | Where (at `28916b4`) | What is wrong | How it was reproduced | Status |
|---|---|---|---|---|---|
| T1 | High | `modlight-app/src/tools/history.rs:467` (`confirm`, `PromptAction::Rename`) | "Rename file" in both history windows used `std::fs::rename`, which replaces a file that already has the new name; upstream's `File.Move` refuses and shows "Rename file error". Renaming `a.png` to `b` destroyed `b.png`. Errors were also swallowed. | `tools::history::tests::renaming_never_replaces_another_file`: `b.png`'s content became `a.png`'s. | Fixed `9a2c16f` (a taken name is refused, the error is shown as upstream) |
| T2 | High | `modlight-app/src/tools/kit.rs:719` (`texture`) | The image viewer, a pinned picture and the image combiner's preview uploaded the whole picture as one texture. A side over the GPU limit (16384 px on Apple GPUs: a long scrolling capture, a stack of screenshots combined) fails to upload; wgpu treats that as fatal (egui asserts it in debug builds): viewing, pinning or combining such a picture ended the app. | `tools::kit::tests::a_texture_never_exceeds_the_largest_side`: egui's "Texture has size 300x6151, but the maximum texture side is 2048". | Fixed `18ce790` (a copy that fits is uploaded; `MACOS.md` line added) |
| T3 | High | `modlight-tools/src/color.rs:297` (`hex_to_color`) | The "0x" prefix check sliced the first two bytes of the text. The colour picker's hex box passes what is typed or pasted; a two-byte letter in second place ("aé", a Polish letter) cut inside the letter: panic on the UI thread. | `color::tests::hex_text_with_other_letters_is_no_colour`: "byte index 2 is not a char boundary". | Fixed `1622483` |
| T4 | Medium | `modlight-app/src/tools/mod.rs:243` (`viewports`) | All tool windows were taken out of `state.tools.windows` for the whole frame, so nothing a window did while drawing or closing could reach another window. Effects: the analyze options' Save never updated the open analyze window (a key just entered did not work until the window was reopened); the options dialog never made the analyze window wait (`ShowDialog`); the image effects window's "clear all" confirmation (Yes) cleared nothing. | `tools::tests::a_window_reaches_the_others_while_it_draws_and_closes`: a dialog could not find its parent. | Fixed `63b923c` (each window is out of the list only while it draws or hears it closed; the analyze window keeps its dialog's key from the call that opens it) |
| T5 | Medium | `modlight-app/src/tools/pin.rs:247` | Pinned picture wheel: `f32::signum(0.0)` is 1.0, so events without a vertical part (sideways swipe, Shift+wheel, the zero-delta events a trackpad sends when a gesture starts and ends) zoomed in (or raised opacity with Cmd). After a two-finger scroll down the picture grew back a step. Upstream: `Delta.Y == 0` does nothing. | `tools::pin::tests::only_the_vertical_wheel_scales_and_fades`: a (6, 0) wheel event scaled 100 % to 110 %. | Fixed `eafee8e` |
| T6 | Medium | `modlight-app/src/tools/history.rs:1020` (`key_act`), `pin.rs:283` | egui-winit turns Cmd+C and Cmd+Shift+C into `Event::Copy` and sends no key press of C. The history windows' Ctrl+C (copy URL) and Ctrl+Shift+C (copy file path) and the pinned picture's Ctrl+C (copy image) never fired in the real app; only headless tests, which send key events, saw them. | `tools::history::tests::the_copy_keys_arrive_as_copy_events`, `tools::pin::tests::cmd_c_copies_the_picture` (with the events egui-winit sends). | Fixed `a966b66` |
| T7 | Medium | `modlight-app/src/tools/history.rs:302` and the tag, rename and edit paths | After a favourite, tag, item edit or rename the port filtered again. In the image history that is `ResetLoadedEntries`: the grid jumped to the top, the selection and the loaded batches were dropped, every thumbnail was read again. Upstream refreshes the card in place (`RefreshVisibleItem`) and filters again only for a favourite under the favourites filter. | `tools::image_history::tests::a_favourite_keeps_the_place_in_the_grid`: 6 loaded cards fell back to the first batch of 2. | Fixed `fb87216` (`HistoryHost::edited`; the list window still filters again and keeps its selection) |
| T8 | Medium | `modlight-app/src/tools/video.rs:64` (`run_conversion`) | Stop sent `q`, a second `q` 100 ms later and killed ffmpeg 2 s after that. Upstream cancels once: one `q`, then ffmpeg finishes the file however long it takes (with `+faststart` it rewrites the whole file). Stopping a long conversion left an unreadable output. | `tools::video::tests::a_stop_lets_ffmpeg_finish_the_file`: a stand-in that needs 3 s after `q` was killed before it wrote its file. | Fixed `c390c50` |
| T9 | Medium | `modlight-app/src/tools/analyze.rs:1345` (`save_ai_keys`) | The OpenAI key is stored once, bound to its address; a task whose `OpenAICustomURL` is another address does not load it. Saving that task's analyze options without typing a key (another provider's key, a model) took the empty field for a cleared key and deleted the stored key every other task used. | `tools::analyze::tests::saving_options_keeps_a_key_entered_for_another_address`. | Fixed `ccd9e2f` (a key that was never loaded stays; a shown key that is cleared is still removed) |
| T10 | Low | `modlight-macos/src/network.rs:28` | `network_available` counted any up, running, non-loopback interface with an address. A Mac on no network keeps `utun*`, `awdl0` and `llw0` up with link-local `fe80::` addresses, so the answer was always yes: the offline queue never showed "no network" and kept attempting. Windows (and the port's Windows build) leave tunnels out. | `network::tests::addresses::link_local_addresses_are_no_network` (the interfaces this Mac's `ifconfig` shows up with only `fe80::` addresses, without Wi-Fi's own address). | Fixed `cd58f1d` (pure function over the interface list; `MACOS.md` row updated) |
| T11 | Low | `modlight-app/src/tools/region.rs:43` (`tool_region`) | When another capture held the screen, a tool's region request returned silently. The QR window stayed hidden with its scan buttons disabled for good, the pin startup window never came back, the OCR and analyze windows stayed minimised. | `tools::region::tests::a_region_that_cannot_start_still_answers`. | Fixed `59fbd7e` (the tool hears "no picture", as for a cancelled region) |
| T12 | Low | `modlight-tools/src/pin.rs:105,129,142` | Pinned picture size, keep-centre shift and placement rounded halves away from zero; upstream's `Math.Round` rounds them to even: a point off in those cases. | `pin::tests::halves_round_to_even_as_dotnet`. | Fixed `8c38914` |
| T13 | Low | `modlight-tools/src/video.rs:186` (`output_path`) | `Path.HasExtension("clip.")` is false (upstream makes it `clip.mp4`); Rust's `extension()` is `Some("")`, so the port kept `clip.` and ffmpeg failed to choose a format. | `video::tests::a_name_ending_in_a_dot_gets_the_extension`. | Fixed `c0fff7b` |
| N1 | Low | `modlight-app/src/tools/jobs.rs:105` (`save_task_member`, `OptionsHome::Hotkey`) | A tool's edited options are written to the first hotkey of that job that overrides the settings group. With two hotkeys of one tool that both override (an OCR hotkey per language, say), edits from the second land in the first. | By reading. | Not fixed: `TaskSettings` carries no hotkey identity (the description need not be unique); carrying one is a change in `modlight-core` and the hotkey dispatcher, outside this area. |
| N2 | Low | `modlight-app/src/tools/history.rs:1546` (`remembered_spec`) | The history windows restore size and place but not `IsMaximized`, and do not check that the remembered place is on a connected display (upstream `Screens.ScreenFromPoint`); a window remembered on an unplugged display may open off screen. | By reading. | Not fixed: needs a maximized field in `WindowSpec` and display geometry for the spec; low impact. |
| N3 | Low | `modlight-app/src/tools/print.rs:49` (`page_layout`) | The picture is placed for the paper the print settings name before the print panel opens; a paper size or orientation changed in the panel is not followed (upstream places it in `PrintPage`, after the dialog). | By reading. | Not fixed: needs an `NSView` subclass that lays the picture out at print time. |
| N4 | Info | `modlight-app/src/tools/history.rs` (`run`, `confirm`) | Multi-selection actions look each selected row up by id (`O(n·m)`); select-all and delete or copy on tens of thousands of rows takes seconds. | By reading. | Not fixed: speed, not a wrong result. |

## Shared files touched outside the area

- `app/docs/MACOS.md`: the network row (link-local addresses) and one line under
  "Behaviour that differs on purpose" (pictures larger than the GPU takes).
- No code outside the area. T4 changes how every tool window is hosted (`tools/mod.rs`,
  in this area), which also makes `image_effects.rs`'s `ConfirmWindow` ("clear all",
  Yes) reach its parent window; `image_effects.rs` itself is unchanged.

## Checked and OK

- Colours (`color.rs`, `color_picker.rs`, `color_format.rs`): HSB and CMYK conversions
  both ways against `ColorHelpers`, the picker boxes' half-to-even rounding, hex and decimal
  in RGB/RGBA/ARGB (negative ints), `ParseColor`'s two patterns and 20-character limit,
  known colour names (closest web colour, exact `KnownColor` for the screen picker),
  "copy all details" text and culture, recent colours (front, unique, 32), the control's
  box/slider/marker math and pixel bitmaps against `AdvancedColorPickerControl`, the
  `$`-tokens of `CodeMenuEntryPixelInfo` and `ScreenColorPickerTextFormatter` (order,
  case, `$r1{n}` precision, the formatter's own CMYK).
- Screen colour picker: pixel under the pointer per display scale, magnifier off the
  desktop, press/release/cancel and Cmd format, the copy and sound and toast.
- Image combiner: layout (spacing, wrap, centre with integer truncation, right/bottom,
  union size), source-over drawing and auto-fill background against
  `ImageHelpers.CombineImages`; the window's list, move, remove, options limits.
- Video converter: arguments for all 15 codecs (quality clamps, bitrate, `hvc1`, audio per
  codec, animation-only inputs), extensions, persistence, checks before starting, the
  error's last line, a real ffmpeg run (`tests/tools_video.rs`).
- Metadata: ExifTool parsing, grouping, copy text, absolute path first in the arguments,
  strip from the window and the hotkey, error texts.
- Directory indexer: text/HTML/XML/JSON against upstream (footers, sizes, counts,
  escaping, `DisplayPathLimited`), hidden entries, max depth, symlinked folders listed but
  not followed (no loops).
- QR code: UTF-8 byte limit, rendering with margin 1, SVG writer, luminance, rotation and
  inversion, several codes, joined results, file names, the window's scan/save/copy/upload.
- OCR: Vision language list and tag matching, scale clamp, single line, services (URL
  encoding of `{0}`, editor), silent mode (clipboard cleared on no text), text file beside
  the picture.
- Analyze image: requests and answers per provider, GPT-5.x effort rule, URL combining,
  model list sorting, connection summary, the A1 key binding.
- Background remover: model list and selection, sizes, device names, save over PNG/WebP,
  "save as" names.
- History: filters (wildcard over names and tags, type, host, URL, local date range,
  favourites), statistics (groups, order, `N0`), copy formats and menu states, tag text
  and parsing, edit, delete prompts, folder import (duplicates, order, kinds), viewer
  neighbours, preview loading.
- Print placement (auto rotate/scale/enlarge/centre, unscaled corner) against
  `PrintHelper.PrintImage`; settings window.
- Pin to screen: limits, inset, minimise size, toolbar rule, options ranges, keys.
- Image viewer: folder and list navigation, status line, closing rules.
- Input, output and error windows; the panic hook.
- macOS: `ocr.rs` (Vision request), `print.rs` (margins, flipped origin), `pasteboard.rs`
  (PNG before TIFF), `sound.rs` (sounds kept alive while playing), `system.rs` (http(s)
  only, `open --`).

## Verification

`cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`;
`cargo test -p modlight-tools -p modlight-macos`; `cargo test -p modlight-app` (lib, `snapshots`
in every look, `tools_video` with Homebrew's ffmpeg, `capture_upload`, `reference_app`):
all green on `logic-08-tools`.
