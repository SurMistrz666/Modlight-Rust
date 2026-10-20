# Windows to macOS: what the port substitutes, and why

Upstream (`upstream/Modlight-App`) is a Windows program. Where Windows has a mechanism
macOS lacks, the port uses the closest macOS mechanism and records the swap here.
Everything not listed behaves as upstream does. The per-screen state is in
`STATUS.md`; the working rules in `PORT.md`.

## Platform services

| Windows (upstream) | macOS (this port) | Notes |
|---|---|---|
| Token encrypted with DPAPI inside `UploadersConfig.json` | Login Keychain, generic password, service `Modlight`, written and read through `/usr/bin/security` | An item created by the app binary itself would be tied to that build and every rebuild would prompt; `security` is a stable owner. QA runs (`--qa-profile`) keep the token in a plain file in the profile instead and never touch the Keychain. The item holds the token together with the origin it was issued for: with the host in one place and the token in another, a host changed in `UploadersConfig.json` (an edited file, somebody else's backup) would otherwise receive the token. A host that is not that origin reads as signed out until the person signs in there; a token saved without its origin (an earlier build) asks for one new sign-in. |
| Personal folder `Documents\Modlight` (`Program.PersonalFolder`) | `~/Library/Application Support/Modlight` | Same file names inside: `ApplicationConfig.json`, `Queue.json`, `Queue/`, `History.db`. |
| Notification area icon and its menu (`TrayIconService`) | Menu bar extra (`tray-icon` + `muda`) | Normal mode only; never in QA mode. Exit from it ends the process directly, because a hidden window may never be redrawn to receive a close. |
| Window chrome: custom title bar with caption buttons | Native traffic lights over a full-size content view | The logo and title start at x = 78 to clear the lights; the band drags and double-click zooms the window. |
| `Ctrl` shortcuts | `Cmd` | `Cmd` stands for `Ctrl` everywhere. Up a folder: Backspace, Alt+Left or Cmd+Up. Trash: Delete (Fn+Backspace) or Cmd+Backspace. Delete for good: Shift with those, Cmd+Option+Backspace, or the same keys in the trash view. |
| Explorer drag and drop onto the window | Finder drag and drop (winit's dropped files) | Same overlay, same targets (the open folder, a folder tile). |
| `Process.Start(url)` | `/usr/bin/open` with http and https links only | Anything else is refused rather than handed to the system. |
| Region format (`CurrentCulture`) | `AppleLocale` region | en-US and pl-PL are modelled exactly as .NET formats them; other regions format like en-US. Interface language from `AppleLanguages`. |
| "Reduce motion" (`SystemParameters`) | `com.apple.universalaccess reduceMotion` | Read once at start. |
| Desktop notice placement (`NotificationWindow`, screen working area) | The visible frame of the main screen (menu bar and Dock excluded) | |
| Explorer "Show in folder" | `open -R` (reveal in Finder) | Opening a file or folder is `open -- <path>`. |
| `NetworkChange` events + `NetworkInterface.GetIsNetworkAvailable` | `getifaddrs`: an interface other than loopback that is up, running and has an address, read before every pass of the queue (at least every 15 s) | Nothing is sent to find out. There is no change event: the queue's 15 s poll and its wake-ups stand in for `NetworkAvailabilityChanged`. |
| `FileSystemWatcher` (watch folders) | FSEvents through the `notify` crate | Same filter rules, 1 s duplicate timer and "wait until the size stops changing" timing. FSEvents repeats flags and does not pair renames, so files are tracked by device, inode and birth time; a rename inside one folder is ignored, a move between folders or from outside is reported (as Windows' Deleted + Created). Folders and symlinks are never reported. On SMB volumes only this Mac's own changes are seen. A watched folder's "move to the screenshots folder" takes a unique name when the name exists instead of asking (nobody is at the window). |
| Clipboard (`ClipboardHelpers`) | NSPasteboard through `arboard` | Text, images and Finder file lists. Reading asks for the file list first: Finder also puts the file's icon and name on the pasteboard, which upstream's image-then-text order would upload instead of the file. A picture is read as its PNG or TIFF bytes (PNG first, as the pasteboard holds it) and decoded under the app's limits, not by arboard. QA and headless runs never read the real clipboard (macOS may ask for paste permission). |
| Taskbar progress (`TaskbarManager`) | Not shown | macOS has no per-window taskbar progress; the tray icon's progress and the Upload tab carry it. The setting's check box is shown disabled, as upstream shows it where the platform does not support it. |
| Inno Setup installer | `scripts/build-app.sh`: release build, `Modlight.app` with `Info.plist`, `.icns` cut from `ShareX/Modlight_Icon.ico`, ad hoc signature | Bundle id `org.modlight.Modlight` (to confirm with the owner). The script never starts the app. A DMG and components downloaded from the Components window come with phase 3. |
| Start with Windows (`StartupManager`) | Login item through `SMAppService.mainApp` | Only a copy inside an app bundle can register; the check box says why it cannot change (turned off in System Settings, waiting for approval, not an app bundle) and offers System Settings' Login Items. |
| Explorer context menu "Upload with Modlight" and "Send to" | Finder Services: `NSServices` in `Info.plist` (`uploadFiles`, any file or folder), provider installed at start | macOS lists the service by itself; it cannot be switched on or off from the app, so the integration page and the first-run window explain where it is and open the Services shortcuts. Files arrive through `uploadFiles:userData:error:` and go through the task flow. The Polish name is upstream's "Prześlij za pomocą Modlight" (`ServicesMenu.strings`). |
| Print Screen and global hotkeys (`RegisterHotKey`) | `global-hotkey` (Carbon `RegisterEventHotKey`, no permission) | Mapping: `Control` = Cmd, `Alt` = Option, `Win` = Control, `Shift` = Shift. Print Screen, Scroll Lock and Pause are F13, F14 and F15 (their place on Apple keyboards) and are labelled so; typing F13 stores Print Screen, so a binding means the same key on Windows. Defaults avoid the system's Cmd+Shift+3/4/5/6: region Cmd+Shift+2, full screen Cmd+Shift+1, active window Cmd+Shift+7, recording Cmd+Shift+8, GIF Cmd+Shift+9. Registered in the real app only. |
| Typing a hotkey on the shortcuts page (`IgnoreHotkeys` while capturing) | Every hotkey is unregistered while one is typed, and registered again after | A combination the system already delivers to this app as a hotkey would never arrive as a key press. |
| `DisableHotkeysOnFullscreen` | The active app's presentation options include full screen | |
| GDI / DXGI capture (`Screenshot`, `CaptureHelpers`) | ScreenCaptureKit (`SCScreenshotManager`) for pictures; `CGWindowListCopyWindowInfo` for windows; CoreGraphics for displays and the pointer | Coordinates are CoreGraphics global points (top left of the main display); a picture is taken at the highest scale of the displays it covers. Needs Screen Recording permission; without it captures report "not permitted" instead of a black picture. |
| Region capture forms (`RegionCaptureWindow`, `RegionCaptureLightForm`) | One borderless always-on-top viewport per display, raised above the menu bar and full-screen spaces (`NSWindow` level 1000, all spaces) | Same selection state machine (window and control detection, magnifier, crosshair, keyboard, multi-monitor, info text). Annotation tools from the image editor, drawn on the frozen desktop. |
| Scrolling capture (`ScrollingCaptureManager`, `InputHelpers.SendMouseWheel`) | Posted CoreGraphics scroll and key events | macOS delivers posted events only for apps trusted under Accessibility; the stitcher is upstream's. |
| Screen recorder input (`gdigrab`, `ddagrab`, dshow) | ScreenCaptureKit (`SCStream`, own app excluded) frames piped to ffmpeg's standard input (`-f rawvideo -pix_fmt bgra`), cameras and microphones through ffmpeg's avfoundation | Stop closes standard input (ffmpeg prints no "Press [q]" for a pipe, so the first frame written marks the recording as started); pause records segments joined with concat, as upstream. The screen source is stored as upstream's `gdigrab`, and `ddagrab`, `screen-capture-recorder` and the old "GDI grab" all mean the screen, so a document synced to Windows records the screen there too. |
| FFmpeg options window (`FFmpegOptionsWindow`) | Same window | Sources list the screen and avfoundation devices ("DirectShow" in the status line reads "AVFoundation", from `Mac.resx`); no "Get recorder devices" (a DirectShow filter). NVENC, AMF and Quick Sync are not offered; a synced value naming one encodes with VideoToolbox and keeps only its bitrate. The custom command template starts after the pipe input, which a custom command always gets first. |
| `TaskHelpers.CheckFFmpeg` | Same: the components window offers FFmpeg and the recording goes on when it arrives | |
| WebView2 (YouTube sign-in) | WKWebView | See the components rows below. |
| `DebugHelper.Logger` and `Logs\Modlight-Log-yyyy-MM.txt` | The same line format in `Logs/Modlight-Log-yyyy-MM.txt` of the personal folder; the app's crates from Info, others from Warn | Upstream's main menu no longer reaches the Debug log window; the port links it from About. "Loaded assemblies" is not ported (there are none). |
| Installer's first-time configuration (`FirstTimeConfigWindow`) | Opens on the first launch of the app bundle (no settings file yet) | Its Windows options become the login item and the note on the Finder service. |
| Settings backup (`.sxb`: ApplicationConfig, UploadersConfig, HotkeysConfig, History) | ApplicationConfig.json, HotkeysConfig.json and History.db | There is no UploadersConfig.json on a Mac (the token stays in the Keychain). Import restores exactly `ApplicationConfig.json` and `HotkeysConfig.json`, by name; `History.db` is passed over as upstream does. Upstream took any `.json`/`.xml` entry: every other entry (`UploadersConfig.json`, `Queue.json`, any name with a folder, `..` or a dot first) is left out and named in a message after the import. |
| Personal folder setting (`PersonalPath.cfg` next to the exe) | Fixed at `~/Library/Application Support/Modlight` | The page shows it with Open; Browse stays off. |
| Windows OCR | Vision | See "ShareX tools". |
| Import of ShareX settings | Not applicable | There is no ShareX on macOS. |
| `CaptureHelpers.IsActiveWindowFullscreen` (`DisableNotificationsOnFullscreen`) | The active app's presentation options include full screen (`NSApplication.currentSystemPresentationOptions`) | Read on the main thread when a task finishes. |
| `FileHelpers.DeleteFile(path, sendToRecycleBin)` (the notice's "Delete file") | `NSFileManager.trashItemAtURL` (to the Trash) | After the same Yes/No question. |
| `Directory.GetFiles(folder, "*.*", AllDirectories)` for a folder upload | The same walk, without Finder's `.DS_Store` | A Mac writes one into nearly every folder it opens; Windows has nothing comparable, so it is not sent as one of the person's files. |
| `Ctrl+V` on the main window (`KeyDown`) | `Cmd+V` through an AppKit local key monitor | egui only reports a paste when the clipboard holds text; the monitor sees the key whatever is on the pasteboard. A local monitor needs no permission. |
| `SystemSounds` / `SoundPlayer` (task sounds) | `NSSound` with upstream's WAV files | Normal mode only. |
| Drag-and-drop upload window (`DragDropUploadWindow`) | An undecorated, always-on-top viewport at the configured screen corner | Text dragged onto it is not accepted: winit delivers only files. |
| Proxy settings for uploads | Stored, not applied | The proxy page edits upstream's `ProxySettings`; uploads and downloads still go direct. |

## ShareX tools (phase 5)

| Windows (upstream) | macOS (this port) | Notes |
|---|---|---|
| Windows OCR (`Windows.Media.Ocr`) | Apple Vision (`VNRecognizeTextRequest`, accurate level) | No permission of its own. Languages are Vision's BCP 47 tags, named in the app's language and sorted by name; a tag saved on Windows ("en", "pl") selects the Vision language with the same language subtag. Upstream's help button (getsharex.com) is not drawn. |
| Screen colour picker on a GDI copy of the desktop | A frozen desktop from ScreenCaptureKit, one texture per display | Screen Recording permission (as captures). `Ctrl` keys are `Cmd`. |
| Pinned pictures (layered top-most forms) | Transparent, undecorated, always-on-top viewports | Opacity with Cmd+wheel (Ctrl+wheel upstream). |
| ZXing.Net | rxing 0.9.3 (a Rust port of ZXing) | Same formats and hints; SVG output written like ZXing.Net's. |
| Video converter: `ffmpeg.exe` with NVENC, AMF, Quick Sync | ffmpeg from Components (or the recorder's own ffmpeg path, which a backup import resets) | The Windows hardware encoders are not offered; a saved one encodes with VideoToolbox (`h264_videotoolbox`, `hevc_videotoolbox`). Arguments are passed as a vector, never through a shell. |
| Metadata: `exiftool.exe` | ExifTool's Perl distribution from Components, run with `/usr/bin/perl` | Upstream's arguments (`-G -t -m -q`; strip `-all= -overwrite_original -q`). |
| Analyze image keys: `[JsonEncrypt]` (DPAPI) inside the settings file | The login Keychain only: service `Modlight`, accounts `ai-openai-api-key`, `ai-gemini-api-key`, `ai-openrouter-api-key`, written through `/usr/bin/security` | The settings file never holds a key and a key found in one (an import) is not read. QA runs keep keys in memory. Requests go out only when the person runs an analysis, "Test connection" or "Load models". |
| Background remover: ONNX Runtime with DirectML on the adapter with the most video memory | ONNX Runtime 1.31 through `ort` 2.0.0-rc.15, linked statically (the build fetches Microsoft's library from pyke's CDN and checks its pinned SHA-256); "GPU" is Core ML, "Auto" Core ML then the CPU | Same pre- and post-processing (Skia's Catmull-Rom without mipmaps, ImageNet normalisation, sigmoid for logits, `DstIn`); the picture is decoded without EXIF turning, as `SKBitmap.Decode`. WebP saves are lossless (Skia's were lossy at quality 100). The guide button (getsharex.com) is not drawn. |
| Active window borderless (window styles off, `SetWindowPos` to the screen) | The frontmost app's focused window moved and sized to its screen (or the area free of the menu bar and Dock, "exclude taskbar area") through the Accessibility API; the same hotkey on that window puts it back | Another app's title bar cannot be taken off without private APIs, so the frame stays. Needs Accessibility permission (the system asks once; the hotkey then explains where to allow it). |
| Active window top-most (`WS_EX_TOPMOST`) | Not possible: the hotkey says so and points at Pin to screen | Window levels belong to the app that owns the window; changing another app's needs private APIs (`CGSSetWindowLevel` from that app's connection) or code injection. |
| History windows: shell thumbnails, Avalonia date pickers, dragging rows out | Thumbnails decoded from the files (no Quick Look); the date range as text in the culture's short date; no dragging out (egui has no drag source) | "Delete file and history item" deletes the file for good, as upstream (the dialog says so). The preview of an uploaded picture is fetched (30 s, 32 MB at most) and decoded under the app's limits. |
| Printing: `PrintDocument`, `PrintDialog`, `PrintPreviewDialog` | `NSPrintOperation` (with the print panel when "Show print dialog" is on); the preview is the page as a PDF opened in Preview | Placement in .NET's hundredths of an inch, turned into points, so a picture prints at the size it printed on Windows. The printer override is a printer's name (`NSPrinter`). |
| `ImageHelpers.LoadImage` on files, the clipboard and URLs | `modlight_core::image_decode`'s limits, EXIF orientation applied | |
| Unhandled exceptions window | Panics on background threads open the error window | |

## Components, downloads and updates (phase 3, media)

| Windows (upstream) | macOS (this port) | Notes |
|---|---|---|
| Components in `Documents\Modlight\Tools` / `Models` | `~/Library/Application Support/Modlight/Tools` / `Models` | Same staging (`.incoming`), part names, retired-file names. |
| ShareX/FFmpeg 8.1 win zip | Martin Riedl's macOS arm64 8.1 zip (one `ffmpeg`) | SHA-256 `cc3a7e0c...01fa3a`, 27185234 bytes, hash published beside the zip. |
| yt-dlp.exe / yt-dlp_arm64.exe | `yt-dlp_macos` 2026.08.19 (universal), installed as `Tools/yt-dlp` | SHA-256 `0f192b7e...f7b202`, 37146048 bytes; GitHub's digest, `SHA2-256SUMS` (GPG-signed) and our hash agree. Self-update needs both published digests to agree and an https artefact on exactly `github.com` (upstream accepted either digest). |
| ExifTool 13.29 Windows zip | ExifTool 13.59 Perl tarball; `exiftool` + `lib/` in `Tools/ExifTool/`, run with `/usr/bin/perl` | SHA-256 `668ea3ac...8fd65a`, 7916358 bytes (exiftool.org checksums). SourceForge keeps only recent versions: the pin will need a bump. |
| Background model | upstream's pin, unchanged (`bria-rmbg-2.0.onnx`, 1024331469 bytes, `5b486f08...e79958`) | Checked by metadata only. |
| Installer component page and first-run components window | Components are offered by the features that need them and by Settings -> Components | |
| `ChildProcessJob` | Process group per child, killed as a group on cancel and at exit | A crash leaves children to finish on their own. |
| Tool search: beside the exe, `Tools`, PATH | Same, plus Homebrew, MacPorts and `~/.local/bin` | An app opened from Finder inherits only the system PATH. |
| Bundled QuickJS | Not bundled; found in `Tools` or the search above | |
| WebView2 with its own profile for the YouTube sign-in | `WKWebView` in a native window, non-persistent data store, Safari's product token in the user agent | Cookies go to a Netscape jar (folder 0700, file 0600) passed to yt-dlp only for YouTube links. Google may refuse an embedded browser: a real-window check. |
| Update: Inno Setup `.exe` downloaded and run | `.dmg`/`.zip` downloaded, size and SHA-256 verified, revealed in Finder | Off unless `MODLIGHT_DEFAULT_HOST` is set at build time (https, no trailing slash). The server's `/api/releases/app` publishes one release, so a Mac build hides the Windows installer: a per-platform manifest is a server change. |

## Texts

- Where upstream's text names Windows, a PC, DirectShow or Explorer, the port draws the
  macOS text from `assets/mac/l10n/Mac.resx` / `Mac.pl.resx` (`modlight_core::l10n::mac`):
  the hotkey notice, the FFmpeg device status and path placeholder, the integration and
  local folders pages and their navigation entries, the watch folders and file names
  hints, the first-run window, the print dialog check box.
- Inter's Latin subsets have no "→"; it is drawn with Lucide's arrow.

## Look

- **Font.** Upstream asks for `Geist, Segoe UI Variable Text, Segoe UI, Inter`. On a Mac
  none but Inter exists, so Inter is what Avalonia itself would draw with there, and it
  is what the port bundles (Regular, Medium, SemiBold; Latin and Latin Extended). The
  Windows screenshots use Segoe UI, whose line height is 1.33 against Inter's 1.21:
  heights that follow text are a few pixels shorter (a file card is 195 px instead of
  201 px) and lines run a little longer, so a header that fits one line in Segoe UI may
  wrap to two. Monospace is egui's Hack instead of Cascadia Mono / Consolas.
- **Text colour and weight.** Upstream's global `TextBlock` style makes every
  TextBlock SemiBold and `Text.Secondary` unless the element says otherwise, and as a
  style setter it beats a colour inherited from a parent (a selected list row, a
  hovered button). Labels the caller wrote into a button as TextBlocks follow it; a
  button whose `Content` is a plain string draws its label in the button's own
  Foreground (`Text`; Accent.Foreground on `.primary`), as the Windows screenshots
  show. The port follows both rules rather than egui's defaults.
- **Shadows.** egui has no negative spread; the notice's drop shadow approximates
  Avalonia's blur and alpha.
- **Toasts** in the window corner stack newest first (upstream's `Insert(0)`); a notice
  with an empty name or detail line keeps the line's height, as Avalonia does.

## Behaviour that differs on purpose

- **Name previews** on the file names page are computed when an input changes, as
  upstream's `UpdatePreviews`, not on every frame.
- **About** keeps the team and the project page; upstream's links to getsharex.com and
  ShareX's stores and social accounts are left out (PORT.md). The logo's clicker game is
  not ported.
- **Image effects** is the effects branch's; until it lands its settings are edited and
  stored and its buttons say "later". Printing and OCR are phase 5's tools.
- **The image editor selector** stores the choice the first time the editor is asked
  for; both choices open the same editor, because the legacy (Greenshot) editor is a
  WinForms program and is not ported.
- **Language** applies at once; upstream asks for a restart. Only Automatic, English and
  Polish are offered, because those are the two translations upstream ships.
- **QA mode** (`--qa-profile <dir>`, `--qa-shot <screen> <out.png>`): settings, queue,
  history and token in the given folder, no tray, no Keychain, no global hotkeys, no
  login item, no browser, downloads to `<dir>/Downloads` without a save panel. Captures
  come from a fake desktop (`<dir>/qa-capture/{desktop.png,layout.json}` or a built-in
  one), hotkeys from a fake backend (pressed through `captures.simulated`), devices of
  the FFmpeg window from a fixed list, and "Save as" writes to `<dir>/Saved`; no
  ScreenCaptureKit, Accessibility, camera, microphone or login item call is made.
- **Settings backup import** restores what runs by itself switched off: an external
  program of "Perform actions", the "Perform actions" step after capture, and watch
  folders, whenever the backup switches one on that this installation did not already
  have switched on for the same task (the default task settings; a hotkey by its place
  in the list; a program only when the very same one was on). A custom FFmpeg path
  (`OverrideCLIPath` with `CLIPath`) that this task did not already start comes back to
  the FFmpeg from Components. It is the only path to a program the settings hold: yt-dlp
  and ExifTool are found in Tools and the usual places, never read from the settings, and
  `BrowserPath` is stored but never started (links go through `/usr/bin/open`). A
  message after the import names each one, and they are set again in the task
  settings. Upstream restores the file whole, so a backup from somebody else could start
  a program on the next capture or recording. Restoring one's own backup on the same Mac
  changes nothing; on another Mac these wait to be set again.
- **Pictures from outside the app** (thumbnails, previews and the account picture from
  the server, Upload from URL previews, the editor's "open from URL", the clipboard,
  files opened, inserted or uploaded) are decoded under limits
  (`modlight_core::image_decode`): at most 32768 px a side and 512 MiB for a still
  picture, 128 MiB per frame of a moving preview, decoded one frame at a time. A larger
  one counts as no picture (the editor says upstream's "The URL does not point to a
  valid image" or "Failed to load image file"); the editor stops a download from an
  address past 256 MiB. Files the system decodes for the editor (`NSImage`, HEIC) are
  measured first and refused by the same limits. Upstream (SkiaSharp) decoded whatever
  size a file claimed.
- **Deleting the folder you stand in** reloads the folder list; upstream loses that
  refresh and keeps showing the deleted folder until the next reload.
- **Sign-in loopback** follows the audit (PORT.md section 5): CORS limited to the
  server's origin, no `Access-Control-Allow-Private-Network`, and a request with the
  wrong `state` is answered and ignored instead of ending the sign-in.
- **Redirects** are never followed by any request that carries the token (upstream
  followed them in the library client). A stored tus session URL is moved onto the
  account's current origin before every request, so the token can never follow a
  `Location` to another host.
- **Live upload refusals** are read as soon as the server answers, without sending the
  rest of the body (upstream wrote the whole file first). A file name containing `"`,
  CR or LF goes out percent-encoded in the multipart header.
- **Error texts** keep upstream's shape (the readable sentence, then the request URL,
  status, headers and response) without the .NET stack trace section; transport errors
  are Rust's wording, and the HTTP reason phrase is the standard one for the status.
- **Upload from URL**: a download that answers with a non-success status is an error
  (upstream saved nothing and went on to upload a missing file).
- **Task questions** (large file, file exists, before upload) open one window at a
  time, oldest first; every task waiting for a sign-in hears the answer when the sign-in
  window closes.
- **Paste window**: the hint says "Ctrl+Enter" as upstream does; Cmd+Enter works too.
- **Desktop notice actions**: "Annotate image" opens the editor on the file; "Pin to
  screen" pins it (phase 5); every other action is upstream's.
- **GIF bit depth 4** is written as a 256-colour GIF (the encoder has no 16-colour
  palette mode). A custom time zone other than UTC in file name patterns falls back to
  local time.

## Image editor

| Upstream (Avalonia, SkiaSharp, Windows) | This port | Notes |
|---|---|---|
| `Ctrl` shortcuts of the editor (`Ctrl+Z`, `Ctrl+C`, `Ctrl+Shift+S`, ...) | `Cmd` | As everywhere. Delete also on Backspace (a Mac keyboard's Delete key); Shift+Backspace clears all. Cmd+H (toggle toolbars) is the system's Hide on a Mac and may never reach the window. Menus and tooltips show "Cmd". |
| Ctrl+mouse wheel zoom, middle-button pan | Cmd+wheel and trackpad pinch (one zoom level per step, 150 ms apart), middle-button pan, two-finger scroll | |
| Cmd+V in the editor | AppKit key monitor (as the main window) | egui reports a paste only when the clipboard holds text. |
| Avalonia's default font (Inter) and `FontWeight.Bold` | Bundled Inter Regular and SemiBold; bold Inter text uses SemiBold (Bold is not bundled), other families bold and italic from the system or simulated (Skia's fake bold, 0.2 skew) | "Segoe UI" (upstream's default family) is not on a Mac and falls back to Inter, as Avalonia does there. |
| Emoji (`EmojiPreviewImage`, Segoe UI Emoji / Noto) | Apple Color Emoji `sbix` strikes | |
| Cursor annotation (`WindowsCursorBitmapRenderer`, Windows cursors; nothing on a Mac) | `NSCursor` pictures at 2x, trimmed to the cursor | Arrow, Default, AppStarting, Help and WaitCursor are the arrow; SizeAll/NoMove2D the open hand; vertical and horizontal kinds the resize cursors; diagonals macOS 15's frame-resize cursors (crosshair before). |
| Wallpaper background (`MacOSDesktopWallpaperService`: `osascript` to System Events, `defaults read`) | `NSWorkspace.desktopImageURL`/`desktopImageOptions` of the main screen, read through `NSImage` (HEIC) | Upstream's way needs the Automation permission; this one needs none. Scaling none = Center, axes independently = Stretch, proportionally down = Fit, up or down = Fill (Fit without clipping). |
| `ColorView` (Avalonia's colour picker) | Same three tabs (spectrum, palette, components), `FluentColorPalette` with `ColorPickerPanel`'s three replacements, the previewer with accent shades, "Pick colour from screen" | Drawn by the port; sizes approximate Avalonia's template. "Pick colour from screen" is the tools' screen colour picker. |
| Font picker previews in each family | Drawn with the editor's text engine | egui renders only its own fonts. |
| Text box over a text annotation (Avalonia `TextBox` in the family) | egui text box in Inter at the annotation's size | The annotation itself is drawn in its family once the box closes. Rotated text is edited unrotated. |
| Printing, pin to screen | The tools' print window and pinned picture | See "ShareX tools". |
| `EditorWindow` maximised (`WindowState="Maximized"`) | Zoomed window (`with_maximized`) | "Remember window state" keeps size and zoom. |
