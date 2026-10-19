# Port status, per upstream screen and feature

Legend: **done** = ported 1:1 and covered by tests or snapshots; **partial** = there,
with the gaps named; **phase N** = not started, belongs to that phase of `PORT.md`.
Substitutions for Windows-only mechanisms are in `MACOS.md`.

Snapshots of every screen below that is drawn live in
`crates/modlight-app/tests/snapshots/<screen>/{dark,light}-{en,pl}.png`
(`cargo test -p modlight-app --test snapshots`; `UPDATE_SNAPSHOTS=1` to refresh).

## Main window (`Presentation/MainWindow`)

| Upstream | State | Notes |
|---|---|---|
| Shell: title band, tab strip, storage chip, account button and menu, settings menu | done | |
| Tray icon and menu (`TrayIconService`) | partial | Sign in / Signed in as (account submenu), Settings, Show Modlight, Exit. Capture and tool entries come with phase 3. Not yet checked in a real window. |
| Toasts in the window corner (`UploadToastViewModel`) | done | |
| My space tab | done | See below. |
| Upload tab (`UploadView`) | phase 2 | |
| Download tab (`DownloadView`, yt-dlp) | phase 3 | The tab shows its plan card. |
| Capture tab tiles | partial | Tiles drawn; the captures are phase 3. |
| Tools tab cards | partial | Cards drawn from the same list as upstream; the tools are phases 3 to 5. |
| Server tab (`AdminView`, disk report) | phase 3 | The tab shows its plan card; the API call is ported (`get_disk_report`). |
| Hotkey problem banner (`HotkeyProblemViewModel`) | phase 3 | |

## My space (`Presentation/MySpace`)

| Upstream | State | Notes |
|---|---|---|
| Header and breadcrumbs, library panel, folder shelf, file grid, thumbnails, motion previews, type plates | done | |
| Panels: signed out, loading, empty, error; error footer; pager bar | done | |
| Details drawer (every section and link format) | done | |
| Context menus, keyboard, drag cards onto folders | done | Keys mapped as in `MACOS.md`. |
| Actions: move, publish, custom URL, link settings, trash, restore, delete for good, empty trash, new / rename / public / delete folder, download, download as (conversion check) | done | |
| Duplicates window | done | |
| Upload into a folder ("Upload here", folder tile button, Finder drop) | partial | Phase 1 sends one plain `POST /upload` per file; phase 2 routes these through the task flow and the offline queue. |

## Sign-in and account

| Upstream | State | Notes |
|---|---|---|
| Sign-in window (browser loopback, manual token) | done | Loopback hardened as `MACOS.md` says. |
| `/api/me`, admin flag, account adoption | done | |
| Settings sync (`/api/client-settings`) and skin sync (`/api/theme`) | done | Document format shared with Windows installs. |

## Settings window (`Presentation/Settings`)

| Page | State |
|---|---|
| Navigation, search box, every page's header | done |
| Account: profile, links, storage | done |
| Appearance: theme and colours | done |
| Application: language and tray | done |
| Sharing: copy formats, from clipboard, upload limits, history | phase 2 |
| Tools: watch folders | phase 2 |
| Capture, recording, text from image, file names, shortcuts, sounds, editor, external programs, main window tiles, integration, components, local folders, backup, print, proxy, advanced | phase 3 |

## Windows reached from the task flow

| Upstream | State |
|---|---|
| Desktop notice (`NotificationWindow`): tip | done |
| Desktop notice after a task, with its image, buttons and click actions | phase 2 |
| Before upload, after upload, large file warning, multi-upload confirmation, file exists | phase 2 |
| Drag and drop upload, clipboard upload, paste (text with language), shorten URL, URL upload | phase 2 |
| After capture, auto capture, image editor selector, quick task menu editor | phase 3 |
| First run (`FirstTimeConfig`), about, debug log, components | phase 3 |
| History and image history windows (`ShareX.HistoryLib` views) | phase 3 |
| Image editor | phase 4 |
| Image effects and the other ShareX tools | phase 5 |

## Engine

| Upstream | State |
|---|---|
| API client: every endpoint the desktop app uses, refusal handling, timeouts | done |
| Task flow (`TaskManager`, `WorkerTask`, `UploadManager`, `UploadInfoManager`) | phase 2 |
| Offline queue (`ModlightUploadQueue`, `ModlightQueueStore`, `Queue.json`) and resumable uploads (tus) | phase 2 |
| History store (`History.db`) | phase 2 |
| Watch folders | phase 2 |
| Captures, recording, hotkeys, components with pinned SHA-256, yt-dlp | phase 3 |

## Packaging

| Upstream | State |
|---|---|
| Installer | done as `scripts/build-app.sh` (`MACOS.md`) |
| Update check (`ModlightUpdateChecker`, fixed host) | phase 3 |
