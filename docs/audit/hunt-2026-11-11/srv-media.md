# Bug hunt 2026-11-11: srv-media (media processing and serving, server)

Scope: `server/src/media/**`, `server/src/routes/{oembed,embed_card,avatar}.rs`.
Status: finished 2026-11-11. 3 findings fixed (MEDIA-01 and MEDIA-03 in the HEIF metadata stripper, MEDIA-02 in the delete refunds), 4 left open (section 3).

## 1. Files read

Legend: **R** read in full (code and tests); **C** code read in full, tests skimmed;
**F** code skimmed, then exercised with targeted probes (see "Probes" below).

- [x] R `media/serve.rs` (lookup, viewer relation, `consume_view`, view grant, `ViewBudget`, cache headers, `parse_range`, `stream_media`; compared with upstream `serve.ts`)
- [x] R `media/mime.rs`, `media/sniff.rs` (compared with upstream `mime.ts`, `sniff.ts`)
- [x] R `media/render_ticket.rs`
- [x] R `media/vips.rs`, `media/vips/ffi.rs`, `media/vips/operation.rs`, `media/vips/image.rs`, `media/vips/sharp.rs` (FFI ownership, GValue/area refcounts, cache-build failure path, timeout handler lifetime, tracked blobs)
- [x] R `media/sharp_runtime.rs`, `media/image.rs`, `media/probe.rs`, `media/avif_brand.rs`, `media/blurhash.rs`, `media/avatar.rs`
- [x] R `media/strip.rs`, `media/strip/jpeg.rs`, `png.rs`, `webp.rs`, `gif.rs`, `heif.rs`, `raster.rs`, `av.rs`, `pdf.rs`, `office.rs`, `fuzz.rs`
- [x] R `media/ffmpeg.rs`, `media/av_probe.rs`, `media/av_workspace.rs`, `media/av_action.rs`, `media/av_convert.rs`, `media/av_cost.rs`, `media/av_meta.rs`, `media/av_targets.rs`
- [x] R `media/transform_queue.rs`, `media/transform_cost.rs`, `media/abort.rs`, `media/derived_cache.rs`
- [x] R `media/motion.rs`, `media/motion_preview.rs`, `media/poster.rs`, `media/timeline.rs`, `media/timeline_preview.rs`, `media/frame_step.rs`
- [x] R `media/convert.rs`, `media/convert_in_place.rs`, `media/convert_plan.rs`, `media/convert_targets.rs`, `media/convert_verdict.rs`, `media/convert_notes.rs`
- [x] C `media/convert_size.rs`, `media/download_menu.rs`, `media/playback_health.rs`, `media/player_i18n.rs` (keys checked against `messages/{en,pl}/viewer.json`), `media/source_preview.rs`
- [x] R `media/embed.rs`, `media/card.rs`, `media/card_render.rs`, `media/card_art.rs`, `media/bots.rs`, `media/urls.rs`, `media/link_formats.rs`, `media/paste_type.rs`, `media/paths.rs`, `media/mod.rs`, `media/part_a.rs`, `media/actions.rs`
- [x] R `media/docx.rs`, `media/docx_support.rs`, `media/zip_guard.rs`, `media/pdf_lib.rs`
- [x] R `media/mammoth/mod.rs`, `mammoth/writers/html_writer.rs`, `mammoth/jszip.rs` (reader part), table spans in `mammoth/docx/body_reader.rs`
- [x] F the rest of `media/mammoth/**` (`document_to_html.rs`, `documents.rs`, `dingbat_to_unicode.rs`, `images.rs`, `js.rs`, `lop.rs`, `options_reader.rs`, `style_reader.rs`, `zipfile.rs`, `docx/*`, `html/*`, `styles/**`, `writers/mod.rs`, `xml/*`): a faithful file-by-file port with its own parity corpus (229 documents); slicing and arithmetic grepped, then fuzzed (below)
- [x] R `routes/oembed.rs`, `routes/embed_card.rs`, `routes/avatar.rs`
- [x] Callers read for context (not my area): the view-budget part of `routes/media_action.rs`, `upload/finalize.rs` (metadata columns), `vault/view.rs` (write pass-through), `vault/access.rs::open_files`

Probes (scratch, not kept): 50 000 random hostile `document.xml` bodies (markup fragments,
multibyte text, broken entities, CDATA, PIs, odd attributes, numbering and fields) through
`mammoth::convert_to_html`: no panic. The deepest nesting the reader accepts (632 nested
tables, 949 nested `w:sdt`, 1896 nested hyperlinks / `w:ins` / `w:smartTag` / `w:fldSimple`,
1897 `w:customXml`) through `docx_to_html` in a debug build: all render, no stack overflow
on the 256 MB conversion thread. A release build of the pre-fix `boxes_in` loop (MEDIA-01).

## 2. Findings

| Id | Severity | Where | What goes wrong | Fix | Test |
|---|---|---|---|---|---|
| MEDIA-01 | high | `media/strip/heif.rs:76` (`boxes_in`) | With "Remove metadata" on, a HEIC/AVIF upload whose `meta` (or `iinf`, `moov`) holds a box with a 64-bit size near `u64::MAX` made `at + length` wrap. Release builds (no overflow checks) took the wrapped sum as "fits", set `at` back to an earlier box and walked the same boxes for ever on a blocking thread, pushing headers into a `Vec` until the process ran out of memory (confirmed with a release build of the same loop: 50 million boxes and counting from a 24-byte input). The byte rewriters have no deadline, so any account that turns the switch on could take the server down with one upload. Debug builds panicked instead. | `checked_add` before the comparison, as `top_level` already did. | `strip::heif::tests::a_box_size_that_wraps_around_is_refused` (panicked "attempt to add with overflow" before the fix) |
| MEDIA-02 | medium | `media/actions.rs:120` (`delete_media_bulk`), `:147` (`delete_media_hard`) | The quota refund was computed from the rows the CALLER had read, not from the rows the `delete` removed. Two permanent deletes of the same files at once (a double-clicked "Empty trash", two `DELETE /api/uploads?mode=permanent` calls, a user's delete racing the retention purge) both refunded: each round of upload, trash and a doubled empty lowered `used_quota` by twice the bytes freed, so an account could store past its quota at will. Upstream has the same race (parity), but it is a quota bypass. | `delete ... returning "owner_id", "size"`; refunds are summed from the rows that statement removed (and use the row's current size). `deleted`/`bytes`/`failed` keep their upstream meaning. | `tests/media_actions_db.rs` `a_file_deleted_twice_is_refunded_once` (quota 400 instead of 700 before the fix) |
| MEDIA-03 | high | `media/strip/heif.rs` (`locations`, `item_kinds`, `meta_patches`, `top_level`) | The in-place HEIC/AVIF cleaner (runs on every upload of an account with "Remove metadata" on, on a blocking thread with no deadline) let the file decide how much memory and time it gets: (a) `iloc` fields may be 0 bytes wide, so an item of 8 bytes listed 65535 extents that cost no input: 8 bytes of `iloc` became 1 MiB of extents, a 16 MiB `meta` box terabytes; (b) items are compared with items (`kinds` x `locations`), with no cap on either: millions of empty items never finished; (c) metadata items may overlap each other until the very end, so thousands of XMP items over the same 16 MiB region each allocated a 16 MiB replacement before the overlap check; (d) every top-level box was remembered: a large upload of 8-byte boxes cost 4x its size in headers. Any account could exhaust the server's memory (OOM kill or abort) or pin a blocking thread with one upload. | Ceilings a real picture never reaches: 4096 items in `iinf` and in `iloc`, 16384 extents in all, 16 MiB of replacements in all (`held`), 4096 top-level boxes; past them the file is "damaged" and kept as it came (fail open, as before). | `strip::heif::tests::extents_that_cost_no_bytes_are_bounded`, `items_are_bounded_in_number`, `metadata_items_over_the_same_bytes_cannot_multiply_the_memory`, `a_file_of_endless_tiny_boxes_is_refused_without_remembering_them_all` (all four failed before the fix) |

## 3. Not fixed

| Id | Severity | Where | What | Recommendation |
|---|---|---|---|---|
| MEDIA-N1 | low | `media/strip/office.rs` (`Action::Copy`) | Entries copied "byte for byte" keep their DOS time and date and any extended-timestamp extra field (local header and directory record): when each part of the document was last saved stays in a "cleaned" file. Only the replaced parts get the zip epoch. The design (SKELETON 9.47) says entries are copied verbatim, so this is the owner's decision. | If the owner wants it, rewrite the time/date words of copied records to the zip epoch and drop the `0x5455`/`0x000a` extra fields (needs the local header rewritten too; CRCs are unaffected). |
| MEDIA-N2 | low | `media/strip/office.rs` (`plan_ooxml`) | Only the canonical part names are cleaned (`docProps/core.xml`, `app.xml`, `custom.xml`). OOXML finds the core properties through the `_rels/.rels` relationship and part names are case-insensitive, so a package that stores them elsewhere (rare; not Word or LibreOffice) keeps them. | Resolve the targets of the core/extended/custom-properties relationships from `_rels/.rels` instead of fixed names. |
| MEDIA-N3 | low | `media/av_meta.rs` (`whole`), `upload/finalize.rs` | A crafted MP4 whose `nb_frames` (or width/height) exceeds `i32::MAX` makes the insert's `$11::int` cast fail, so the upload fails with 500 instead of storing the file without that number. Upstream fails the same way (an `integer` column); it only hurts the uploader of such a file. | Treat values outside `i32` as unknown in `av_meta_from` (`whole` -> `<= i32::MAX`). Left alone: parity, and finalize belongs to srv-files. |
| MEDIA-N4 | info | `media/ffmpeg.rs` (`input_args`) | `-use_absolute_path 0` stops absolute MOV data references; how far a relative `alis` dref with `nlvl_from > 1` may climb above the uploader's storage directory depends on ffmpeg's own checks, which I could not verify here (crafting such a file needs tooling I did not have; Homebrew's ffmpeg 9 is not the image's 5.1). Upstream passes the same arguments. | Worth one empirical test in the production image: an MP4 whose `dref` points at `../<other tag>/<file>` converted with `/convert?to=mkv`. |

## 4. Outside my area

- `routes/media_action.rs` (srv-files) was read for the view-budget flow: every branch that
  returns content (`/raw` whole or ranged, thumbnails, moving tiles, the timeline sheet,
  `/download`, `/convert` through `admit`) consumes a view or needs the grant; a spent budget
  answers only the grant holder's exempt shapes. No bypass found. Observation (parity with
  Next, not a bug): `HEAD` of a capped file runs the GET handler and spends a view.
- MEDIA-02 changes what `media::actions::delete_media_bulk` / `delete_media_hard` refund (the
  rows they actually removed). Their callers (`uploads.rs` empty trash, `actions/uploads.rs`,
  `actions/admin_library.rs`, `routes/media_delete.rs`, `routes/api/uploads.rs`) need no
  change; their reported `deleted`/`bytes` keep the upstream meaning.

## 5. Checks run at the end

In `server/` (CARGO_BUILD_JOBS=6, nice):

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test` with `MODLIGHT_TEST_DATABASE_URL` (dev Postgres on 55421, database tests
  included): 1431 passed, 0 failed, 4 ignored (the upstream parity harnesses that need the
  reference container), across 39 test binaries.
- Each new test was run against the code before its fix and failed there (MEDIA-01:
  "attempt to add with overflow"; MEDIA-02: quota 400 instead of 700; MEDIA-03: all four
  returned `Ok`).

No migration, no snapshot, no web panel change.
