# HANDOFF: app-1 (phases 1 and 2 of the macOS Rust port of Modlight App)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-1`, branch `app-1`, everything
under `app/`. Never pushed. Rules: `app/docs/PORT.md`.

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd app
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # includes the reference-server tests (see below)
cargo build -p modlight-app
# headless render of one screen, never a window:
target/debug/Modlight --qa-shot my-space /tmp/x.png [--qa-lang pl] [--qa-skin <id>] [--qa-light] [--qa-size 1486x943] [--qa-scale 2]
```

All four were green at the last commit. Never launch `Modlight` without `--qa-shot`
from automation: a plain run opens a window and reads the Keychain.

## Done

Phase 1 (library, sign-in, settings shell, notifications, tray) and its leftovers
are finished: snapshot tests (`tests/snapshots.rs`, 4 looks per QA screen),
`scripts/build-app.sh` (never launches the app), `app/docs/MACOS.md`,
`app/docs/STATUS.md`. See STATUS.md for every screen.

Phase 2 (uploads) is finished:

- `modlight-core`: `names` (NameParser, UploadInfoParser, file and URL helpers),
  `history` (History.db, upstream's table and value shapes), `watch` (watch folders
  over FSEvents), `dotnet` (Newtonsoft date shapes), `upload/*` (offline queue:
  items, Queue.json and spool, sentences, progress, worker, tus transport),
  `tasks/*` (TaskSettings + AppUploadSettings read from the config, TaskInfo,
  WorkerTask, TaskManager, UploadManager, upload gate, uploader glue with upstream's
  error texts, paste model, recent tasks, image saving).
- `modlight-api`: `mime`, `upload` (one streamed multipart `POST /upload`, ShareX's
  bytes), `tus` (create/HEAD/PATCH/DELETE, resume, stall detection, session moved onto
  the account's origin), `web` (Upload from URL, no credential).
- `modlight-macos`: `network` (getifaddrs), `sound` (NSSound), `keys` (Cmd+V local
  monitor), `system::{open_path, move_to_trash, active_window_fullscreen}`.
- `modlight-app`: `uploads.rs` (the window's side: task questions one at a time,
  effects, every upload window, many-files questions with folders, watch folders,
  menus, notice actions), `upload_tab.rs`, dialogs `message`, `upload_confirm`,
  `file_exists`, `before_upload`, `after_upload`, `paste`, `text_upload`,
  `url_upload`, `shorten_url`, `clipboard_upload`, `drop_window`; tray built from
  menu entries (Upload, after-capture/after-upload toggles, screenshots folder, recent
  items) with the progress icon; window title with the average progress; dialogs
  centred on the screen and Topmost where upstream's are. Phase-1 direct upload is
  gone: drops, "Upload here", folder tiles go through UploadManager.
- QA: `qa_uploads.rs` adds 16 screens (`tab-upload-busy`, `paste`,
  `paste-languages`, `text-upload`, `url-upload`, `shorten-url`, `clipboard-upload`,
  `clipboard-upload-text`, `drop-window`, `before-upload`, `after-upload`,
  `large-file`, `multi-upload`, `file-exists`, `message-box`, `notification-upload`);
  snapshots for all of them in 4 looks.
- Tests against 8420: `modlight-api/tests/uploads.rs` (14: live upload bytes,
  progress, cancel, refusals; tus small/empty, cut + closed port + restart resume,
  stall, abandon, refusals; scripted protocol cases), `modlight-core/tests/task_flow.rs`
  (6: small upload, text + paste, shortener + code-taken refusal, signed out, offline
  queue across a restart, 24 MiB queued upload cut at 8 MiB and resumed after a
  restart), `modlight-app/tests/reference_app.rs` (2: the phase-1 library run, now
  uploading through the task flow; eleven files through the many-files question, a
  paste, the history read back). Everything uploaded is deleted afterwards.
- A review of the upload paths against upstream (WorkerTask, TaskManager,
  UploadManager, notice) was done and its findings fixed (commit `cc585f0`).

## Not done / for phase 3

1. Not verified in a real window (only headless): every upload window, the drop
   target, Cmd+V, the tray's new submenus and progress icon, sounds, Topmost dialogs,
   the clipboard (writing and reading). Needs a run with the owner (`--qa-profile`).
2. Settings pages for upload options (copy formats, clipboard upload, limits,
   history, watch folders) are phase 3; the settings are read from the config and
   honoured, only their pages are not drawn.
3. History windows (`ShareX.HistoryLib` views), captures, hotkeys, the image editor,
   pin to screen: phase 3+. The notice's "Annotate image" and "Pin to screen" are
   no-ops until then.
4. Upstream's desktop app has no duplicate check before uploading (only the web panel
   calls `POST /api/uploads/check`), so none was ported. Ask the owner if one is wanted.
5. A Finder drop onto a folder tile uploads into the open folder (winit gives no
   pointer position during an OS drag).
6. Proxy settings are not applied to uploads (upstream reads them from its proxy page).

## Decisions and known differences (for MACOS.md / STATUS.md)

- Font: Inter (upstream's Avalonia fallback on a Mac); the Windows screenshots use
  Segoe UI, whose line height is 1.33 vs Inter 1.21, so text-driven heights are a few
  px shorter (cards 195 px vs 201 px). Monospace is egui's Hack, not Courier New.
- Title bar: native traffic lights over a full-size content view; logo and title start
  at x=78; drag/zoom handled by the band. No custom caption buttons.
- Buttons' labels and icons are Text.Secondary (the global TextBlock style), only
  `.active` buttons repaint them; widgets.rs follows that.
- Keys: Backspace / Alt+Left / Cmd+Up go up; Delete (Fn+Backspace) or Cmd+Backspace
  trash; Shift or Cmd+Option+Backspace or the trash view delete for good; Cmd = Ctrl.
- Toasts: newest at the top of the stack (upstream `Insert(0)` order); empty name and
  detail lines keep their line height, as Avalonia does.
- Notice shadow: egui has no negative spread; blur/alpha approximated.
- Language applies live (upstream asks for a restart); only Automatic/English/Polish.
- QA downloads go to `<qa-profile>/Downloads` without a save panel.
- Upload/Download/Server tabs show their plan cards with `MainWindow_PlanLater`;
  Capture and Tools draw the real tiles/cards and answer "built in a later stage".
- Settings window: every page in the navigation; unimplemented pages show their header.
- Deleting the folder you stand in reloads the folders (upstream loses that refresh).
- Tray (Normal mode only): Sign in / Signed in as (account submenu), Settings, Show
  Modlight, Exit. Exit from the tray calls `process::exit` because a hidden window may
  not be redrawn; menu items needing UI un-hide the windows through AppKit first.

## Gotchas

- Phase 3 capture tasks start through `modlight_core::tasks::upload_manager::run_image_task`
  / `WorkerTask::image_upload`; the after-capture jobs (`do_after_capture_jobs`) and
  `TaskQuestion`s are already wired to the window. Add capture questions as new
  `TaskQuestion` variants and windows in `uploads.rs::open_next_ask`.
- The task flow never touches a window: it sends `TaskMessage`s over `TaskChannel`;
  `AppState::pump_uploads` (each frame) answers them. Tests can play the window
  (`modlight-core/tests/task_flow.rs::Host`).
- A restored queue item is due at once (`QueueItem::revive`); tests must wait for the
  outcome, not for the item.
- egui ignores `TextEdit::margin` when a frame is given: use
  `Frame::NONE.inner_margin(n)`.
- In hand-placed cards, draw buttons in a child `Ui` (`ui.new_child`) so
  `allocate_rect` does not move the list's cursor back up.
- `modlight-core::tasks::manager::set_fullscreen_probe` and
  `modlight_macos::keys::install` are set up in `ModlightApp::new` (Normal and Qa only).

- `Runtime::spawn` takes a future returning a closure `FnOnce(&mut AppState,
  &egui::Context)`; annotate the closure's parameter types.
- Mixing painter-positioned layouts with `ui.allocate_*` moves the cursor: dialogs
  laid out by hand must place their footer with `dialogs::footer_at`.
- `qa::render` uses `run_steps`, not `run()`: some screens request repaints forever
  (indeterminate progress), and `run()` would hit max steps.
- QA renders set thread-local language, culture and UTC (`set_thread_utc`) so output
  does not depend on the machine.
- egui 0.36: `App::ui(&mut self, ui, frame)`, `TextEdit::frame(Frame::NONE)`,
  `ctx.egui_wants_keyboard_input()`, `DroppedFile` is a trait (`f.path()`).

## Created outside the repo

- Reference server accounts and API tokens, recorded (names only here) in
  `/Users/marcinasdfg/Desktop/modlight/_wzorzec/qa-accounts.env`:
  `QA_ADMIN_EMAIL`, `QA_ADMIN_PASSWORD`, `QA_ADMIN_TOKEN_WRITE`, `QA_ADMIN_TOKEN_FULL`,
  `QA_APP1_EMAIL`, `QA_APP1_PASSWORD`, `QA_APP1_TOKEN_WRITE`.
- Scratch: `/Users/marcinasdfg/Desktop/modlight/_scratch/app-1/` (notes, converted
  upstream screenshots in `shots/`, renders in `out/`, small tools in `tools/`).
- No processes left running by this session; the reference stack on 8420 and the
  shared Postgres were not touched.
