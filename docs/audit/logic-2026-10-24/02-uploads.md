# Logic review 2026-10-24, area 02: uploads

Scope: the live upload and the resumable (tus) client (`modlight-api/src/{upload,tus}.rs`),
the offline queue (`modlight-core/src/upload/*`), the task flow
(`modlight-core/src/tasks/*`), watch folders (`watch.rs`), the history store
(`history.rs`), and the window's side (`modlight-app/src/{uploads,upload_tab,qa_uploads}.rs`
and the upload dialogs). Read against upstream's `ModlightUploadQueue`, `ModlightQueueStore`,
`ModlightQueueItem`, `ModlightQueueText`, `ModlightUploadGate`, `ModlightResumableUpload`,
`TaskManager`, `WorkerTask`, `UploadManager`, `TaskInfo`, `WatchFolder*`, the Modlight
uploader, `RecentTaskManager` and the `Presentation` windows of the area. Branch
`logic-02-uploads`, from main `28916b4`. Line numbers are those of `28916b4`.

## Findings

| Id | Severity | Where | What is wrong | How it was reproduced | Status |
|---|---|---|---|---|---|
| U1 | Medium | `modlight-core/src/tasks/worker.rs:1296-1325` (`on_task_completed`, `release_image`); the picture is put in `info.metadata.image` at `:204` and `:668` | A finished image task (every capture, clipboard picture, editor or tool upload) kept its decoded RGBA picture in `info.metadata.image`: only the task's own `image` field was released, and the task list keeps every task for the whole session. Memory grew by one full picture per capture (about 59 MB for a 5K screen) until the app quit. Upstream's `Dispose` frees the one bitmap both fields point at. | Unit test: an image task without upload, with the toast on and off, then the picture's `Arc` count | Fixed, `27f9f56` (+ clippy `7b91cac`); test `tasks::worker::tests::a_finished_image_task_lets_go_of_its_picture` |
| U2 | Medium | `modlight-app/src/uploads.rs:1306` (`update_watch_folders`), `:1340` (`pump_watch_folders`) | Only the default task settings' watch folders were watched. The hotkey settings window has its own watch folders page (`OVERRIDE_GROUPS`), so a folder added there (with that hotkey's watching on) was saved and never watched. Upstream's `UpdateWatchFolders` also adds every hotkey's list and uploads a file from it with that hotkey's settings (`GetSafeTaskSettings`); here even a watched file always went up with the defaults. | Unit test over a QA state with one default folder, one enabled and one disabled hotkey folder | Fixed, `cc9b6b7`; test `uploads::tests::hotkeys_watch_their_own_folders_with_their_own_settings` |
| U3 | Medium | `modlight-api/src/tus.rs:497-513` (`head_offset`), used by `ensure_session:476-493` | A `HEAD` of a stored session answered 408, 429, 502, 503 or 504 was read as "the server forgot the session": a new session was made and the whole file sent again. A proxy's 502/503 while the server restarts (exactly when the queue retries) threw away a large upload's progress. Upstream does the same, though its comment says only 404/410 mean the session is gone, and the queue exists to resume "where it stopped rather than at zero". Now those answers keep the session and end the attempt as that server error (Retry-After honoured), as a `POST` answered the same way would. Upstream bug: noted in `MACOS.md`. | Scripted server test: each status to the `HEAD`, then a 500 that still starts over | Fixed, `776d9f3`; test `a_busy_server_does_not_cost_a_stored_session` (`modlight-api/tests/uploads.rs`) |
| U4 | Low | `modlight-core/src/upload/queue.rs:1036` (`release`) | The DELETE that abandons a cancelled item's tus session was started on `Handle::try_current()`, which fails on the UI thread where the Upload tab's Cancel runs (`commands.rs` `QueueCancel`). Cancelling a waiting or stopped item never told the server; its bytes stayed until the server's daily clean-up (upstream sends it from the thread pool). The queue now keeps its worker's runtime for this. | Unit test calling `cancel` from a thread with no runtime after a pass on a runtime | Fixed, `39b8694` (+ rustfmt `6e63752`); test `upload::queue::tests::cancel_from_the_window_thread_abandons_the_session` |
| U5 | Low | `modlight-app/src/uploads.rs:1101` (`run_clipboard_content`) via `modlight-core/src/tasks/upload_manager.rs:214` | A clipboard picture (Cmd+V, the clipboard viewer, the tray and hotkey "Upload from clipboard") started its task at once. Upstream's `ProcessImageUpload` goes through `RunImageTask`, which shows the quick task menu and the after-capture window when the task asks for them (with `ProcessImagesDuringClipboardUpload` on, the task's own after-capture work applies). Clipboard pictures now go through the window's `run_image_task` with the settings `ProcessImageUpload` makes (`upload_manager::clipboard_image_settings`). | App unit test: after-capture window and quick task menu settings, then what is open and what started | Fixed, `285216f`; test `uploads::tests::a_clipboard_picture_asks_what_the_task_asks_first` |
| U6 | Low | `modlight-core/src/tasks/settings.rs:805` (`large_file_bytes`) | `ShowLargeFileSizeWarning` from the settings file multiplied by 1000*1000 or 1024*1024 unchecked: past about 1.7e13 a debug build panicked in the upload ("attempt to multiply with overflow") and a release build wrapped to a small threshold that asks about every upload. Saturates now. | Unit test with `i64::MAX`, both units | Fixed, `e745f90`; test `tasks::settings::tests::a_huge_large_file_threshold_does_not_overflow` |
| N1 | Medium (data loss) | `modlight-core/src/upload/queue.rs:386-395` (`restore`), `upload/store.rs:71-146` (`load`, `save`) | A `Queue.json` that exists but cannot be read (damaged, unreadable, or a `Version` from a newer build) read as an empty queue: the spool sweep that followed deleted every spooled capture (content that existed nowhere else), and the next change wrote a new file over the uploads it described. Upstream is identical, though its comment says "the files it points at are still on disk either way". Now (coordinator's decision) such a file is moved to `Queue.json.unreadable-<UTC time>` (never over an earlier one; a copy when the rename is refused) before anything saves a new queue, on the first read or on a save that comes first. The debug log says so, and the spool is not swept that run. Upstream bug: noted in `MACOS.md`. | Unit test: a newer version and a damaged file at restore, two in the same second, a save before the read, then a readable queue sweeping again | Fixed, `7a1611c`; test `upload::queue::tests::an_unreadable_queue_file_is_kept_and_so_is_the_spool` |
| N2 | Low | `modlight-core/src/upload/queue.rs:373-395` (`initialize`/`restore`) and `:435-482` (`try_take_over`) | The file is read on the worker after it is spawned. A take-over persisted before that read would be read back as a second copy of the same item (sent twice), and the previous run's items would be overwritten and their spool swept. Same race upstream. | Code reading | Not fixed: not reachable in practice. Tasks start after `initialize` (first frame), and a take-over needs a failed upload plus `MaxUploadFailRetry`'s one-second pause, while the read takes milliseconds. |
| N3 | Medium (data loss) | `modlight-core/src/tasks/worker.rs:447-480` (the `finally` part of `thread_do_work`) | "Delete file locally" deleted the capture whatever became of the upload: after a refusal (507 full, 413, 401...), a failure, a stop or a Cancel in the before-upload window, the only copy was gone and the Upload tab had no file to Retry. Upstream behaves the same, but the fork's own comments (`KeepLocally`, `TryTakeOver`) read the flag as "delete it once it is safely uploaded". Now (coordinator's decision) a task that uploads deletes the file only after the upload succeeded (`do_upload_job` says whether the file went up), and a task with no upload step deletes it as before. Upstream bug: noted in `MACOS.md`. | Unit test: captures against a stand-in server answering 507, 500 and 200, a cancelled before-upload window, and a task with no upload step | Fixed, `645479a`; test `tasks::worker::tests::delete_file_locally_waits_for_the_upload_to_succeed` |
| N4 | Info | `modlight-core/src/tasks/paste.rs:154-165` | The paste window counts characters as `char`s; upstream counts .NET `Length` (UTF-16 units) for the footer and the 5,000,000 ceiling. Differs only for characters outside the BMP (emoji): the footer shows 1 instead of 2 each. | Code reading | Not changed: a paste goes up as a file through `POST /upload`, so the ceiling enforces nothing the server checks. |

## Checked and OK

- **Offline queue** against `ModlightUploadQueue`: which answers are taken over (none,
  408, 429, 502, 503, 504) and only for uploads whose file goes to Modlight; the name
  treated as the live path treats it; DeleteFile stripped on take-over; the live tus
  session carried on only for the same length; spooling only without a file, at most
  512 MiB; backoff ladder, jitter, Retry-After clamped to 1..3600 s; the attempt budget
  spent only on `ServerError` and reset by Send now; progress resetting the backoff;
  NoNetwork/SignedOut stamped and cleared without persisting; another account's items
  skipped with their reason, matched on host (case, trailing slash) and tag; the quota
  check, its 60 s freshness, Refused as TokenRejected; a changed file restarts the session,
  a missing one fails with its path; expiry after 14 days as a history row; Cancel in
  flight left to the worker, the claim held until the ending is recorded; every confirmed
  chunk saved at once; Queue.json written atomically in upstream's shape; the spool's W2
  guard.
- **tus client**: chunks of at most 50 MiB from the confirmed offset, a short or missing
  `Upload-Offset` ending the attempt, 409/404/410 to a `PATCH` keeping the session with
  offset 0, offsets past the end, the empty file finished on creation, the one retry
  without a refused folder, `Location` and stored URLs put on the account's origin, the
  stall watch (no arithmetic underflow), cancel between and during chunks, Retry-After as
  seconds or a date, server text clipped at 200 UTF-16 units.
- **Live upload**: ShareX's multipart bytes, progress through `ProgressManager` (250 ms
  samples, the same speed and ETA rounding as upstream's rows), refusal sentences and the
  error dump, `Modlight_Invalid_response`, the landed folder only for the file service,
  files over one chunk over tus with their session kept between attempts and abandoned
  when nobody will resume it.
- **Task flow**: `WorkerTask` order (gate before the work, KeepLocally, after-capture,
  file and text jobs, large-file question, sign-in gate, before-upload window, retries
  with a 1 s pause, hand-over, after-upload jobs, OnTaskCompleted states), Stop in every
  state, `TaskManager` upload limit and `StartTasks`, history and recent writes with
  `HistorySaveTasks`/`HistoryCheckURL`, queue outcomes into history and recent (successes
  only, spooled paths left out), sounds and notices gated as `PlayNotificationSoundAsync`
  and `Task_TaskCompleted`, the multi-upload question with nested folders, `UploadManager`
  entry points.
- **History store**: upstream's table text, cell shapes (local `ToString("o")`, NULL as
  `""`, Newtonsoft tags), validity rule, migration of History.json.
- **Watch folders**: `FileSystemWatcher` filter matching, the 1 s duplicate timer, the
  1 s + 4 x 250 ms size test within 5 s, renames inside the tree ignored and moves in
  reported, the move to the screenshots folder (unique name, documented).
- **Upload tab**: transfer rows, queue rows (Send now and Cancel wired the right way round,
  countdown wording at 90 s), history rows from tasks, the store and the queue, merge and
  newest-first order, Refresh, Retry only with the file still there.
- **Dialogs** against their `.axaml.cs`: before upload (options, prompt, settings handed
  back), after upload (format order and groups, image-only formats, auto close, buttons'
  enabled states), file exists (new name rules, Enter and Escape, closing overwrites),
  large file and many files, clipboard viewer (don't-show-again, Upload disabled when
  empty), text upload, URL upload (validation), shorten URL (expiry list, use-tag default
  on), paste (title sanitising, empty paste refused), drop window (size, opacity, corner).

## Tests run

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test -p modlight-core`, `cargo test -p modlight-api`, `cargo test -p modlight-app --lib`,
  and the snapshot test for the area's 17 screens (`QA_SCREEN=<screen>`), all in 4 looks.
- Against the reference server (each test deletes what it uploaded):
  `modlight-api/tests/uploads.rs` (23) and `modlight-core/tests/task_flow.rs` (7).
- After the second round (N1, N3): fmt and clippy again, `cargo test -p modlight-core`
  (task flow against the reference server included) and `cargo test -p modlight-app --lib`;
  `modlight-api` was not changed by it.
