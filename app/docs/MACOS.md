# Windows to macOS: what the port substitutes, and why

Upstream (`upstream/Modlight-App`) is a Windows program. Where Windows has a mechanism
macOS lacks, the port uses the closest macOS mechanism and records the swap here.
Everything not listed behaves as upstream does. The per-screen state is in
`STATUS.md`; the working rules in `PORT.md`.

## Platform services

| Windows (upstream) | macOS (this port) | Notes |
|---|---|---|
| Token encrypted with DPAPI inside `UploadersConfig.json` | Login Keychain, generic password, service `Modlight`, written and read through `/usr/bin/security` | An item created by the app binary itself would be tied to that build and every rebuild would prompt; `security` is a stable owner. QA runs (`--qa-profile`) keep the token in a plain file in the profile instead and never touch the Keychain. |
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
| Clipboard (`ClipboardHelpers`) | NSPasteboard through `arboard` | Text, images and Finder file lists. Reading asks for the file list first: Finder also puts the file's icon and name on the pasteboard, which upstream's image-then-text order would upload instead of the file. QA and headless runs never read the real clipboard (macOS may ask for paste permission). |
| Taskbar progress (`TaskbarManager`) | Not shown | macOS has no per-window taskbar progress; the tray icon's progress and the Upload tab carry it. |
| Inno Setup installer | `scripts/build-app.sh`: release build, `Modlight.app` with `Info.plist`, `.icns` cut from `ShareX/Modlight_Icon.ico`, ad hoc signature | Bundle id `org.modlight.Modlight` (to confirm with the owner). The script never starts the app. A DMG and components downloaded from the Components window come with phase 3. |
| Start with Windows | Login item (`SMAppService`) | Phase 3. |
| Explorer context menu / Send to | Finder Services / Quick Action | Phase 3. |
| Print Screen and global hotkeys (`RegisterHotKey`) | `global-hotkey`, defaults that do not clash with the system's own shortcuts | Phase 3. |
| GDI / DXGI capture | ScreenCaptureKit | Phase 3. |
| WebView2 (YouTube sign-in) | WKWebView | Phase 3. |
| Windows OCR | Vision | Phase 5. |
| Import of ShareX settings | Not applicable | There is no ShareX on macOS. |
| `CaptureHelpers.IsActiveWindowFullscreen` (`DisableNotificationsOnFullscreen`) | The active app's presentation options include full screen (`NSApplication.currentSystemPresentationOptions`) | Read on the main thread when a task finishes. |
| `FileHelpers.DeleteFile(path, sendToRecycleBin)` (the notice's "Delete file") | `NSFileManager.trashItemAtURL` (to the Trash) | After the same Yes/No question. |
| `Directory.GetFiles(folder, "*.*", AllDirectories)` for a folder upload | The same walk, without Finder's `.DS_Store` | A Mac writes one into nearly every folder it opens; Windows has nothing comparable, so it is not sent as one of the person's files. |
| `Ctrl+V` on the main window (`KeyDown`) | `Cmd+V` through an AppKit local key monitor | egui only reports a paste when the clipboard holds text; the monitor sees the key whatever is on the pasteboard. A local monitor needs no permission. |
| `SystemSounds` / `SoundPlayer` (task sounds) | `NSSound` with upstream's WAV files | Normal mode only. |
| Drag-and-drop upload window (`DragDropUploadWindow`) | An undecorated, always-on-top viewport at the configured screen corner | Text dragged onto it is not accepted: winit delivers only files. |
| Proxy settings for uploads | Not used yet | Upstream takes them from its proxy page (phase 3); uploads go direct. |

## Look

- **Font.** Upstream asks for `Geist, Segoe UI Variable Text, Segoe UI, Inter`. On a Mac
  none but Inter exists, so Inter is what Avalonia itself would draw with there, and it
  is what the port bundles (Regular, Medium, SemiBold; Latin and Latin Extended). The
  Windows screenshots use Segoe UI, whose line height is 1.33 against Inter's 1.21:
  heights that follow text are a few pixels shorter (a file card is 195 px instead of
  201 px) and lines run a little longer, so a header that fits one line in Segoe UI may
  wrap to two. Monospace is egui's Hack instead of Cascadia Mono / Consolas.
- **Text colour.** Upstream's global `TextBlock` style colours every label
  `Text.Secondary`, including the ones inside buttons; only `.active` buttons repaint
  theirs. The port follows that rather than egui's defaults.
- **Shadows.** egui has no negative spread; the notice's drop shadow approximates
  Avalonia's blur and alpha.
- **Toasts** in the window corner stack newest first (upstream's `Insert(0)`); a notice
  with an empty name or detail line keeps the line's height, as Avalonia does.

## Behaviour that differs on purpose

- **Language** applies at once; upstream asks for a restart. Only Automatic, English and
  Polish are offered, because those are the two translations upstream ships.
- **QA mode** (`--qa-profile <dir>`, `--qa-shot <screen> <out.png>`): settings, queue,
  history and token in the given folder, no tray, no Keychain, no global hotkeys, no
  login item, no browser, downloads to `<dir>/Downloads` without a save panel.
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
- **Desktop notice actions** "Annotate image" and "Pin to screen" do nothing until the
  image editor and the pin tool arrive (phases 3 and 4); every other action is
  upstream's.
- **GIF bit depth 4** is written as a 256-colour GIF (the encoder has no 16-colour
  palette mode). A custom time zone other than UTC in file name patterns falls back to
  local time.
