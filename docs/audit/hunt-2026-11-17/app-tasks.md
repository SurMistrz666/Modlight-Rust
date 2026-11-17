# Bug hunt 2026-11-17 (round 2): app-tasks

Area: the desktop app's task pipeline (after-capture and after-upload work, upload,
history), file naming and destinations, the upload queue's hand-over, settings sync.
Compared with upstream ShareX / Modlight-App in `upstream/`.

## Flows traced

1. Capture / file / text task: `WorkerTask` constructors -> `start` -> `thread_do_work`
   (gate, `do_thread_job`, after-capture jobs in upstream's order, file jobs, OCR) ->
   `do_upload_job` (large-file warning, sign-in gate, before-upload window, retries,
   hand-over to the queue) -> `finally` (clipboard clear, "Delete file locally") ->
   `do_after_upload_jobs` (regex, https, shortener, share, copy, open, QR) ->
   `on_task_completed` -> `TaskManager::task_completed` (history, recent, sound, notice,
   after-upload window) -> the window's effect handler (`uploads.rs::task_effect`).
2. Naming: `TaskContext::file_name` / `naming::file_name` (pattern vs active-window
   pattern, counter, time zone, lengths), `screenshots_folder` (override, sub-folder
   patterns), `handle_exists_file`, and every place that moves the name counter
   (tasks, the naming page, the editor's own save).
3. Hotkey task settings: `hotkeys::resolve_task_settings` (per-group "use default"),
   and which settings each effect is carried out with in the window.
4. Upload queue hand-over: `hand_over_to_queue` -> `UploadQueue::try_take_over` /
   `attach` -> `send_now`, `due` (account switch), `pump`.
5. Settings sync: `tick_settings_sync` -> `run_round` -> `settings_round_done` /
   `apply_report` (who wins, account switch while a round is out).

## 1. Files read

- [x] `modlight-core/src/tasks/`: `mod.rs`, `worker.rs`, `manager.rs`, `context.rs`,
  `gate.rs`, `uploader.rs` (upload part), `naming.rs` (file naming part), `host.rs`
  (effects and questions)
- [x] `modlight-core/src/hotkeys.rs` (task settings resolution)
- [x] `modlight-core/src/upload/queue.rs` (take-over, send now, due, pump)
- [x] `modlight-core/src/sync/plan.rs`, `sync/engine.rs` (`apply_report`),
  `sync/document.rs` (members)
- [x] `modlight-app/src/uploads.rs` (setup, pump, effects), `sync.rs`,
  `settings.rs` (task page counter), `editor_window.rs` (save naming),
  `tools/pin.rs` (`pin`), `tools/jobs.rs` (`OptionsHome`), `tools/qr.rs` (`open_qr_code`)
- [x] upstream: `WorkerTask.cs`, `TaskManager.cs`, `TaskHelpers.cs`
  (`GetScreenshotsFolder`, `HandleExistsFile`), `NameParser.cs` (`Parse`),
  `Modlight/ModlightUploadQueue.cs` (`TryTakeOver`, `Attach`),
  `Modlight/ModlightSettingsSync.cs` (`SyncAsync`, `Adopt`)

## 2. Findings

### TASK-01, medium: "Show QR code" after upload did nothing

`modlight-core/src/tasks/worker.rs` (`do_after_upload_jobs`). The after-upload task
"Show QR code" is offered in the tray/main menus (`uploads.rs`) and in the task settings
(`dialogs/after_capture.rs`), and the QR tool exists (`tools/qr.rs`), but the worker
still carried the phase-4 note "Show QR code arrives with the QR tool (phase 5)" and
never asked for it: ticking it changed nothing. Upstream ends `DoAfterUploadJobs` with
`TaskHelpers.OpenQRCode(Info.Result.ToString())`.

Fix: a new `TaskEffect::ShowQrCode(link)` sent last in `do_after_upload_jobs` (after the
shortener, so a shortened link is the one encoded, as upstream), carried out by the
window with `open_qr_code(Some(link))`.

Test: `tasks::worker::tests::show_qr_code_after_upload_opens_the_link_as_a_code`
(an upload against a stand-in server; no QR effect before the fix).

### TASK-02, low: "Pin to screen" after a capture used the default task settings

`modlight-app/src/uploads.rs` (`task_effect`, `TaskEffect::PinToScreen`). The window
pinned the task's picture with `default_task_settings()`, not the task's. A hotkey with
its own tool settings ("Use default tools settings" off) got the default pin options
(size, border, shadow, opacity...), its own action sound settings were ignored, and
changes made in the pin were saved into the defaults instead of the hotkey. Upstream:
`TaskHelpers.PinToScreen(imageCopy, Info.TaskSettings)`.

Fix: `TaskEffect::PinToScreen` carries the task's settings, and the window pins with
them (as OCR and Analyze image already did).

Test: `tasks::worker::tests::pin_to_screen_carries_the_tasks_own_settings`.

### TASK-03, low (wrong names, can overwrite with "Overwrite"): the name counter set on the naming page did not reach the tasks

`modlight-app/src/settings.rs` (task page, `auto_changed`), `editor_window.rs` (save
without a path), `modlight-core/src/tasks/context.rs` (`auto_increment`). Tasks name files
from `TaskContext::auto_increment`, read once from the settings file at start; upstream
has one `Program.Settings.NameParserAutoIncrementNumber`. Two writers missed one side:

- The naming page wrote only the settings file. Reset the counter to 0 and the next
  capture still went on from the old value (say `%i` = 57 -> 58) and wrote 58 back over
  the reset, so the reset was lost.
- The image editor's own save (no file yet) moved only the tasks' copy, never the
  settings file: after a restart the counter went back and the next names repeated
  (with "File exists: Overwrite" the earlier file is replaced).

Fix: `AppState::set_name_counter` sets both, and the naming page and the editor use it.

Test: `uploads::tests::a_name_counter_set_in_the_settings_names_the_next_capture`
(names `shot-6` after the reset to 0 before the fix, `shot-1` after).

### TASK-04, low (wrong content uploaded): the offline queue sent the original file instead of the processed picture

`modlight-core/src/tasks/worker.rs` (`hand_over_to_queue`). When an upload cannot go out
(no answer, 408/429/502/503/504) the task hands it to the offline queue, which takes the
task's file whenever one is on disk (`TakeOver.file_path`, upstream `Attach` the same).
With "Process images during file upload" the task's file is the ORIGINAL the person
picked, while the upload was the task's own bytes: the picture after image effects, the
editor, a watermark, or another image format. The queue then sent the unprocessed
original under the processed name, e.g. `photo.jpg` holding the PNG it was made from.
The same for a text task saved as a file (the file has a byte order mark the upload does
not), and for a save whose write failed over an older file of the same name.

Fix (not upstream, commented): the file stands in for the upload only when it holds
what the task was sending (no bytes, the same file, or the same bytes); otherwise the
queue spools the task's bytes, as it already did for captures that were never saved.

Test: `tasks::worker::tests::a_processed_picture_is_queued_as_processed` (a 503 stand-in;
fails before the fix on "a copy of the task's bytes, not the original").

## 3. Not fixed

| Id | Severity | Where | What | Recommendation |
| --- | --- | --- | --- | --- |
| NF-01 | Low | `modlight-app/src/sync.rs` (`tick_settings_sync` / `settings_round_done`), `modlight-core/src/sync/engine.rs` (`apply_report`) | A round plans from the document built when it starts. A setting changed here while the round is out (a tray toggle of an after-capture task, a "don't show again", anything outside the settings window, which is the only thing `apply_report` checks) is overwritten when the round ends in Adopt, and the marker then records the server's document, so the change is never pushed. Same in upstream (`SyncAsync` builds `local` first, `Adopt` checks only the settings window). | Needs a change of a few hundredths of a second to a few seconds wide. If wanted: keep the digest of the document the round was planned from and, in `settings_round_done`, skip an Adopt when `document::build(&self.config)` no longer has that digest (the next round then sees a local change and pushes it). |
| NF-02 | Low | `modlight-core/src/tasks/worker.rs` (`do_after_capture_jobs`, `SaveImageToFileWithDialog`) | When writing to the chosen path fails, the save panel opens again with the first name, where upstream opens it with the name just chosen (`Info.FileName`). | Cosmetic; pass `self.lock().info.file_name` on each pass of the loop. |
| NF-03 | Info | `modlight-core/src/tasks/worker.rs` (`thread_do_work`, "Delete file locally") | With "Process images during file upload" a file upload becomes an after-capture job, so "Delete file locally" in the after-capture tasks deletes the ORIGINAL file the person picked once it is up, even with "Use after-capture tasks during file upload" off. Same as upstream (`Info.Job == TaskJob.Job`), and what ShareX means by processing the image as a capture. | The owner's decision; if unwanted, require `use_after_capture_tasks_during_file_upload` for a `Job` made by `file_upload` too. |

Leads checked and found correct: hotkey task settings per "use default" group
(`resolve_task_settings` matches `SetDefaultSettings`, including the destinations group
and the overridden screenshots folder); after-capture order and stop-on-empty-image
(Beautify, effects, annotate end the task, as upstream); file jobs (actions chained on
the previous output, one clipboard action, Analyze / QR scan for pictures only); text
jobs (BOM in the file, none in the upload); `HandleExistsFile` modes; download naming;
the retry loop and the hand-over before the failure is reported; "Delete file locally"
after a hand-over; history and recent list only for tasks with a result; notices and the
after-upload window only with the toast, as upstream; `ProcessTextUpload` order;
`GetScreenshotsFolder` (override, window sub-folder pattern); queue `due` on an account
switch, `send_now`.

## 4. Outside my area

None found.

## 5. Checks run at the end

- `cargo fmt --all`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings` (macOS): clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`:
  clean.
- `cargo test --workspace`: 45 suites, 1070 passed, 0 failed, 4 ignored (the reference
  server tests on 8420/8430 ran).
- Snapshots: none changed. Migrations: none.

## Summary

Found 7, fixed 4 (TASK-01..04, each with a test that fails before the fix), not fixed 3
(NF-01..03, low or the owner's decision). New tests: four in `tasks::worker::tests`,
one in `uploads::tests`. `TaskEffect::PinToScreen` now carries the task's settings and
there is a new `TaskEffect::ShowQrCode`; nothing else in the app matches on them.
