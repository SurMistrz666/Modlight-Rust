# Bug hunt 2026-11-11: app-ui (main window, settings, config, names, components, small dialogs)

Worktree `_wt/hunt-app-ui`, branch `hunt-app-ui`. Work is the unstaged diff on top of the
staged base patch. Status: finished 2026-11-12 (a usage limit stopped the run once; it continued from this file and the diff).

## 1. Files read

Read top to bottom (the drawing-only stretches of the big UI files skimmed for logic,
indexing, casts and slicing; every function with data in it read):

- [x] `modlight-app/src/`: `app.rs`, `app_pages.rs`, `settings.rs`, `settings_kit.rs`,
  `theme.rs`, `widgets.rs`, `side_panel.rs`, `shell.rs`, `state.rs`, `runtime.rs`,
  `controls.rs`, `about_window.rs`, `components_window.rs`, `first_run.rs`,
  `information.rs`, `notification.rs`, `task_editors.rs`, `task_pages.rs`, `media.rs`,
  `media_windows.rs`, `main.rs`, `lib.rs`, `win32.rs`, `qa.rs`, `qa_settings.rs`,
  `qa_showcase.rs`, `qa_uploads.rs`, `qa_media.rs`
- [x] `modlight-app/src/dialogs/`: `chrome.rs`, `confirm.rs`, `message.rs`, `prompt.rs`,
  `drop_window.rs`, `mod.rs`
- [x] `modlight-core/src/` top level: `account.rs`, `admin.rs`, `background_removal.rs`,
  `backup.rs`, `build_info.rs`, `config.rs`, `culture.rs`, `debug_log.rs`, `dotnet.rs`,
  `folders.rs`, `format.rs`, `history.rs`, `hotkeys.rs`, `icons.rs`, `image_decode.rs`,
  `l10n.rs`, `lib.rs`, `messages.rs`, `theme.rs`, `update.rs`, `vault.rs`, `watch.rs`
- [x] `modlight-core/src/names/`: `file_helpers.rs`, `helpers.rs`, `mod.rs`,
  `name_parser.rs`, `upload_info.rs`, `url_helpers.rs`
- [x] `modlight-core/src/components/`: `archive.rs`, `download.rs`, `install.rs`,
  `mod.rs`, `net.rs`, `process.rs`, `tool_release.rs`, `updates.rs`, `verify.rs`
- [x] `modlight-macos/src/`: `sound.rs`, `print.rs`, `ocr.rs`, `editor.rs`, `lib.rs`
- [x] `modlight-windows/src/`: `sound.rs`, `print.rs`, `ocr.rs`, `editor.rs`, `lib.rs`,
  `wide.rs`
- Leads searched in every crate of `app/crates`: slicing by bytes (`[..n]`, `[n..]`,
  `split_at`, `truncate`, computed indices on text), narrowing casts of outside numbers,
  full-size textures, egui defaults that act on every window.

## 2. Findings

| id | severity | file:line | what goes wrong | fix | test |
|---|---|---|---|---|---|
| UI-01 | high | `modlight-app/src/notification.rs:600` (`image_ui`) | The desktop notice over a picture uploaded the whole picture as one texture. The app's GPU device takes at most 8192 px a side (egui-wgpu's limit); a larger texture fails wgpu's validation, and wgpu's default error handler panics: the app dies. Scenario: "Show notification after task completed" is on by default; uploading a 61 MP photo (9504 x 6336), a 3-monitor capture or a long scrolling capture (up to 32768 px tall) ends the app when the notice opens. | The texture is the picture fitted to the card's size in pixels, capped at `max_texture_side` (`media::fitted_color_image`, new). Also less GPU memory. | `notification::tests::a_picture_larger_than_a_texture_still_shows` (failed before: egui's texture-size assertion) |
| UI-02 | medium | `modlight-core/src/config.rs:60` (`read_object`, `quick_verify_json`), `account.rs:192` (`AccountStore::load`), `backup.rs:207` (`import`) | A settings file with a byte order mark was read as broken. Newtonsoft (upstream) reads through a `StreamReader`, which takes a UTF-8, UTF-16 or UTF-32 mark off and decodes by it. On Windows (where the personal folder is upstream's `Documents\Modlight`) Notepad before 1903 and Windows PowerShell's `Set-Content -Encoding UTF8` write the UTF-8 mark, Windows PowerShell's `>` writes UTF-16: a hand-edited `ApplicationConfig.json` silently fell back to `Backup/` (or the defaults) and the next save wrote that over the person's file; `UploadersConfig.json` read as signed out with no host; a backup holding such a file failed to import; the save did not keep such a file as its backup copy. | `config::parse_object` decodes as `StreamReader` does (`names::helpers::decode_text`, now `pub(crate)`, the reader `%rf` already used), then reads the JSON; the loads, the save's check of the file it keeps, the backup import (its auto-run checks run on that same reading) and `AccountStore::load` use it. | `config::tests::a_settings_file_with_a_byte_order_mark_is_read` (failed before: English from the backup instead of Polish; also covers UTF-16 and the backup copy), `account::tests::an_account_file_with_a_byte_order_mark_is_read`, `backup::tests::a_settings_file_with_a_byte_order_mark_imports_with_its_checks` |
| UI-03 | low | `modlight-app/src/theme.rs:273` (`font_set`) | The icon family held Lucide alone, which has neither '◻' nor '?': epaint logged "Failed to find replacement characters '◻' or '?'. Will use empty glyph." at every start (the lead), and any code point Lucide lacks drew as nothing (zero width). | The text faces (Geist under Graphite, Inter, egui's own) sit behind Lucide in the icon family; Lucide stays first, so every icon and the row height are unchanged. | `theme::tests::a_character_the_icon_font_lacks_is_still_drawn` (failed before: width 0) |
| UI-04 | low | `modlight-app/src/app_pages.rs:697` (`sanitized_text`) | The path boxes of the Paths page (custom screenshots folder, sub folder patterns) stored `SanitizePath(text)`, which trims, and showed the stored value back on the next frame: a space typed at the end vanished at once, so "Application Support" or "%y-%mo Shots" could only be typed as "ApplicationSupport". Upstream's setter stores the cleaned text and leaves the box as typed. | While the box has the focus it shows the typed text (kept in egui's temp data, `settings_kit::edited_text`) as long as it cleans up to the stored value. | `app_pages::tests::a_path_box_keeps_a_space_while_it_is_typed` (failed before: "MyShots%y") |
| UI-05 | low | `modlight-app/src/task_pages.rs:2440` (Advanced page, `Kind::List`) | "Image extensions" and "Text extensions" are typed as one comma-separated line (upstream's `BoundValue` text box keeps what was typed and stores the split list). The port stored the split list at every key and showed it joined back, so a comma vanished the moment it was typed: a second extension could not be added by typing ("png, webp" became the one extension "pngwebp"). | The box keeps the typed text while focused as long as it splits to the stored list (`settings_kit::edited_text`, shared with UI-04). | `task_pages::tests::a_list_box_takes_a_second_item_as_it_is_typed` (failed before: `["pngwebp"]`) |
| UI-06 | low | `modlight-core/src/config.rs:335` (`theme_options`) | `ThemeOptions` was read as one struct: one member of the wrong type (`"UseSystemAccentColor": null`, `"BrandVersion": "one"`) made the whole block the defaults, so the skin, the light/dark choice and the accent were gone, and the next save wrote the defaults over them (the lead "one bad field resetting everything"). Upstream's `SettingsBase` error handler lets such a member keep its default and loads the rest. | `ThemeOptions::from_value` reads member by member (a blank skin is the default, as the setter has it). | `config::tests::a_bad_member_of_the_theme_options_costs_only_itself` (failed before: skin "graphite" instead of "nebula") |
| UI-07 | medium | `modlight-app/src/app.rs:1094` (`builder`), `media_windows.rs:19`, `app.rs:874` (drop window), `app.rs:925` (notice) | Every dialog was given its position in every frame: `Placement::owner(main)` from the main window's rectangle, `Placement::screen` from `visible_frame()` (on a Mac the screen of the KEY window). egui's `ViewportBuilder::patch` sends `OuterPosition` whenever that position changes, so the settings window (skins other than Graphite), the sign-in and paste windows followed the main window whenever it was moved, and on a Mac with two screens every centre-screen window (after upload, messages, text/URL upload, file exists, Components, the update windows) jumped to the middle of the screen it was dragged to; the notice and the drop window jumped to the corner of whatever screen got the key window. Avalonia's `CenterOwner`/`CenterScreen` place a window once, as it opens. | A window is placed only while it is not open yet (`app::is_open`: eframe lists every open window in the frame's input); the notice and the drop window keep the screen area they opened on. | `app::tests::a_dialog_is_placed_once_as_it_opens` (new API; the move itself needs real windows: reproduced by reading egui's `ViewportBuilder::patch`, which pushes `OuterPosition` on any change) |
| UI-08 | medium | `modlight-app/src/app.rs:116` (`ModlightApp::new`) | egui's own shortcuts, on by default, act in every window of the app (one context). `zoom_with_keyboard` zooms the whole interface on Cmd/Ctrl+Plus, Minus and 0: those are the app's keys (a pinned picture's opacity is Cmd+Plus/Minus, `tools/pin.rs:294`), so changing it also zoomed the main window, the settings and every dialog by 10% a press, and the main window's bounds were recorded in zoomed points (a smaller window at the next start). `quit_shortcuts` sends `Close` to the window with the key on Cmd/Ctrl+Q: on Windows, where no menu takes the key first, Ctrl+Q typed in the text upload or paste window closed it and dropped the text. Upstream has neither. | `app::prepare_context` turns both off before the first frame (a Mac's Quit stays the menu bar's). | `app::tests::egui_s_own_shortcuts_do_not_zoom_or_close` (failed before: zoom 1.1 after Cmd+=; with the zoom off alone, `[Close]` after Cmd+Q) |

### Checked and OK

- Slicing text by bytes, in every crate: no panicking site is left. Every `&s[a..b]` on
  text takes its bounds from `find`/`rfind`/`char_indices` or from ASCII-only checks
  (`names/*`, `l10n::format`, `update::parse_version`, `download/error.rs`,
  `tools/dotnet.rs`, `tools/color.rs`, `api/tus.rs`, `api/web.rs`); `truncate` and
  `split_at` work on vectors or after ASCII digit counts. Nothing to hand to other areas.
- Earlier fixes of the area still hold: S1-S5 (fallback load, weekly copies, window
  bounds and their clamp, reset), S7, S8, S10; N3 (`visible_in_desktop` measures against
  the primary screen) was fixed since; W1-W3 and A1 (backup import allow-list and
  auto-run checks) are unchanged and now also run on a file with a byte order mark.
- Components: pinned URLs and digests, size known before the download, a longer body
  refused, verify-or-delete before use, archive entries outside the folder skipped, no
  links or devices created, the size ceiling counted on what is written.
- Name parser, file helpers and URL helpers: UTF-16 lengths, char boundaries kept in
  `replace_all`/`for_each_between`, Windows root lengths on ASCII bytes only.

## 3. Not fixed

- Transparent windows render black (known, the brief says not to rewrite the renderer):
  the drop window and the desktop notice ask for `with_transparent(true)`, but eframe
  makes ONE wgpu painter for every window and takes its `support_transparent_backbuffer`
  from the root viewport only (`eframe-0.36.2/src/native/wgpu_integration.rs:228`), and
  the main window is not transparent, so every surface gets an opaque alpha mode. A fix
  needs the root viewport created with `with_transparent(true)` (then every surface asks
  for `PreMultiplied`/`PostMultiplied`), with the main window and every dialog painting an
  opaque background of their own (`clear_color` is transparent today, `app.rs`), checked
  on both systems for window shadows and compositor cost.
- Backup import and a save at the same moment (low): `import_backup` writes
  `ApplicationConfig.json`/`HotkeysConfig.json` on a blocking thread and the settings are
  reloaded only when its completion runs on the UI thread. A save in between (the main
  window's bounds are written a second after they stop changing, before the frame drains
  completions; a settings sync arriving in the same drain) writes the old settings over
  the imported file, and the reload then reads them back: the import is lost while its
  status line says it worked. The window is one frame wide. Fix: let the worker only read
  and check the archive (`backup::import` split into prepare and write) and write the
  files in the completion, on the UI thread, right before `reload_settings_files`.
- `config::save_settings_file` does not `fsync` the temporary file before the rename
  (upstream opens it `WriteThrough`). After a power cut the file can be empty; the load's
  fallback to `Backup/` then costs at most the last change. Left as it is.

## 4. Outside my area

- Full-size picture textures elsewhere (same crash as UI-01, owners of those files):
  `dialogs/after_upload.rs:415`, `dialogs/after_capture.rs:513`,
  `dialogs/before_upload.rs:291`, `dialogs/clipboard_upload.rs:122`, `scrolling.rs:242`
  load `image.width() x image.height()` as one texture. A picture over 8192 px a side
  (a large photo uploaded with the after-upload window, a long scrolling capture) crashes
  the app. `crate::media::fitted_color_image(image, max_w, max_h)` (added for UI-01) fits a
  picture to the size it is shown at, capped at `ctx.input(|i| i.max_texture_side)`.

- Tool windows placed every frame (same as UI-07, tools area): `tools/mod.rs:188`
  (`builder`) centres a tool window without a stored position on `visible_frame()` in
  every frame, so on a Mac a tool window dragged to another screen jumps to the middle of
  it. `crate::app::is_open(ctx, viewport)` (added for UI-07) tells whether to place it.

## 5. Checks

Run at the end (2026-11-12), with the whole diff in place:

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings` (macOS): clean.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (with `wincc/env.sh`): clean.
- `cargo test --workspace --no-fail-fast`: 45 suites, 990 passed, 4 ignored, 11 failed.
  The 11 are all of `modlight-api/tests/reference_server.rs`, every one on "Connection
  refused (os error 61)": the local reference server (loopback, port 8420) was down at
  that moment (nothing answered on 8420 or 8430), the environment and not this diff. The same
  suite passed in the full run of 2026-11-11 with the fixes up to UI-07 in place (that run:
  every suite green). `modlight-app` lib 150 passed, `modlight-core` lib 439 passed, the
  snapshot suite (`tests/snapshots.rs`, every screen in every look) passed: no snapshot
  changed, none updated.
- The new tests were run against the unfixed code first and failed (the reasons are in
  the table), except `app::tests::a_dialog_is_placed_once_as_it_opens` (UI-07), whose
  subject is a new function (the move it prevents needs real windows), and the account
  and backup tests of UI-02, written after the shared reader was fixed (serde_json
  refuses a byte order mark, which is what made `UploadersConfig.json` read as signed
  out and the import fail).
