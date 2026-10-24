# Logic review 2026-10-24, area 09: settings and persistence

Branch `logic-09-settings` (from main `28916b4`). Scope: `ApplicationConfig.json` /
`HotkeysConfig.json` persistence, the settings windows (application, task and theme pages),
first run, backups, name patterns, translations, About and the debug log, app state and
runtime glue, the login item and Finder service. Specification: upstream
`ShareX/{SettingManager, ApplicationConfig, TaskSettings}.cs`, `ShareX.HelpersLib/Settings/SettingsBase.cs`,
`NameParser`, `Presentation/{ApplicationSettings, TaskSettings, MainWindow, FirstTimeConfig}`.

Every fixed finding has a test that failed before the fix (run against the unfixed code
first), except where the column says it was reproduced by reading (a missing caller cannot
be called by a test).

## Findings

| id | severity | file:line (at `28916b4`) | what is wrong | how it was reproduced | status |
|---|---|---|---|---|---|
| S1 | High | `modlight-core/src/config.rs:55-84` (`read_object`, `AppConfig::load`) | A settings file that does not parse (empty after a crash, cut short, edited wrong, `null`, an array) was read as an empty document and the next save wrote the defaults over it: every setting lost. Upstream `SettingsBase.Load` falls back to `<file>.temp`, `Backup/<file>` and the newest weekly copy; its saves (`CreateBackup`, `CreateWeeklyBackup`) keep the file they replace and a weekly copy. The port did neither, for either file. | `config::tests::a_broken_settings_file_falls_back_to_what_the_saves_left` and `a_save_keeps_the_file_it_replaces_and_a_weekly_copy` failed on the old code | fixed, `535a9a3` (+ `the_weekly_copy_is_named_like_upstream_names_it`) |
| S2 | Medium | `modlight-app/src/main.rs:240-253` | "Automatically cleanup old backup files" did nothing (only the logs were cleaned at start). With S1 the weekly copies exist; `CleanupManager.Cleanup` thins `ApplicationConfig-*.json`, `HotkeysConfig-*.json` (and the Windows-only names) to `CleanupKeepFileCount`. | by reading (no code read the member) | fixed, `535a9a3`, test `debug_log::tests::the_start_cleans_the_weekly_backups_when_asked` |
| S3 | Medium | `modlight-core/src/config.rs:289` (`set_main_window`), `main.rs:270,303-313` | "Remember main window position" / "... size" (on by default) did nothing: `MainFormSize`, `MainFormPosition`, `MainFormMaximized` were read at start but nothing ever wrote them (`set_main_window` had no caller). The restore also had no `RestoreWindowBounds` clamp, so once saving works a place remembered on an unplugged display would open the window off screen. | by reading (grep: no caller); new tests `config::tests::the_main_window_is_recorded_as_upstream_records_it`, `a_remembered_window_opens_on_a_screen_that_exists`, `app::tests::a_frame_reports_the_main_window_as_upstream_reads_it` | fixed, `8355aa0`, `bac892d` (screens asked for only when needed; `NSScreen.screens` checked to answer before the event loop starts) |
| S4 | Medium | `modlight-app/src/app_pages.rs:1999-2003, 2307` | Reset: upstream loads without fallback support and saves all three files. With S1's fallback the reset reloaded `Backup/ApplicationConfig.json` (what had just been reset), and `HotkeysConfig.json` was never rewritten, so the next start brought the old hotkeys back from the backup. Found by the test written for S1's interplay; without S1 it was harmless. | `app_pages::tests::a_reset_does_not_come_back_from_the_backup` failed | fixed, `5590c57` |
| S5 | Low | `modlight-app/src/app_pages.rs:1999-2003` + `state.rs:302` | After an import or a reset `save_config` wrote the page size My space was showing over `ModlightPageSize`: a backup's page size was lost, a reset kept the old one. | `app_pages::tests::an_import_keeps_the_page_size_the_backup_brought` failed | fixed, `5590c57` |
| S6 | Medium | `modlight-core/src/tasks/naming.rs:28-32`, `modlight-app/src/task_pages.rs:1696-1698, 1820-1840` | "Use custom time zone" always named files in UTC: the stored `UploadSettings.CustomTimeZone` (a `TimeZoneInfo` as Newtonsoft writes it) was never read, so a zone chosen on Windows (synced) named files hours off. MACOS.md promises UTC for UTC and local time for other zones, which `NameParser` already models. The preview did the same, and the zone list showed "UTC" whatever was stored while its choice wrote nothing. | `tasks::naming::tests::the_custom_time_zone_is_the_one_the_settings_name` and `task_pages::tests::the_naming_preview_uses_the_zone_the_settings_name` failed | fixed, `d32f6a6` |
| S7 | Medium | `modlight-app/src/task_pages.rs:2075` | Crash: the Actions page draws a row for every `ExternalPrograms` entry; ticking the check box of an entry that is not an object (`"ExternalPrograms": ["oops"]`, a damaged or hand-edited file) ran `list[index]["IsActive"] = ...`, which serde_json answers with a panic. | standalone reproduction of the expression (panic "cannot access key \"IsActive\" in JSON string"); test `task_pages::tests::a_broken_action_entry_does_not_crash_its_check_box` | fixed, `3dd6dc2` |
| S8 | Medium | `modlight-app/src/settings.rs:1907-1925` | The accent colour could not be changed: upstream binds `AccentColor` to a `ColorPickerDropdown`; the port drew a swatch and the hex that nothing could edit (`Rgba::to_hex_argb`, meant for the setter, had no caller). | by reading; test `settings::tests::a_picked_accent_colour_is_stored_like_upstream_stores_it`, snapshot `settings-theme` updated | fixed, `2c9272e` |
| S9 | Medium | `modlight-core/src/image_decode.rs:57-61` | `image_decode::open` chose the decoder from the extension; `ImageHelpers.LoadImage` (`Image.FromStream`) decodes what the bytes are. A PNG named `.jpg` did not open: no thumbnail in the notice and upload windows, no picture for "process images during file upload", the editor's background image. | `image_decode::tests::a_file_decodes_by_its_content_not_its_extension` failed ("Illegal start bytes: 8950") | fixed, `3a7c8c4` |
| S10 | Low | `modlight-app/src/app_pages.rs:2222` | An exported backup had no `HotkeysConfig.json` when the hotkeys were never changed (the port writes that file only on a change; upstream's `ExportAsync` runs `SaveAllSettings` first), so restoring it elsewhere kept that installation's hotkeys instead of the backup's. | `app_pages::tests::an_export_carries_the_hotkeys_even_when_never_changed` failed | fixed, `3f364ba` |
| S11 | Low | `app/docs/MACOS.md`; `app_pages.rs` Advanced paths | `UseMachineSpecificUploadersConfig`, `CustomUploadersConfigPath`, `CustomHotkeysConfigPath` are stored and not applied, without a documented reason. Not implemented on purpose: there is no `UploadersConfig.json` on a Mac, and a hotkeys file read from a path the settings name would bypass the backup import's auto-run checks (a backup could point it at a file that starts programs). | by reading | documented in MACOS.md, `3a7c8c4` |
| N1 | Low | `modlight-app/src/capture_flow.rs` (`pump_hotkeys`), `modlight-core/src/hotkeys.rs` | `HotkeyRepeatLimit` (Advanced, default 500 ms, min 200) is never read: upstream's `HotkeyForm.CheckRepeatLimitTime` drops a hotkey press that follows the last accepted one within the limit. | by reading (no reader besides the page and the sync list) | not fixed: the hotkeys area's code (capture_flow.rs / hotkeys.rs); flagged for that area |
| N2 | Low | `modlight-tools/src/images.rs` (`load`) | `RotateImageByExifOrientationData` (Advanced) is ignored: the tools' loader always turns pictures by their EXIF orientation; upstream turns them only when the setting is on. | by reading | not fixed: tools area |
| N3 | Low | `modlight-macos/src/windows.rs:32-47` (`visible_frame`) | The y of the visible frame is computed against `mainScreen`'s own height, not the primary screen's: when the key window is on a secondary display with another height or offset, CenterScreen dialogs, the desktop notice and the drop window land off by the difference. The fix is `top = primary_height - (visible.y + visible.height)` with `screens[0]`, as `visible_frames` (added for S3) does. | by reading (coordinates) | not fixed: notice/dialog placement belongs to another area; one-line fix described |
| N4 | Low | `modlight-core/src/theme.rs` (`ThemeOptions::default`), no `MigrateToastButtons` / `MigrateWindowMemory` | Upstream's one-time migrations (`MigrateBrand`, `MigrateToastButtons`, `MigrateWindowMemory`) are not ported and `BrandVersion` defaults to 1 instead of 0. They only change a configuration that never went through a Modlight build (ShareX's), which on a Mac arrives only through a ShareX backup import; MACOS.md lists ShareX import as not applicable. | by reading | not fixed (no Modlight-made file is affected) |

## Checked and OK

- Translations: every `.resx` copy under `assets/upstream/l10n` is byte for byte upstream's;
  `Mac.resx`/`Windows.resx` have the same keys in English and Polish, none empty. No key used
  in code is missing or empty in Polish (3102 uses of 1923 keys, module aliases resolved per
  file). Keys are compile-time constants, so none can be missing in English. This is now a
  test, `modlight-core/tests/l10n_coverage.rs` (`5673245`): build.rs records which keys the
  Polish table really carries (`l10n::has_polish`), since a missing one falls back to English
  silently; taking one key out of `Mac.pl.resx` makes the test name it and its files. `enum_text`'s lookups: every enum it is used with has its keys in
  HelpersLib's table only, as upstream's `GetLocalizedDescription` reads them.
- `NameParser` against upstream: code order (`%t` first, `%mon2`/`%yy`/`%width`/`%unix`
  before their prefixes, `{n}` forms first), counters (`%i`, `%ia`, `%iAa`, `%ib`, `%ix`,
  wrap-around, negative precisions), random codes, `%guid`/`%GUID`, `%rf` (preview mode,
  `AllowFileRead`), sanitising by type, `MaxNameLength`/`MaxTitleLength` in UTF-16 units,
  `HourTo12`, `WeekOfYear` (first-day rule, Monday), month and day names in en-US/pl-PL.
  `GetFileName`'s choice between the window and the plain pattern and the counter write-back
  match `TaskHelpers.GetFileName`.
- Defaults of `config.rs` accessors, `AppUploadSettings`, the task settings groups and every
  application page control (paths, ranges, defaults, enum lists) against `ApplicationConfig.cs`,
  `TaskSettings.cs`, `RegionCaptureOptions.cs`, `ApplicationSettingsView.axaml` and
  `TaskSettingsPageBuilder.cs`; `SanitizePath` on the path boxes; the buffer size list
  (2^n KB in the chosen units); `ToSizeString` and `CloudSize`; `string.Format` placeholders
  used by the tables (`{0:N0}`, `{0:0.0}`).
- Unknown members survive: the settings document is kept as JSON, pages edit by path, the
  editors (actions, watch folders, copy formats) keep unknown members of the entry they edit;
  `ThemeOptions` holds every member upstream serialises.
- Backup: export name (`Modlight-{version}-{machine}-backup.sxb`), import allow-list and
  auto-run checks (W1-W3, A1) unchanged; `History.db` in a backup is passed over on import as
  upstream does, so copying it while the history is open cannot reach the app.
- Save timing: every change is saved at once (atomic rename); `on_exit`, the close button and
  the menu bar's Exit (which saves as settings change) keep the settings; window bounds are
  written a second after they stop changing.
- First run (opens on the first launch of the bundle, login item and Finder service note),
  the image editor selector, About links, Debug log line format and monthly file, the login
  item states and the Finder service provider, the runtime's completion channel, the language
  switch (applies at once; the tray menu rebuilds from its labels).

## Changes outside the area

- `modlight-core/src/tasks/settings.rs` (`UploadSettings.custom_time_zone`) and
  `modlight-core/src/tasks/naming.rs` (S6): the zone the settings hold reaches `GetFileName`.
- `modlight-macos/src/windows.rs`, `modlight-macos/src/system.rs`: `visible_frames()` (S3).
- `modlight-windows/src/windows.rs`, `modlight-windows/src/system.rs`: `visible_frames()`
  (S3), the same expressions as its `visible_frame()`. Not compiled here (cross-checking for
  `x86_64-pc-windows-msvc` stops at SQLite's C build without the Windows SDK); to build on the PC.
- `modlight-core/src/image_decode.rs` is in the area list; its callers in the editor, the
  tools and the effects were tested (`cargo test -p modlight-tools -p modlight-editor -p modlight-effects --lib`).

## Verification

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test -p modlight-core -p modlight-macos -p modlight-app`: all pass, including the
  reference-server tests (`task_flow`, `reference_app`, `capture_upload`, the server on 8420
  was up) and every snapshot (only `settings-theme` changed, by S8).
- `cargo test -p modlight-tools -p modlight-editor -p modlight-effects --lib` (callers of
  `image_decode::open`, S9): pass.
- Not checked in a real window: the window bounds (S3) and the accent colour dropdown (S8)
  were tested headlessly only; no window was opened on this Mac.
