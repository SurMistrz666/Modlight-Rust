# Bug hunt 2026-11-11, area app-net: sign-in, sync, uploads, downloads, library, vault, updates

Worktree `_wt/hunt-app-net`, on top of the staged base patch. Everything below is the
unstaged diff; nothing was committed. Line numbers are those of the base unless said.

## 1. Files read

API client (`app/crates/modlight-api`):

- [x] `Cargo.toml`, `src/lib.rs`, `src/client.rs`, `src/error.rs`, `src/host.rs`, `src/account.rs`
- [x] `src/signin.rs`, `src/upload.rs`, `src/tus.rs`, `src/library.rs`, `src/extras.rs`
- [x] `src/downloads.rs`, `src/vault.rs`, `src/web.rs`, `src/models.rs`, `src/metadata.rs`
- [x] `src/mime.rs` (code and tests; the 982-entry table skimmed)
- [x] `tests/*.rs` skimmed for their harnesses (they talk to the reference servers)

Core (`app/crates/modlight-core/src`):

- [x] `upload/{mod, item, store, queue, queue_tests, transport, progress, text, target}.rs`
- [x] `download/{mod, link, tool, run, item, error, format, cookies, version_check}.rs`
- [x] `sync/{mod, engine, plan, document, theme}.rs`
- [x] `update.rs`, `update/{codesign, install, helper}.rs`
- [x] `library/{mod, caches, budget, queue, files, folders, pager, convert}.rs`
- [x] `vault.rs` (the grant keeper; the 30-day memory lives here)
- [x] read for context: `components/{download, archive}.rs` (the updater's fetch and
      unpack helpers), `tasks/{uploader, worker}.rs` (the live upload's tus branch and
      Upload from URL)

App (`app/crates/modlight-app/src`):

- [x] `signin.rs`, `sync.rs`, `uploads.rs`, `upload_tab.rs`, `download_tab.rs`, `vault.rs`
- [x] `update_window.rs`, `personal_folder.rs`, `admin_tab.rs`
- [x] `myspace/{mod, actions, model, view, drawer, details, menus, drag, pager}.rs`
      (`view`, `drawer`, `menus`, `drag`, `pager`, `details`: drawing code skimmed for
      indexing and arithmetic; the logic read fully)
- [x] `dialogs/{after_upload, before_upload, clipboard_upload, text_upload, url_upload,
      upload_confirm, shorten_url, paste, duplicates, file_exists, folder_delete,
      link_settings}.rs`

Platform:

- [x] `modlight-macos/src/{keychain, network, webview, pasteboard, integration}.rs`
- [x] `modlight-windows/src/{credentials, network, webview, pasteboard}.rs`,
      `webview/host.rs` (folder sweep, cookie harvest, lifetimes),
      `integration.rs` (pipe protocol, request decoding, path checks)

## 2. Findings

Each test below was run against the unfixed code first and failed there (a hang caught by
the test's own deadline counts as failing), then passed with the fix.

### NET-01 (medium) My space downloads never end when the server stops sending

`app/crates/modlight-api/src/library.rs:347` (`Api::download_to_file`).

What went wrong: "Download" and "Download as" in My space stream the body into a hidden
part file beside the chosen destination, with no deadline of any kind (the shared client
has a connect timeout only, and `content_request` sets none). A link that drops mid-body
without a reset (a laptop that sleeps, a NAT that forgets the connection, a proxy that
hangs) left the download waiting for ever: the "download started" notice was the last
word, and the hidden `.<name>.<tag>.part` file stayed in the person's folder until the
app quit (and after: nothing removes it then). Upstream's `DownloadToFileAsync` runs on
`HttpClientFactory`'s client, whose 100 s timeout ends it.

Fix: the body may bring nothing for at most `DOWNLOAD_STALL_TIMEOUT` (100 s, measured from
the last bytes, so a large file on a slow line still arrives), and the answer's headers
must come within `DOWNLOAD_ANSWER_TIMEOUT` (300 s: a conversion is made before its first
byte and the server allows its longest one 150 s). Either ends the download as
`ApiError::Timeout`, through the existing path that removes the part file and leaves the
destination as it was.

Tests: `library::tests::a_download_that_stalls_ends_and_cleans_up` (10 of 1000 bytes, then
silence: before the fix still waiting after 10 s), `a_server_that_never_answers_ends_the_download`.

### NET-02 (low) Upload from URL destroyed the file it was told to overwrite, and left half a file

`app/crates/modlight-api/src/web.rs:118` (`download_to_file`, behind
`Api::download_url_to_file`, called by the task's `download_from_url`).

What went wrong: the destination was created (truncated) before a byte of the body had
arrived, and a failure (a cut connection, a stall, a body short of its length) returned
with the half-written file under the real name in the screenshots folder. When the name
was taken and the person chose to overwrite (or `FileExistAction::Overwrite` is set), a
failed download destroyed the old file too. Upstream's `WebHelpers.DownloadFileAsync`
reads the whole answer with `GetAsync` before it opens the file, so a failure there never
touches the disk.

Fix: the body goes to a hidden part file beside the destination (`library::part_path`,
now `pub(crate)`) that is renamed over it once complete and removed on any failure, as
My space's downloads already do (L03-04).

Test: `web::tests::a_failed_download_leaves_the_file_it_would_replace` (before the fix the
old file held "0123456789").

### NET-03 (low) The sign-in's loopback listener spun a core when `accept` kept failing

`app/crates/modlight-api/src/signin.rs:288` (`LoopbackListener::wait_for`); the lead left
by the 2026-10-24 review (its N1).

What went wrong: `let Ok((stream, _)) = accepted else { continue };` went straight back to
`accept()`. A failure that persists (the process out of file descriptors, `EMFILE`) made
the wait a busy loop: 100 % of a core for as long as the person took in the browser (up
to the 10-minute approval timeout).

Fix: a failed `accept` waits `ACCEPT_RETRY` (100 ms) before the next one; Cancel still ends
the wait at once. The loop moved into `wait_accepting`, which takes the source of
connections as a closure (`wait_for` passes the listener's `accept`), so a test can make
`accept` fail without exhausting the test process's descriptors (the reason the earlier
review left it).

Test: `signin::tests::a_failing_accept_does_not_spin` (before the fix about 1.3 million
attempts in half a second; now at most a handful).

### NET-04 (low) A Downloads listing with `"built": -9223372036854775808` panicked the update check

`app/crates/modlight-api/src/downloads.rs:207` (`time_of`).

What went wrong: a number time is read as milliseconds or seconds by `value.abs()`, and
`i64::MIN` has no absolute value: a debug build panicked in the update check's task
("attempt to negate with overflow"), a release build wrapped it and read it as seconds.
The listing is the server's (any administrator's publish, or a proxy), read on every
check.

Fix: `unsigned_abs()`. Such a time is no time (`None`), and the build is then never
offered (`MissingBuilt`), as for any other unreadable time.

Test: `downloads::tests::absurd_times_are_no_time` (panicked before the fix).

### NET-05 (low) The offline queue's room check outlived "Send now" and a change of account

`app/crates/modlight-core/src/upload/queue.rs:799` (`has_room`), `:561` (`send_now`).

What went wrong: before sending, the queue asks `/api/me` how much room the account has
and trusts the answer for a minute, whoever is signed in. Two consequences:

- An item stopped with "There is no room: 1.4 GB needed, 300 MB free"; the person deletes
  files on the website and presses "Send now" (upstream's own comment: "a person pressing
  this is asserting that something has changed - they have freed space"). Within the
  minute the cached answer stopped the item again, with the old figures, without asking.
- Signed out of a full account and into another one within the minute: the new account's
  items were stopped as "no room" with the previous account's numbers.

Upstream has the same static `quotaAskedUtc`/`quotaRemaining`.

Fix: the answer is kept with the account it is about (`quota_for`, server and tag) and
counts as stale for any other; `send_now` drops it. Recorded in `app/docs/MACOS.md`,
"Behaviour that differs on purpose".

Tests: `upload::queue::tests::send_now_asks_for_the_room_again`,
`the_room_of_one_account_is_not_another_ones` (both stopped the item again before the fix).

### NET-06 (low) "Annotate image" ran on a notice of a file that is not a picture

`app/crates/modlight-app/src/uploads.rs:713` (`notification_action`); a lead of the
2026-10-24 review.

What went wrong: the notice's buttons are offered only for pictures (`can_execute`), but a
click on the notice itself runs the click action whatever the file: the middle click's
default is `AnnotateImage`, so middle-clicking the notice of an uploaded PDF or zip sent
it to the editor, which put up a failure notice naming the file. Upstream's
`ExecuteAction` runs `AnnotateImageFromFile` only when `FileHelpers.IsImageFile` says so,
and does nothing otherwise.

Fix: the same check before `annotate_file`.

Test: `uploads::tests::annotate_from_a_notice_is_for_pictures_only` (a PDF raised a corner
notice before the fix; a PNG still opens the editor).

### NET-07 (medium) A question asked while another was up replaced it, and the first was lost

`app/crates/modlight-app/src/dialogs/mod.rs:45` (`Windows::confirm`, one slot), and every
`windows.confirm = Some(...)`: `myspace/actions.rs` (Trash / Delete forever, Empty trash,
duplicates' Trash), `uploads.rs:764` (a notice's "Delete file"), and outside the area
`components_window.rs:1217` (Remove a component), `app_pages.rs:2162` (Reset settings),
`tools/image_effects.rs:2458` (enable image effects). The 2026-10-24 review's N-05 and a
lead of this hunt.

What went wrong: the app has one confirmation window, and each of these assigned it. A
question asked while another was up (the empty-trash count finishing while the person
answers a notice's "Delete file", the components window's Remove over a Trash question)
took its place: the first question vanished with no answer and nothing said, and what it
was about simply did not happen. Upstream's questions are modal message boxes, answered
one at a time.

Fix: `AppState::ask_confirm` puts a question up, or lines it up behind the one on screen
(`Windows::confirm_waiting`; the same question twice is asked once), and `confirm_finished`
brings the next one up. Every site above goes through it (one line each in the three files
outside the area). `screen_recording_refused`, which already refuses to replace another
question, is unchanged.

Test: `uploads::tests::a_question_asked_while_another_is_up_waits_its_turn` (before the fix
the notice's question replaced the Trash question). The existing
`a_replaced_empty_trash_question_does_not_disable_the_button` still passes; its comment
was brought up to date.

### NET-08 (low) An absurd size from yt-dlp overflowed the Download tab's progress sum

`app/crates/modlight-core/src/download/run.rs:262,275-276` (`LineReader::read_progress`).

What went wrong: the progress of a download made of several streams is the sum of the
finished streams and the current one, added with `+`. The sizes are what the site says
(yt-dlp passes `Content-Length` on), so a huge one overflowed: in a debug build the
reader task panicked holding the reader's lock, the run then panicked on the poisoned
lock and the row stayed "Downloading" for good; a release build wrapped to a negative
size.

Fix: saturating additions.

Test: `download::run::tests::absurd_sizes_do_not_overflow` (panicked before the fix).

### NET-09 (low) Absurd `affected` counts stopped Empty trash for the rest of the session

`app/crates/modlight-app/src/myspace/actions.rs:898` (`delete_trash`), `:1513`
(`trash_duplicates`, and `window.trashed`).

What went wrong: the same kind as L03-20 (which saturated the trash's byte sum): the
batches' `affected` counts, the server's numbers, were added with `+=`. Past `i64::MAX` a
debug build panicked the deleting task, its completion never ran, and `emptying_trash`
stayed set (the button did nothing until restart); a release build reported a negative
number of files deleted.

Fix: saturating additions in both loops and in the duplicates window's running total.

Test: `myspace::actions::tests::absurd_counts_do_not_stop_emptying_the_trash` (two
batches of `i64::MAX`: before the fix the worker panicked and the flag stayed set).

### NET-10 (low) An absurd quota from `/api/me` panicked the sign-in window

`app/crates/modlight-app/src/signin.rs:156` (`SignInWindow::show_account`).

What went wrong: the account card's "left" figure fell back to `max - used`, computed
eagerly (`unwrap_or`) even when the server sent `remaining`. A negative `used` (a server
or proxy answering nonsense) overflowed: a debug build panicked on the window's thread
(the whole app), a release build showed a wrapped figure.

Fix: `unwrap_or_else` with `saturating_sub`.

Test: `signin::tests::an_absurd_quota_does_not_overflow_the_card` (panicked before the
fix; an ordinary quota still reads 3 KB left of 4 KB).

### NET-11 (medium) A live upload whose link died uploaded for ever and never reached the queue

`app/crates/modlight-api/src/upload.rs:253` (`Api::upload`), used by every upload of a file
up to 50 MiB (nearly every capture).

What went wrong: the live upload had no deadline of any kind ("a capture of several
gigabytes legitimately takes hours"; those go over tus now, which has a stall watch). A
socket that neither delivers nor fails (a laptop closed mid-upload, a NAT or proxy that
dropped the connection silently) left the task "uploading" at its last percentage for
ever: it never failed, so the offline queue, which takes exactly the uploads that got no
answer, never got it, and only Stop ended it. Upstream's `HttpWebRequest` runs with
`ReadWriteTimeout` (five minutes by default) bounding every read and write.

Fix: a stall watch as tus has: nothing of the body taken by the connection and no answer
for `LIVE_STALL_TIMEOUT` (300 s, `ReadWriteTimeout`'s default; the wait for the answer
after the last byte included, as in tus) ends the upload as `LiveOutcome::NoAnswer`
("The request timed out."), which the task reports and hands to the offline queue like
any unanswered upload. No overall deadline: a slow line that keeps moving is never cut.

Test: `upload::tests::an_upload_nobody_answers_ends` (a server that takes the request and
never answers: before the fix still uploading after 10 s).

## 3. Not fixed

| Id | Severity | Where | What | Why not / recommendation |
|---|---|---|---|---|
| NF-01 | Low | `modlight-app/src/uploads.rs:1477-1490` (`pump_watch_folders`) | A watched file moved to the screenshots folder takes a unique name when the name is taken, but between that check and `fs::rename` (which replaces an existing file on macOS) a file of the same name can appear (a capture saved in the same second) and is then overwritten. Upstream's `File.Move` refuses an existing destination. | A race of milliseconds between two writers of the same name. If wanted: move with a no-replace rename (`renameatx_np(RENAME_EXCL)` / `MoveFileExW` without `REPLACE_EXISTING`) and take the next unique name on `AlreadyExists`. |
| NF-02 | Low | `modlight-app/src/signin.rs:357` (`finish_sign_in`) | A browser sign-in whose result was already computed when the person closed the window is still applied (the completion finds no window and skips the generation check), so closing in that instant still signs in. Generations restart at 0 in a reopened window, so an old completion can also match the new window's generation. | Needs a window closed within milliseconds of the token arriving. A per-app counter for generations (instead of per window) would close both. |
| NF-03 | Low | `modlight-app/src/myspace/actions.rs` (`edit_done`, `files_removed`, `create_folder`... completions) | Completions of edits, moves and deletes started for one account are applied after a switch to another account: they find none of their ids in the new listing (no-ops), but their corner notice ("Moved 3 files", or the old account's failure) still shows. | Cosmetic; the server enforces ownership, so nothing of the new account is touched. Checking `identity_of(credential) == space.identity` in those completions would drop the notices. |
| NF-04 | Info | `modlight-api/src/web.rs` (Upload from URL) | A body without `Content-Length` that ends because the connection closed early is taken as complete (there is nothing to compare it with), and is uploaded as it is. | Same as upstream (`GetAsync` reads to the close); a chunked body that is cut is detected. |

### The brief's leads, and what else was checked

| Lead | Outcome |
|---|---|
| `signin.rs` `accept()` without backoff | Real: NET-03. |
| Async results applied after the account changed or after sign-out | Searched the area: sign-in window, settings and skin sync, update checks, the vault, the Server tab and My space listings already drop answers about another credential (generations, `credential()` checks). Found: the queue's quota answer (NET-05); edit/delete completions only show a stale notice (NF-03). |
| "Annotate image" for a file that is not an image | Real: NET-06. |
| One question window loses a question | Real: NET-07 (queued). |
| Files overwritten where the original refuses or asks | Upload from URL (NET-02); the watch-folder move race (NF-01). My space downloads (part file, after the save panel's own question), the queue file and spool (`create_new`, set-aside names), the cookie jar and the updater (own folders) are fine. |
| Chunked uploads and retries, offsets | tus: chunk offsets, short confirmations, 404/409/410, finished-on-creation, stall watch, checkpoints and the queue's resume all read right; the live upload had no stall watch (NET-11). |
| Downloads without `Content-Length` treated as complete | The updater and components refuse an unknown size; My space and Upload from URL now have deadlines (NET-01, NET-02); NF-04. |
| Tokens and passwords never logged | Checked every `log::` in the area and every `Debug` of `Credential`, `QueueAccount`, `SignedIn`, `UnlockGrant`, `CookieData`: nothing prints a secret. |
| Keychain / Credential Manager errors | Write read back, "not found" as done on delete, UTF-16 values read; a failed delete at sign-out is logged and the account file still records the sign-out (L11). OK. |
| Update checks and the self-updater | https or loopback only, same-origin download with the token, size and SHA-256 before unpacking, Mac zip checked entry by entry before `ditto`, team id + bundle id + designated requirement, Windows zip unpacked with counted sizes and no links, helpers take paths as arguments, roll back on a failed swap. OK apart from NET-04. |
| Vault (file passwords, 30-day memory) | Grant bound to host and tag, never logged, sent only on content requests of its credential, forgotten and revoked on sign-out / switch / Lock / a refused grant, expiry honoured; rate-limit waits bounded. OK. |
| Server numbers cast without checks | NET-04, NET-08, NET-09, NET-10; the remaining casts in the area saturate or are display-only. |

## 4. Outside my area

- `app/crates/modlight-app/src/components_window.rs:1217`, `app_pages.rs:2162`,
  `tools/image_effects.rs:2458`: one line each, `windows.confirm = Some(..)` became
  `ask_confirm(..)` for NET-07 (the fix needs every site that asks a question to queue it;
  without them those questions would still replace a Trash or Empty trash question).
- `app/crates/modlight-app/src/dialogs/mod.rs`: the new `Windows::confirm_waiting` field
  (NET-07).
- `app/docs/MACOS.md`, "Behaviour that differs on purpose": NET-05 (new bullet) and
  NET-01/NET-02 (the "Downloads from My space" bullet extended).

## 5. Checks run at the end

Run in `app/` on the final diff (2026-11-12):

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (with `wincc/env.sh`): clean (only the build script's note that `Modlight.exe`'s
  resources are embedded on Windows builds only).
- `cargo test --workspace`: 45 test binaries, 1004 passed, 0 failed, 4 ignored (the
  ignores are the existing ones). This includes the tests against the local
  reference servers (ports 8420 and 8430: `modlight-api` `reference_server` 12, `uploads` 23, `vault` 9,
  `signin_pickup` 4; `modlight-app` `reference_app` 2; `modlight-core` `task_flow` 7,
  `settings_sync` 4), all of which passed. Per crate: `modlight-api` lib 56,
  `modlight-app` lib 148, `modlight-core` lib 438.

Every new test was also run against the unfixed code (by restoring the old lines for the
run, then putting the fix back and diffing to make sure) and failed there.

No snapshot changed; no `*.old.png` / `*.diff.png` left. No migration.

## Summary

Found 11 and fixed 11 (NET-01 to NET-11: 3 medium, 8 low); 4 noted and not fixed (NF-01
to NF-04, all low or informational). Changes outside the area: one line each in
`components_window.rs`, `app_pages.rs`, `tools/image_effects.rs`, a field in
`dialogs/mod.rs` (NET-07), and two bullets in `app/docs/MACOS.md`.
