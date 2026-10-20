# HANDOFF: app-5-tools (phase 5 of the macOS Rust port: the ShareX tools)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-5-tools`, branch `app-5-tools` (from
main `d665cb6`, main merged in at `e303cd9` (editor) and `21b3764` (security fixes)),
everything under `app/`. Never pushed. Rules: `app/docs/PORT.md`. Scratch:
`/Users/marcinasdfg/Desktop/modlight/_scratch/app-5-tools/` (QA shots in `out/`).

## Layout

- `crates/modlight-tools`: the tools without windows: colours and formats, settings types
  (`settings.rs`, Newtonsoft-shaped, merge-written), QR (rxing), image combiner, video
  converter arguments, ExifTool, directory indexer, OCR helpers, analyze image requests
  (`analyze.rs`), background remover models and saves (`background.rs`), borderless frames,
  history filters/stats/copy formats/import (`history.rs`), print placement (`print.rs`).
- `crates/modlight-core/src/background_removal.rs`: ONNX inference (ort), the
  `remove_background(image, options)` entry the effects branch stubbed; the real one
  replaces the stub when both land (same signature; `RemoveBackgroundOptions` now has
  `model` and `device`, `Default` = no model, which reports "not installed" as the stub).
- `crates/modlight-macos`: `ocr.rs` (Vision), `ax.rs` (Accessibility window move),
  `print.rs` (NSPrintOperation, PDF preview).
- `crates/modlight-app/src/tools/`: one file per window (`mod.rs` hosts them, `jobs.rs` is
  `TaskHelpers.ExecuteJob` for the tools, `kit.rs` the drawing pieces, `qa.rs` the QA
  screens, `history.rs` what both history windows share).
- Wiring in shared files (kept small): `lib.rs`, `state.rs` (field `tools`), `app.rs`
  (`tools::viewports`), `capture_flow.rs` (`execute_job` falls back to `run_tool_job`;
  `OverlayPurpose::Tool`), `region_overlay.rs`, `qa.rs` (screen list and hooks),
  `uploads.rs` (task effects: pin, OCR, analyze, QR scan, print; the notice's pin),
  `editor_window.rs` (print, pin, screen colour), `components_window.rs`
  (`AfterClose::Tool`), `app_pages.rs` (Image print settings), `task_pages.rs` (OCR
  language list), `recording_ui.rs` (output box). Core: `build.rs` (ShareX.Tools and
  ShareX.HistoryLib tables), `tasks/settings.rs` (`tools_settings`), `tasks/host.rs` +
  `worker.rs` (task effects), `upload_manager.rs` (folder indexer hook), `culture.rs`
  (`thread_utc` public), `lib.rs` (`background_removal`).

## State: every tool done

Each with QA screens and snapshots in 4 looks: colour picker, screen colour picker (also
the editor's), QR code, image viewer, pin to screen, image combiner, video converter,
metadata + strip, input/output boxes, error window, directory indexer (+ clipboard folder
indexing), OCR, analyze image, background remover, print, history, image history.
Active window borderless (Accessibility) and top-most (explains it is not possible).
Hotkeys, tray Tools menu and Tools tab all reach them (`jobs::run`). Left as "later":
`ImageEffects` (effects branch), `ToggleTrayMenu`.

Last full run (2026-10-20): `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace` all green (snapshots included).

## How it was verified

- Unit tests in `modlight-tools` (74): colour conversions and names, formats, QR round
  trips (generate then decode, rotations, inverted), indexer outputs (HTML/TXT/XML/JSON),
  video arguments, ExifTool parsing with a stand-in script, OCR language mapping, analyze
  image requests and answers per provider plus a round trip against a stub on 127.0.0.1,
  background model listing and saves, history filters, statistics, copy formats, import,
  print placement.
- `modlight-core` `background_removal`: pre/post-processing tests and a hand-written ONNX
  model (ReduceMean) run on the CPU and through Core ML ("Auto ran on: Core ML").
- `modlight-app/tests/tools_video.rs`: Homebrew's ffmpeg converts (x264, x265, GIF), a
  stop, an encoder ffmpeg lacks.
- `modlight-macos` `ocr::tests::vision_reads_a_line` (ignored; run once by hand with
  `MODLIGHT_OCR_SAMPLE`): Vision read the QR window's text and listed 33 languages.
- Every window: QA shots viewed (`--qa-shot`, light/dark, EN/PL) and snapshots.
- Nothing was opened on the owner's screen; no permission prompt was triggered.

## Real-window checks still needed (and the permission each needs)

| Check | Permission |
|---|---|
| Screen colour picker on real displays (Retina, two displays, the editor's pick) | Screen Recording |
| QR "scan screen" / "scan region", OCR region, pin from screen, analyze "Select region" | Screen Recording |
| Pinned pictures: transparency, always on top across Spaces, drag, Cmd+wheel opacity, toolbar | none |
| Image viewer full screen on the display with the pointer, keys | none |
| Drag and drop of files onto the combiner, video converter, analyze image, background remover | none |
| Video converter in a window with VideoToolbox encoders; ffmpeg from Components | none |
| Metadata and strip with ExifTool installed from Components | none |
| OCR with Vision on real captures in each language | none (Screen Recording for regions) |
| Analyze image with a real key per provider (network, costs money); keys saved to and read from the Keychain through `security` | Keychain (no prompt expected: items written by `/usr/bin/security`) |
| Background remover with the real 1 GB `bria-rmbg-2.0.onnx` from Components: Core ML compile time, memory, CPU fallback, save over PNG/WebP | none |
| Active window borderless: the system's Accessibility prompt, fill and restore on one and two displays, "exclude taskbar area" | Accessibility |
| Active window top-most: the message | none |
| Print: the print panel, printing without it, printer override by name, the PDF preview in Preview | none (printer access) |
| History windows on a real History.db: context menus, Finder reveal, upload/edit/pin/analyze from a row, remote previews, remembered size and place, import of a large folder | none |

## Differences from upstream that remain

See `app/docs/MACOS.md`, "ShareX tools (phase 5)". In short: Vision instead of Windows
OCR; Core ML instead of DirectML; borderless keeps the window frame and top-most is not
possible; the history windows have no drag-out, text date fields and thumbnails decoded
from files; the help/guide buttons that opened getsharex.com are not drawn; print preview
is a PDF in Preview; WebP saves of the background remover are lossless; analyze image keys
live only in the Keychain.

## Notes for whoever continues

- QA renders draw a window once: work that upstream does in the background is done
  synchronously when `state.env.qa || state.mode != Normal` (previews, thumbnails, OCR).
- `ort` needs network on a clean build (ONNX Runtime from cdn.pyke.io, SHA-256 pinned in
  `ort-sys`'s `dist.tsv`); afterwards it is cached in `~/Library/Caches/ort.pyke.io`.
- The tools' settings go back where they came from (`jobs::OptionsHome`: the defaults or
  the hotkey that overrides `ToolsSettings`/`CaptureSettings`); app-level ones
  (`HistorySettings`, `ImageHistorySettings`, `PrintSettings`) through `save_app_options`.
