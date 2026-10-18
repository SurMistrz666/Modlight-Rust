# HANDOFF: app-1 (phase 1 of the macOS Rust port of Modlight App)

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

- `modlight-api` (committed): typed client for every endpoint the desktop app uses,
  upstream's error/retry/timeout rules, loopback sign-in with the PORT.md fixes,
  12 integration tests against http://127.0.0.1:8420.
- `modlight-core` (committed): resx texts EN+PL generated at build time, skins (14)
  and palettes from `Skins.axaml`, Lucide constants, culture formats, config store
  (unknown members kept), account store, settings-sync document/plan/engine and theme
  sync (ported `ModlightSettingsSyncTests`, `ModlightLibraryTests`), library logic
  (pager, queue, budgets, caches, file/folder models, convert menu).
- `modlight-macos` (committed): Keychain via `/usr/bin/security`, http(s)-only
  `open`, locale/language/reduce-motion from `defaults`, AppKit helpers to un-hide
  windows from the tray and read the screen's visible frame.
- `modlight-app` (committed, compiles, clippy clean):
  - `shell.rs` main window (title band with room for the traffic lights, tab strip,
    storage chip, account button + menu, settings menu, plan cards for Upload /
    Download / Server, Capture tiles, Tools cards, toasts, Finder drop overlay).
  - `myspace/*` header + crumbs, sliding library panel, folder shelf, file grid with
    thumbnails and hover motion previews, type plates, all panels (signed out,
    loading, empty, error), error footer, pager bar, details drawer (every section,
    every link format), all context menus, drag cards onto folders, keyboard
    (mapping documented in `myspace/mod.rs`), every action (move, publish, custom URL,
    link settings, trash/restore/delete forever, empty trash, duplicates, new / rename
    / public / delete folder, download and download-as with conversion check,
    phase-1 direct upload via `POST /upload`).
  - `dialogs/*` prompt, confirm (message box), folder delete (5 s arming), link
    settings, duplicates; `signin.rs`; `settings.rs` (full navigation; account
    pages, Theme and colours, Language and tray filled; other pages header only);
    `notification.rs` (desktop notice viewport); `tray.rs`; `sync.rs`.
  - `qa.rs` fixtures (no network) and `--qa-shot` for 26 screen ids (`qa::SCREENS`).
  - `tests/reference_app.rs`: headless end-to-end run against the reference server
    (sign in with a token, library, folder create/rename/delete, upload, trash,
    restore, delete for good, sign out). Passes.

## In progress / not done (phase-1 deliverables still open)

1. Snapshot tests (`egui_kittest`) for every screen x light/dark skin x EN/PL:
   not written yet. The renderer is ready: `qa::render(screen, &Look)` returns an
   `RgbaImage`; add `crates/modlight-app/tests/snapshots.rs` that loops over
   `qa::SCREENS` and calls `egui_kittest::image_snapshot` (UPDATE_SNAPSHOTS=1 to
   create `tests/snapshots/*.png`). Render at scale 1.0 to keep files small.
2. `scripts/build-app.sh`: not written. Plan: `cargo build --release -p modlight-app`,
   `Modlight.app/Contents/{MacOS,Resources}`, Info.plist, `.icns` from
   `assets/upstream/icons/Modlight_Icon.ico` via `sips` + `iconutil`, ad-hoc
   `codesign --force --sign -`, `codesign --verify`. Must NOT launch the app (the
   reference script in Nerium_panel/.../package-macos.sh launches it: do not copy that
   part).
3. `app/docs/MACOS.md` (substitutions) and `app/docs/STATUS.md` (per screen/feature):
   not written. Content to record is listed under "Decisions" below.
4. Visual comparison: renders were checked by eye against
   `_scratch/app-1/shots/Modlight-App-library.png` (upstream, 1486x943, Classic dark)
   and match closely; known differences are listed below. Light skins and Polish were
   not inspected yet (render with `--qa-light`, `--qa-lang pl`).
5. Not verified in a real window (only headless): tray menu, viewport dialogs sizing,
   window dragging by the title band, hide-to-tray. Needs a human run with
   `--qa-profile /tmp/somewhere`.

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
