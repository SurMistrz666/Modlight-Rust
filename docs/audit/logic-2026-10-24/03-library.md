# Logic review 2026-10-24, area 03: My space (the library), media previews, library dialogs

Branch `logic-03-library` (from `main` 28916b4). Files reviewed:
`app/crates/modlight-api/src/library.rs`, `app/crates/modlight-core/src/library/*`,
`app/crates/modlight-core/src/folders.rs`, `app/crates/modlight-app/src/myspace/*`,
`media.rs`, `media_windows.rs`, `qa_media.rs`, `dialogs/{duplicates, folder_delete,
link_settings, prompt, confirm, message}.rs`, read against upstream
`ShareX/Modlight/{ModlightLibrary, ModlightDuplicates, ModlightFormat, ModlightDownloadMenu,
ModlightConvertText}.cs` and `ShareX/Presentation/MySpace/**`, plus the server's routes where
the client's behaviour depends on them (`app/api/folders`, `lib/uploads.ts`,
`lib/media/urls.ts`).

Every finding below was reproduced by a test written before the fix (the test failed on the
unfixed code, passed after); the test stays in the tree. Line numbers are those of the base
commit 28916b4. Where upstream has the same fault, the fix is recorded in
`app/docs/MACOS.md`, "Behaviour that differs on purpose".

## Findings

| Id | Severity | Where | What is wrong | Reproduced by | Status |
|---|---|---|---|---|---|
| L03-01 | Medium | `modlight-app/src/media.rs:379,404` (`tick_motion`, `begin_motion`) | After the dwell, `tick_motion` called `begin_motion` on every frame until a reel played. An answer that is not a preview (503 from the server's transform limiter, a timeout, a dropped connection) started the next request at once: a tight request loop against a busy server for as long as the pointer rested on the card. Upstream makes one `BeginAsync` per dwell ("a retry loop against a server that just said it is busy is the wrong answer"). | Stub server answering 503: 117 requests in 1.5 s of one hover (expected 1). | Fixed `767eb8a`; `media::tests::a_busy_server_is_asked_once_per_hover`, `a_moving_preview_plays_after_the_dwell` |
| L03-02 | Medium | `modlight-app/src/media.rs:404` | Cached motion bytes that do not animate were decoded, refused, and decoded again on the next frame: `begin_motion` never looked at the refusal it had just filed. CPU burn while hovering. | Undecodable bytes in the cache: 77 decodes in 1 s (expected 1). | Fixed `767eb8a`; `media::tests::bytes_that_do_not_animate_are_decoded_once` |
| L03-03 | Medium | `modlight-app/src/myspace/model.rs:484` (`remove`), `actions.rs:649,476` | Trashing, restoring, deleting or moving away the last files of a page left an empty grid (empty-folder panel) under a pager that had already clamped itself to the new last page, so that page's button was a no-op (`Pager::go_to` = current) and only Refresh got out. Same upstream. | Page 3 of 30-per-page with 61 files, the one file trashed: no new listing; also page 1 emptied, a folder page emptied by a move. | Fixed `dc013a8` (page refilled from the server); `myspace::actions::tests::emptying_a_page_lists_the_page_that_now_holds_the_files` |
| L03-04 | Medium | `modlight-api/src/library.rs:334` (`download_to_file`), `modlight-app/src/myspace/actions.rs:1156` | A failed download destroyed the file the person had agreed to replace in the save panel: the destination was truncated before the body arrived (a cut connection left a partial file under the real name), and the caller deleted the destination after ANY failure, including a refusal (404, 401, failed conversion) that came before a byte was written. Same upstream (`File.Delete(path)` in `SaveAsync`). | Stub server: a 10-of-100-byte body left "0123456789" in place of the old file; a 404 deleted the old file. | Fixed `4a5b6cd` (part file beside the destination, renamed when whole) and `d7b2ec1` (caller no longer deletes); `library::tests::a_download_that_does_not_finish_leaves_the_old_file_alone`, `myspace::actions::tests::a_refused_download_keeps_the_file_it_would_have_replaced` |
| L03-05 | Low | `modlight-app/src/myspace/actions.rs:1108` (`download`) | "Download as" asked for the destination (save panel) and announced "download started" BEFORE the conversion dry run; upstream runs the dry run first so a refusal never reaches a save dialog. | Stub answering 415 + `X-Convert-Refused`: "Download started" toast and the QA Downloads folder created before the refusal. | Fixed `d7b2ec1`; `a_refused_conversion_never_asks_where_to_save`, `an_accepted_conversion_is_saved` |
| L03-06 | Low | `modlight-core/src/library/files.rs:365` (`base_of`) | Sliced the server's link as text at `len - len(".ext")`; a link that does not end in its extension and ends in multi-byte characters panicked (char boundary) while building the card. | `base_of("https://h/T/ąąę", "jpeg")` panicked. | Fixed `059238d`; `library::files::tests::a_link_cut_inside_a_character_is_not_a_panic` |
| L03-07 | Low | `modlight-core/src/library/files.rs:572` (`stem`) | "Download as" names the file `Path.GetFileNameWithoutExtension(name)` and falls back to the code when that is empty; `stem()` kept ".bashrc" (saved as hidden ".bashrc.png") and did not drop folder parts. | `stem(".bashrc")` was ".bashrc". | Fixed `6c14af7`; `library::files::tests::the_stem_is_what_dotnet_calls_the_name_without_extension` |
| L03-08 | Low | `modlight-app/src/state.rs:302` (`save_config`) | Every settings save copied the library's current folder (and page size) into `ModlightLastFolderId`/`ModlightPageSize`. A session started signed out never takes the remembered folder up, so the first save of anything (theme, panel, quitting) wrote "no folder" over it; a sign-out did the same. Upstream writes them only in `RememberFolder`/`RememberPageSize`. | Signed-out state with a remembered folder, one save: folder "". | Fixed `aefd38a`; `myspace::actions::tests::an_unrelated_save_keeps_the_remembered_page_size_and_folder` |
| L03-09 | Low | `modlight-app/src/myspace/model.rs:501,513` | The loose-files shelf's "Files: N" and size ignored loose files that were moved, trashed, deleted or restored (tally keyed by the file's folder id, null for exactly those files). Same upstream (`AdjustFolderTally` returns on null). | Moving a loose file into a folder left the shelf at 3 files. | Fixed `f75036a`; `myspace::model::tests::loose_files_leaving_or_arriving_correct_the_shelf` |
| L03-10 | Low | `modlight-core/src/library/caches.rs`, `modlight-app/src/myspace/actions.rs:476` | The still and motion preview caches are keyed by code; a custom URL left the bytes (and a motion refusal) under the old code, which the server frees: a file later given that code was drawn with this file's picture. Same upstream. | Rename a→"logo", b→a's old code: b found a's cached bytes. | Fixed `d5a5ac0`; `myspace::actions::tests::a_new_custom_url_takes_the_cached_previews_with_it`, `library::caches::tests::a_renamed_code_takes_its_bytes_and_frees_the_old_one` |
| L03-11 | Low | `modlight-app/src/media.rs` (caches kept for the session) | Codes are unique on one server only; after signing in to another deployment, a file there with a cached code (a vanity code such as "logo") got the other server's picture or refusal. Same upstream (static caches). | Switch the account to another host: "logo" still cached. | Fixed `7697bfd` (+ docs `14ada43`); `myspace::actions::tests::another_server_does_not_see_this_ones_cached_previews`, `library::caches::tests::clearing_forgets_bytes_and_refusals` |
| L03-12 | Low | `modlight-app/src/myspace/view.rs:98`, `media.rs:379` | Hover/unhover were seen only by a card that was drawn: a card that left the screen with the pointer on it (wheel scroll, folded section, another tab) stayed hovered and its reel requested a repaint every frame delay, unseen, until the pointer reached another card. | Harness frame with the playing card not drawn: still hovered and playing; tick behind another tab: same. | Fixed `31214e2`; `myspace::view::tests::a_card_that_is_not_drawn_stops_moving`, `another_tab_stops_the_moving_preview` |
| L03-13 | Low | `modlight-app/src/myspace/model.rs:435` (`merge_newest`), `actions.rs:476` | Cards pushed off a full page 1 by new uploads, or moved out of the open folder, kept their textures and their share of the 64 MiB budget until the next listing (upstream's `Merge`/`Remove` call `pictures.Forget`/`motion.Forget`). | Texture and budget entry still present after the merge and after the move. | Fixed `af29336`; `myspace::actions::tests::cards_that_leave_the_grid_give_their_pictures_back` |
| L03-14 | Medium | `modlight-app/src/media.rs:222,298` (`clear_grid`, `fetch_thumbnail`) | Upstream cancels the listing's token on every folder/page/account change and every preview fetch carries it. The port let the old page's fetches run to their 45 s deadline while the reset queue started four more, so quick page turns piled requests (each a render on the server) far past the four slots, and an old completion freed a slot the new page held. | Silent stub: 9 connections open after one page turn (expected 4 + the listing). | Fixed `5dfd16e` (+ lint `1b21940`); `media::tests::turning_the_page_stops_the_old_pages_previews` |
| L03-15 | Low | `modlight-app/src/commands.rs:117` (`ShowMainWindow`) | Upstream's `ShowAndActivate` refreshes the account, the library's account and the folder list (folders change in the web panel while the window is in the tray); the port's "Show Modlight" only raised the window. | Stub server: no `GET /api/folders` after Show. | Fixed `ca109bf`; `myspace::actions::tests::showing_the_window_reads_the_folders_again` |
| L03-16 | Low | `modlight-app/src/dialogs/link_settings.rs:338` | Save is the window's default button upstream (`IsDefault`); Enter did nothing. | Harness: Enter, no outcome. | Fixed `cbad8eb`; `dialogs::link_settings::tests::enter_saves_as_the_default_button_does` |
| L03-17 | Low | `modlight-app/src/myspace/pager.rs:289` | Upstream's jump box empties itself on Escape (`OnJumpKeyDown`); the port left the abandoned number in it. | Harness: typed "7", Escape, still "7". | Fixed `9159f42`; `myspace::pager::tests::escape_gives_up_on_the_typed_page` |
| L03-18 | Low | `modlight-api/src/error.rs:113` (`Display` of `ApiError::Status`) | The library's "could not reach" line quotes a failed read's message; .NET's is "...: 500 (Internal Server Error)." and MACOS.md says the reason phrase is kept; the port dropped it. | `ApiError::Status(500).to_string()` lacked the phrase. | Fixed `77b42c2`; `error::tests::a_failed_read_says_its_status_as_dotnet_does` |
| L03-19 | Low | `modlight-core/src/library/pager.rs:26`, `folders.rs:149` | The listing total and folder counts/sizes are server numbers; `total + per_page - 1`, `page * per_page` and the roll-up sums overflowed near `i64::MAX` (panic in a debug build, negative pages/counts in release). | `page_count(i64::MAX, 120)` and a roll-up of two huge folders panicked. | Fixed `e60027a`; `library::pager::tests::absurd_totals_do_not_overflow`, `library::folders::tests::absurd_counts_do_not_overflow_the_roll_up` |
| L03-20 | Low | `modlight-app/src/myspace/actions.rs:779` | Empty trash sums the trashed rows' sizes with `+=`; an absurd total panicked the counting task (debug build), so no question came and the busy flag stayed set (button dead). Release wrapped to a wrong size. | Stub listing two rows of `i64::MAX/2+1`: worker panic, no question. | Fixed `5231ad7`; `myspace::actions::tests::an_absurd_trash_still_asks_before_emptying` |
| L03-21 | Low | `modlight-app/src/dialogs/duplicates.rs:163` (`selected_bytes`) | Same kind of sum for the trash button's label: an absurd size panicked drawing (debug). | Scan with `i64::MAX` sizes panicked. | Fixed `b16e3e2`; `dialogs::duplicates::tests::absurd_sizes_do_not_overflow_the_selection` |
| L03-22 | Medium | `modlight-app/src/myspace/actions.rs:806` | The app has ONE confirmation window. Empty trash kept its busy flag while its question waited there; a question asked elsewhere meanwhile (a desktop notice's "Delete file", the components window, settings) replaced it, the flag was never cleared, and Empty trash stayed disabled until restart. | Question replaced and answered, a second Empty trash did nothing. | Fixed `6d1b6ef` (flag held while counting and deleting, not while the modal question is up); `myspace::actions::tests::a_replaced_empty_trash_question_does_not_disable_the_button` |

Commits on the branch that are not fixes: `058ca5e` (rustfmt of the fixes), `14ada43` (docs).

### Not fixed

| Id | Severity | Where | What | Why not |
|---|---|---|---|---|
| N-01 | Low | `dialogs/link_settings.rs` (`numeric`, view limit) | The spinner computes `current + 1` on the typed number, which overflows at `i64::MAX` (debug panic only; release wraps to 1); a number above the server's `integer` column (2 147 483 647) is sent and refused by the server (reported as a refusal); a number past `i64` parses as "no limit" (the state line then says so before saving). | Needs a 10+ digit view limit typed by hand; nothing silent or lost. Upstream throws on the `decimal`-to-`int` cast in the same place. |
| N-02 | Low | `myspace/menus.rs`, `actions.rs::new_folder_prompt` | Inside the loose-files shelf "New subfolder" sends `parentId: "root"` and says "inside Unfiled"; the server maps "root" to the top level, so a top-level folder is made. | Same as upstream; only the wording is off. |
| N-03 | Low | `actions.rs::create_folder` then `move_files` | "Move to new folder" refreshes the folder list and moves at once; if the list is answered after the move landed and applied before the move's completion, the new folder counts the moved files twice until the next folder load. | Same as upstream; timing-dependent display only. |
| N-04 | Low | `actions.rs::duplicates_action(Close)` | Closing the duplicates window while its trash request is in flight loses the count, so the grid is not reloaded. | Same as upstream (the count is read when the window closes). |
| N-05 | Low | the one confirmation window (`Windows::confirm`) | A Trash / Delete forever / duplicates question can still be replaced by a question asked elsewhere and is then dropped without a word (nothing is deleted; nothing stays stuck after L03-22). | Changing the app's single-confirmation model is outside this area. |

## Checked and OK

- API (`library.rs`, duplicates in `extras.rs`, models): query string order and escaping as
  `GetUploadsAsync`; 401/403 as unauthorized on reads and JSON writes, refusal body parsing;
  thumbnail and motion size fences, 413/415 as refusals and anything else as "not now";
  `check_conversion` verdicts (timeout/offline/missing/header reason); `DELETE /api/uploads`
  modes, `affected`-not-status for trash; keep/count-again bodies; PATCH fields absent vs
  null; redirects never followed; links rewritten onto the account's host. The reference
  server tests of the area pass (`folders_files_links_and_the_trash`,
  `deleting_a_folder_alone_keeps_its_files`, `duplicates_are_found_kept_and_counted_again`,
  `links_from_the_vanity_host_are_fetched_on_ours`, and the app's
  `the_app_signs_in_and_manages_the_library_on_the_reference_server`).
- Core: pager arithmetic and strip against `LibraryPager`/`LibraryPagerViewModel` (sizes,
  clamping, anchor on size change, jump parsing); `ThumbnailQueue` and `visible_band` against
  `ThumbnailQueue.cs`/`VisibleBand.cs`; `ThumbnailBudget` (pinning, touch, never the most
  recent) against `ThumbnailBudget.cs`; `ThumbnailCache` FIFO 400 and `MotionCache` LRU
  16 MiB with "never down to nothing"; motion decode plan against `MotionFrameBudget` and
  `AnimatedThumbnail` (first N frames, delay clamp); "Download as" options for every kind and
  MIME branch against `ModlightDownloadMenu`, hints and refusal sentences against
  `ModlightConvertText`; type families, labels, plates and emblems against `CloudFileType`;
  the five link formats against `CloudFileViewModel.LinkFormat`; `CloudFile::apply` against
  `Apply` (copy vs open link, media base, trashed rows); folder roll-up, depth, cycles, crumbs
  and tile meta against `CloudFolderViewModel`; `cloud_size` and `age_of` against
  `ModlightFormat`/`AgeOf`.
- App: navigation, views, "go up", the no-op-unless-failed rules, the restored folder and
  page size, account identity changes, stale completions dropped by generation (`Owns`),
  merge coalescing and the page-1 rule, PATCH rows applied in place, moves (tallies before
  rows, cards dropped only inside a folder); menus (background, folder, file, trash, copy as,
  download as with the frame heading, move to with the shared-folder tick) against
  `MainWindow.MySpace.Actions.cs`; keys against `HandleMySpaceKey`/`HandleLibraryDeleteKey`;
  drag and drop (selection carried, trash refuses, already-there refused, loose shelf means
  "no folder"); details drawer (follows the selection, closes when its file leaves, steps
  move the selection); folder delete dialog (two paths, five-second arming that repaints,
  Enter keeps contents as `IsDefault`); prompt (trimmed, empty refused in the window, Enter
  confirms); confirm (defaults to No); duplicates (oldest kept, kept sets untouched,
  500-id batches, rescan keeping the status line); empty trash (paged count stopping on no
  new ids, 500-id batches, reload after).
- `media_windows.rs` and `qa_media.rs`: window plumbing and QA fixtures of the components,
  update and Download screens; no library logic, nothing found.
- Snapshots of `my-space*`, `duplicates`, `folder-delete`, `link-settings`, `prompt`,
  `confirm` and `message-box` unchanged.

## Shared files touched outside the area

- `app/crates/modlight-app/src/state.rs` (`save_config` no longer writes the library's
  folder and page size: L03-08).
- `app/crates/modlight-app/src/commands.rs` (`ShowMainWindow` refreshes like
  `ShowAndActivate`: L03-15).
- `app/crates/modlight-api/src/error.rs` (`ApiError::Status` message: L03-18).
- `app/docs/MACOS.md` (five bullets in "Behaviour that differs on purpose", inserted after
  "Deleting the folder you stand in").

## Note on test runs

Once, by mistake, the whole `modlight-api` test suite ran (all of `reference_server.rs` and
`uploads.rs` against 8420, other areas' tests included); every test there makes and removes
its own data and all passed. All later runs were limited to this area's tests.
