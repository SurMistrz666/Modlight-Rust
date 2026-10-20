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
| Tray icon and menu (`TrayIconService`) | partial | Account, Capture, Upload, Tools, after-capture and after-upload tasks, the hotkey switch, Settings, screenshots folder, recent items, Show Modlight, Exit, built from the same entries as the window's menus; every Tools entry opens its tool (phase 5). Not yet checked in a real window. |
| Toasts in the window corner (`UploadToastViewModel`) | done | |
| My space tab | done | See below. |
| Upload tab (`UploadView`) | done | Transfers with progress, speed and time left; the offline queue with countdowns, Send now and Cancel; history from `History.db` and this session's tasks, with copy, open and Try again. Snapshots `tab-upload`, `tab-upload-busy`. |
| Download tab (`DownloadView`, yt-dlp) | done | Queue with pause/resume/cancel/retry/remove, format and destination strip, folder, ffmpeg and yt-dlp notices, error wording, update-and-retry, upload through the task flow. Not run against a real yt-dlp in a window yet. Snapshots `tab-download`, `tab-download-busy`, `tab-download-missing`, `download-youtube`. |
| YouTube sign-in (`YouTubeSignInForm`, `YouTubeCookies`) | partial | WKWebView window; never opened by automation; real-window check needed. |
| Capture tab (`IsCreateSelected`) | done | Tiles with their shortcuts, the capture menu, every capture kind (`capture_flow`). Snapshots `tab-create`, `tab-create-problem`, `capture-menu`. |
| Tools tab cards | done | Cards drawn from the same list as upstream; every card runs its tool (phase 5, see "ShareX tools" below); the Image effects card belongs to the effects branch. |
| Server tab (`AdminView`, disk report) | done | Read when opened (fresh for 2 min) and on Refresh; 404 settled through `/api/me`. Snapshots `tab-server`, `tab-server-problem`, `tab-server-loading`. |
| Hotkey problem notice (`HotkeyProblemViewModel`) | done | On the Capture tab, with "Change shortcut". Snapshot `tab-create-problem`. |

## My space (`Presentation/MySpace`)

| Upstream | State | Notes |
|---|---|---|
| Header and breadcrumbs, library panel, folder shelf, file grid, thumbnails, motion previews, type plates | done | |
| Panels: signed out, loading, empty, error; error footer; pager bar | done | |
| Details drawer (every section and link format) | done | |
| Context menus, keyboard, drag cards onto folders | done | Keys mapped as in `MACOS.md`. |
| Actions: move, publish, custom URL, link settings, trash, restore, delete for good, empty trash, new / rename / public / delete folder, download, download as (conversion check) | done | |
| Duplicates window | done | |
| Upload into a folder ("Upload here", folder tile button, Finder drop) | partial | Through the task flow and the offline queue (`TargetedSettings`). A Finder drop onto a folder *tile* uploads into the open folder instead: winit reports no pointer position during an OS drag. |

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
| Application: language and tray (with tray actions, updates card when a release host is built in), macOS integration, components, local folders, backup and cleanup, print, proxy, advanced | done (print's "Image print settings" waits for printing; proxy settings are stored, not applied) |
| Sharing: copy formats (with their editor), from clipboard, upload limits, history | done |
| Capture: region, screenshots (preconfigured region through the overlay), image format, effects, thumbnails, file names (previews) | done (effects configuration is phase 5) |
| Screen recording page and FFmpeg options window (`FFmpegOptionsWindow`) | done; snapshots `settings-recorder`, `ffmpeg-options*` |
| Text from image | done (Vision's languages; see "ShareX tools") |
| Sounds and notifications (with the notification buttons editor) | done |
| Tools: editor, colour picker, external programs (with the action editor), watch folders (with their editor) | done |
| Keyboard shortcuts (`HotkeySettingsView`) and one hotkey's settings window (`SettingsWindow(TaskSettings)`) | done; snapshots `settings-hotkeys*`, `hotkey-task-*` |
| Advanced: application options, capture options | done |

## Windows reached from the task flow

| Upstream | State |
|---|---|
| Desktop notice (`NotificationWindow`): tip | done |
| Desktop notice after a task, with its image, buttons and click actions | done (snapshot `notification-upload`) |
| Before upload, after upload, large file warning, multi-upload confirmation, file exists, message box | done (snapshots) |
| Drag and drop upload, clipboard upload, paste (text with language), text upload, shorten URL, URL upload | done (snapshots) |
| After capture, auto capture, image editor selector, quick task menu and its editor | done (snapshots `after-capture*`, `auto-capture`, `editor-selector`, `quick-task-*`) |
| First run (`FirstTimeConfig`), about, debug log, components | done (snapshots `first-run`, `about`, `debug-log*`, `components`) |
| Region capture overlay, recording frame and toolbar, scrolling capture window | done in drawing and behaviour with the fake desktop; real-window checks listed in `docs/handoff/app-3.md` (snapshots `region-capture*`, `recording-toolbar*`, `scrolling-capture`) |
| History and image history windows (`ShareX.HistoryLib` views) | done (phase 5; see "ShareX tools") |
| Image editor | done in drawing and behaviour headlessly; see "Image editor" below |
| ShareX tools | done (phase 5; see "ShareX tools"); the image effects window is the effects branch's |

## ShareX tools (phase 5: crate `modlight-tools`, windows in `modlight-app/src/tools/`)

Each tool runs from its hotkey, the tray's Tools menu and the Tools tab (`tools::jobs::run`),
and from the after-capture tasks and the history windows where upstream reaches it. Real-window
checks and the permissions they need are in `docs/handoff/app-5-tools.md`.

| Upstream | State | Notes |
|---|---|---|
| Colour picker (`ColorPickerWindow`, `ColorPickerViewModel`) | done | Recent colours, standard palette, formats, copy; also the settings' and editor's colour dialogs' "pick from screen". Snapshots `color-picker*`. |
| Screen colour picker (`ScreenColorPickerWindow`) | done | Frozen desktop (ScreenCaptureKit), magnifier, info text, copy formats; the editor's "Pick colour from screen". Snapshot `screen-color-picker`. |
| Pin to screen (`PinToScreenWindow`, startup, options, close all) | done | From the screen, clipboard, file, a task, the editor and the notices; zoom, opacity, move, toolbar, minimise. Snapshots `pin-*`, `pinned-image`. |
| Image viewer (`ImageViewerWindow`) | done | Full-screen, folder navigation, the history's neighbours. Snapshot `image-viewer`. |
| Input box, output box, error window | done | The output box shows a failed recording's ffmpeg output; the error window catches panics off the UI thread. Snapshots `input-box`, `output-box`, `error-window`. |
| Print (`PrintWindow`, `PrintHelper`) | done | `NSPrintOperation`; preview as a PDF in Preview. Snapshots `print`, `print-settings`. |
| Image combiner | done | Snapshots `image-combiner*`. |
| Video converter | done | ffmpeg from Components; tested with Homebrew's ffmpeg (`tests/tools_video.rs`). Snapshots `video-converter*`. |
| Metadata viewer, strip metadata | done | ExifTool (Perl) from Components; tested with a stand-in script. Snapshots `metadata*`. |
| Directory indexer (HTML, text, XML, JSON) and the clipboard upload's folder indexing | done | Output tests per format. |
| QR code (generate, decode a picture, scan the screen or a region) | done | rxing (ZXing); round-trip tests. Snapshots `qr-code*`. |
| OCR (`OCRWindow`, silent mode, the after-capture task) | done | Apple Vision. Snapshots `ocr*`. |
| Analyze image (`AnalyzeImageWindow`, options) | done | OpenAI (Responses and legacy Chat Completions), Gemini, OpenRouter; requests tested against a stub on 127.0.0.1. Not in the menus, as upstream. Snapshots `analyze-image*`. |
| Background remover | done | ONNX Runtime with Core ML or the CPU (`modlight_core::background_removal`, also behind `remove_background`); tested with a hand-written ONNX model. Snapshots `background-remover*`. |
| Active window borderless | done (closest macOS behaviour) | Fills the screen through Accessibility; see `MACOS.md`. |
| Active window top-most | not possible | macOS gives no public way; the hotkey says so (`MACOS.md`). |
| History, image history | done | On `History.db`. Snapshots `history*`, `image-history*`. |
| Toggle tray menu (`HotkeyType.ToggleTrayMenu`) | not done | Says "later". |

## Image editor (`ShareX.ImageEditor`, crate `modlight-editor`)

The map of the crate and the effect registry contract are in `EDITOR.md`. Snapshots:
`crates/modlight-editor/tests/snapshots/<scene>/` (30 scenes) and the app's
`image-editor*` screens.

| Upstream | State | Notes |
|---|---|---|
| `EditorCore`, history (mementos, canvas mementos, crop/cut-out, flatten, z-order), clipboard, image insert, new image | done | Unit tests in the crate; ids of pasted and duplicated annotations are new (upstream kept the copied id). |
| Annotations: rectangle, ellipse, line, arrow (5 styles, curves), freehand, text (outlined), speech balloon, step (5 types, tail), image, emoji, cursor, smart eraser, blur, pixelate, highlight, magnify, spotlight, crop, cut-out | done | Rendered with tiny-skia (`examples/gallery.rs` draws each); handles, snapping, Shift and Cmd modifiers, rotation, tails, curve points. |
| Editor window (`EditorWindow`, `EditorView`, built-in toolbars) | done | Canvas with zoom levels, Cmd+wheel, pinch, middle-button pan, scroll; overlay chrome; text boxes; context menu; toast; options persisted in `ToolsSettings.ImageEditorOptions`. Real-window checks in `docs/handoff/app-4.md`. |
| Annotation toolbar, tool options row and every picker (colour with `ColorView`, width, border style, corner radius, font family, alignment, arrow style, cursor type, font size, bold, italic, step type, start number, strength, spotlight blur, ellipse, tail, shadow with its options) | done | `ColorView` is an approximation (see `MACOS.md`). |
| Background panel (margin, padding, smart padding, corner, shadow, ratio, gradient presets, colour, image, wallpaper) | done | Wallpaper through `NSWorkspace`. |
| Effect browser, schema-driven dialog, preview | done | Registry with the editor operations and three sample effects; the effect library plugs in (`EDITOR.md`). |
| Dialogs: start screen, new image, insert image, emoji picker, exit confirmation, toolbar customisation | done | |
| Integration: Image editor hotkey and selector, after-capture "Annotate image" (task mode), "Beautify image" (Background panel open), the notice's "Annotate image", save, copy, upload, open, URL, clipboard, print, pin to screen, the screen colour picker | done | Print, pin and the screen colour picker are phase 5's tools. |
| Region capture's annotation tools (`EditorView` as the overlay's workspace) | done | The editor under the overlay (one canvas per display), the tool strip beside the region tool with the options row, keys to the editor first; the annotations go with the captured picture. Snapshot `region-capture-annotate`. |

## Engine

| Upstream | State |
|---|---|
| Name patterns (`NameParser`), after-upload formats (`UploadInfoParser`), `FileHelpers` / `URLHelpers` pieces | done |
| Live upload (`Modlight.Upload`: one streamed multipart `POST /upload`) and its refusal sentences | done |
| Duplicate check before uploading (`POST /api/uploads/check`) | not in upstream's desktop app (only the web panel calls it), so not ported |
| API client: every endpoint the desktop app uses, refusal handling, timeouts | done |
| Task flow (`TaskManager`, `WorkerTask`, `UploadManager`, `UploadInfoManager`) | done for uploads (file, folder, clipboard, text, paste, URL download, shorten, share); capture jobs phase 3. Tested against the reference server (`modlight-core/tests/task_flow.rs`). |
| Offline queue (`ModlightUploadQueue`, `ModlightQueueStore`, `Queue.json`) and resumable uploads (tus) | done; resume after a cut and after a restart tested against the reference server |
| History store (`History.db`) | done (same table and value shapes as the Windows app) |
| Watch folders | done (FSEvents; see `MACOS.md`) |
| Captures (fullscreen, monitor, window, region, last/custom region and window, auto, scrolling), recording (video and GIF through ffmpeg, two-pass, pause), hotkeys, components with pinned SHA-256, yt-dlp | done; captures tested from the fake desktop through upload to the reference server and back (`modlight-app/tests/capture_upload.rs`), recording against Homebrew's ffmpeg (`modlight-core/tests/recording.rs`) |

## Packaging

| Upstream | State |
|---|---|
| Installer | done as `scripts/build-app.sh` (`MACOS.md`) |
| Update check (`ModlightUpdateChecker`, fixed host) | done, off unless `MODLIGHT_DEFAULT_HOST` is set at build time; `.dmg`/`.zip` only, revealed in Finder |
| Settings backup (`SettingManager.Export/Import/ResetSettings`) | done (`modlight_core::backup`) |
