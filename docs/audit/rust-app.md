# Security audit of the Modlight App (Rust) for macOS

Date: 2026-10-20. Scope: `app/` in `/Users/marcinasdfg/Desktop/modlight` (branch main, commit
`6eed920`), the Rust workspace: `modlight-api`, `modlight-core`, `modlight-macos`,
`modlight-app`, `modlight-editor` (about 125 thousand lines). Method: static reading +
greps + building and running the whole test suite plus small probe tests in a working
copy (`_scratch/sec-review-app`, outside the main repo). The main checkout was read-only.
The rules the port was to preserve: `app/docs/PORT.md` section 5; the macOS substitutions:
`app/docs/MACOS.md`; the audit of the original: `docs/audit/original-client.md`.

Tests: `cargo test --workspace` = 544 passed, 0 failed, 1 skipped (manual),
including the tests against the original server at `http://127.0.0.1:8420`. The QA account was
signed in and cleaned up after the test; nothing was left behind.

## MAIN VERDICT: does the application send data outside

**There is no backdoor, no telemetry and no hidden exfiltration.** Every outgoing connection
is permanently bound to the account's host (the token only in `x-api-key`, never in a URL, never behind a
redirect), to `github.com` (components with a pinned SHA-256) or to the update
host fixed at build time. No Sentry/GA/Mixpanel/Plausible or similar (grep
clean). No disabling of TLS validation. The allow-list of fields in sync works
(the token, AI keys, proxy password, paths, window positions and cookies do not leave).

There is, however, **one real way to get the token out**, not through the network code but
through importing a settings backup: a malicious `.sxb` file can replace the account's host,
leaving the real token in the Keychain (finding **W1** below). It requires the user to
import a file from the attacker themselves, but the effect is the token being sent to the attacker's server.

---

## Does the application send data outside (every connection, verdict)

| Destination | What | When | User's decision | Verdict |
|---|---|---|---|---|
| the account's host (`Credential.host`) | token in `x-api-key`, files, names, metadata, settings/skin blob | upload, library, sync | deliberate sign-in | OK: the token only in the header, the host cannot be overwritten by a reply (see below) |
| the account's host: thumbnails/conversions/downloads | token in a header | browsing/downloading | n/a | OK: every link from the server is passed through `rewrite_to_host` (the origin forced to the account's host) |
| `github.com` / `api.github.com` | plain GETs without user data | components, yt-dlp auto-update | chosen in Components | OK: `net::is_artefact_url` enforces https + exactly `github.com`; the requirement that both digests agree |
| `ffmpeg.martin-riedl.de`, `downloads.sourceforge.net` (+ redirects) | GET | downloading ffmpeg/ExifTool | chosen in Components | OK: https only, redirects only to https, SHA-256 pinned |
| the update host (`MODLIGHT_DEFAULT_HOST`, build-time) | GET `/api/releases/app` + download of the `.dmg`/`.zip` | at start + manually | installation after consent and a matching SHA-256 | OK: empty by default = disabled; redirect none |
| `accounts.google.com` / `youtube.com` (WKWebView) | the user's Google sign-in | only a click on "sign in to YouTube" | deliberate | OK: a non-persistent data store, cookies only local |
| an address typed/pasted in (Upload from URL, the editor's "open from URL") | no credentials | manually | deliberate | OK as to the token; see **S3** (no image decoding limit) |

Evidence that the token does not leak (confirmed by reading):
- `modlight-api/src/client.rs:72-83,221-229`: both `reqwest` clients have
  `redirect::Policy::none()`; `request()` adds `x-api-key` and nothing else;
  `absolute()` (`:231`) joins only relative paths with the account's host, and all calls
  of `send_json`/`get_string` use path literals (`/api/me`, `/api/uploads`, ...).
- `modlight-api/src/host.rs:75-89` `rewrite_to_host`: from a link it takes only the path and
  query, the origin is always the account's host; `:92` `same_origin`.
- `modlight-core/src/account.rs:216-237` `adopt`: the `/api/me` reply never overwrites
  the host or the token (the comment and the code agree with the original).
- tus: `modlight-api/src/tus.rs:799-819` `resolve_location` and `:351`: the session is always
  moved to the account's origin before every request; the token does not follow `Location`.
- sync: `modlight-core/src/sync/document.rs:117-184` builds the document solely from
  `APP_MEMBERS`, normalizes `DefaultTaskSettings` (removes `AIOptions`, paths,
  `CLIPath`/`OverrideCLIPath`, sound paths); `ProxySettings` is not on the list, so
  the proxy password does not leave. The test `only_the_allowlist_travels` confirms this.

---

## Findings (severity, file:line, scenario, confidence, fix)

### [High] W1: importing a backup replaces the account's host and sends the real token to the attacker's server
`modlight-core/src/backup.rs:52-80` (`import`), the filter `:63` + `is_plain_name` `:93`.

`import` writes to the profile folder every zip entry with a "plain" name and the extension
`json`/`xml`. `UploadersConfig.json` is such a name. On macOS the token is NOT in this file;
it lies in the Keychain (`modlight-core/src/account.rs:17,134-144`), and the file holds only the account's
host. After the import, on the next start `AccountStore::load` (`main.rs:151`) reads the host
from the replaced `UploadersConfig.json` and the token from the Keychain (untouched), and pairs them:
`credential()` returns `host = attacker`, `token = real`. Every subsequent request goes
to the attacker's host with `x-api-key: <the victim's token>`. `adopt` does not correct the host.

- **Scenario**: the attacker sends a file "my Modlight settings.sxb"; the victim does
  `Ustawienia → Aplikacja → Importuj kopię` (Settings → Application → Import backup) (`app_pages.rs:1951-1990`). The file contains
  `UploadersConfig.json` = `{"ModlightAccount":{"Host":"https://zlo.example","Tag":"…"}}`.
  After a restart the victim's token flies to `zlo.example`.
- **Regression relative to the original**: on Windows the token is in the same
  `UploadersConfig.json` (DPAPI), so overwriting it also deletes the token: the attacker does not know a
  valid token. Splitting "host in the file / token in the Keychain" (a deliberate macOS substitution)
  created a hole the original did not have. `docs/audit/original-client.md` explicitly
  notes that the ShareX import in the installer deliberately does NOT copy `UploadersConfig.json`,
  "because that would carry secrets over"; the port abandoned that rule.
- **Confidence: confirmed** (a probe in the working copy: after the import the `/api/me` request
  went to the trap host with the `x-api-key` header = the token; the whole path
  import -> load -> credential was read).
- **Fix**: in `import` skip the client state files that `export`
  (`backup.rs:24-48`) never creates anyway: `UploadersConfig.json`, `Queue.json`,
  and `History.db` itself. The export writes only `ApplicationConfig.json`,
  `HotkeysConfig.json`, `History.db`, so excluding `UploadersConfig.json`/`Queue.json`
  does not change correct operation. Best: a whitelist of names allowed for import
  instead of a blacklist.

**Status after the fixes** (branch `app-secfix`, commit `735b6a4`):
- `backup::import` accepts only `ApplicationConfig.json` and `HotkeysConfig.json`
  (a whitelist of exact names); `History.db` is skipped as in the original. Every other
  entry (`UploadersConfig.json`, `Queue.json`, names with a folder, `..`, a leading dot,
  a different letter case) is rejected and listed in the message after the import (a status
  line + a window with a warning). Nothing is written until the whole archive
  has been read.
- Defence in depth: the credential store (Keychain / QA file) keeps the token together with the origin
  it was issued for (`account.rs`, `BoundToken`). A host in `UploadersConfig.json`
  different from that origin (an edited file, someone's backup) gives a "signed out" state: the token is not
  paired with it until the user signs in on that host. A token saved by
  an earlier version (without an origin) requires a one-time re-login.
- Tests: `backup::tests::a_backup_cannot_repoint_the_account`,
  `backup::tests::import_takes_only_the_files_export_writes`,
  `account::tests::a_host_swapped_behind_the_token_never_gets_it`,
  `account::tests::a_token_that_does_not_name_its_host_is_not_used`.

### [Medium] W2: importing a backup (Queue.json) deletes an arbitrary file on the next start
`modlight-core/src/backup.rs:52` + `modlight-core/src/upload/store.rs:71` (`load`) +
`modlight-core/src/upload/queue.rs:963-979` (`expire`) -> `fail` `:900-904` ->
`release` `:983-985` -> `modlight-core/src/upload/store.rs:172-182` (`discard_spool`).

`Queue.json` is also a "plain" `.json` name, so the import writes it. A queue item with
`OwnsSource = true`, an arbitrary `SourcePath` and a past `DeadlineUtc` is loaded at start
(`load`), considered expired (`expire`) and released; `discard_spool`
deletes `source_path` without checking whether it lies in the spool folder, and so any file
of the user. The deletion is permanent (`std::fs::remove_file`, not to the Trash).

- **Scenario**: a malicious `.sxb` with a `Queue.json` pointing at e.g. `~/Documents/praca.docx`,
  `OwnsSource=true`. After a restart the file disappears.
- **Confidence: confirmed** (a probe: the victim file deleted by the restored queue; without
  signing in and without a network).
- **Fix**: as in W1, do not import `Queue.json`. Additionally (defence in depth)
  `discard_spool` should delete only files in the queue's spool folder.

**Status after the fixes** (branch `app-secfix`, commits `735b6a4` and `ce46e09`):
- `Queue.json` is no longer imported (the whitelist from W1, `735b6a4`).
- `QueueStore::discard_spool` (`ce46e09`) deletes a file only if the path is the one
  `QueueStore::spool` assigns: absolute, without `..`, directly in the spool folder
  and named with the item's identifier (`{id}` + extension); never a directory. `OwnsSource = true`
  from `Queue.json` alone is not enough; other paths are skipped with a log entry.
- Tests: `upload::queue::tests::a_queue_file_from_elsewhere_cannot_delete_the_persons_files`
  (a restart with a `Queue.json` pointing at a file outside the spool: the file stays, the queue's own file
  disappears), `upload::store::tests::only_spooled_names_are_ever_deleted`,
  `backup::tests::a_backup_cannot_replace_the_upload_queue`.

### [Medium] W3: importing a backup (ApplicationConfig.json) can configure an external program to run automatically -> code execution
`modlight-core/src/backup.rs:52` -> `app_pages.rs:1972` (`import`) ->
`reload_settings_files` `:1802-1806` -> `apply_settings_document` `:1787-1799`;
`modlight-core/src/tasks/worker.rs:773-790` (`PerformActions`) ->
`modlight-core/src/tasks/actions.rs:104-136` (`run`).

`ApplicationConfig.json` is a legitimate backup file (the export creates it), so the import overwrites it
entirely and applies it at once. `DefaultTaskSettings.ExternalPrograms` with
`IsActive=true`, `Path="/bin/sh"`, `Args="-c '…'"` plus an `AfterCaptureJob` containing
`PerformActions` makes an arbitrary command run on the first capture/hotkey.
`actions.rs` does not use a shell, but the program can BE one (`/bin/sh -c`).
Watch folders from the imported config also start tasks.

- **Confidence: confirmed by reading** (the path import -> reload -> defaults_from ->
  worker -> actions is complete).
- **Note**: this behavior is consistent with the original (restoring one's own backup rightly
  restores external actions) and works only once the user deliberately imports a foreign
  file. It is more a trust boundary than a port bug, but the effect (code execution) justifies
  the Medium severity.
- **Fix**: after an import do not silently enable external actions/watch folders without
  confirmation, or warn that the backup may contain commands run automatically.
  At a minimum: since the export does not save the token anyway, limit the import to the three names
  that the export produces, and consider asking for confirmation when a config enables
  `PerformActions`/a watch folder.

**Status after the fixes** (branch `app-secfix`, commit `ece123a`):
- The import compares each restored task with the same task from before the import
  (`DefaultTaskSettings` from `ApplicationConfig.json`; a hotkey from `HotkeysConfig.json` by
  position in the list) and disables what runs by itself and was not already enabled in that
  task: an active external program (it stays enabled only if an identical program was
  already active), the `PerformActions` step in `AfterCaptureJob` and watch folders (they stay
  only if each folder was already watched). The rest of the file is restored unchanged;
  one's own backup on the same Mac comes back byte for byte, on a different Mac these elements wait for
  manual enabling.
- The message after the import lists every disabled element (task + action/path), so that the
  user enables them deliberately. The departure from the original is described in `app/docs/MACOS.md`
  ("Behaviour that differs on purpose"). A settings file that is not a JSON object
  aborts the import before anything is written.
- Tests: `backup::tests::programs_a_backup_switches_on_come_in_switched_off` (`/bin/sh -c`,
  an own hotkey with `PerformActions`, a new watch folder),
  `backup::tests::ones_own_backup_comes_back_as_it_was`,
  `backup::tests::a_settings_file_that_is_not_json_fails_the_import`.
- Program paths (commit `da9b31e`): a custom FFmpeg path
  (`FFmpegOptions.OverrideCLIPath` + `CLIPath`, in the default and hotkey settings),
  which that task did not run before, goes back to the FFmpeg from Components
  (`OverrideCLIPath = false`, `CLIPath = ""`) and is listed in the message after the import;
  a path with the switch off is not run and stays unchanged. This is the only
  path to a program in the settings: yt-dlp and ExifTool are looked up in `Tools` and
  typical locations, not read from the config, and `BrowserPath` is only stored
  (links go through `/usr/bin/open`). Test:
  `backup::tests::an_ffmpeg_path_a_backup_brings_goes_back_to_the_component`.

### [Medium] S3: no limits when decoding images from the server and from the user's addresses (memory DoS)
`modlight-app/src/media.rs:75` (`decode_still`), `:100,:107` (animations),
`modlight-app/src/uploads.rs:799` and `modlight-app/src/editor_window.rs:557-564`
(`download_image`), `modlight-api/src/web.rs:101-131`.

`image::load_from_memory` / the decoders are called on bytes from the server (thumbnails, avatar) and from
addresses pasted by the user (Upload from URL, the editor's "open from URL") without
setting `image::Limits`. The download is limited by the compressed size (8 MB
thumbnail, 4 MB animation, 50 MB preview), but not by dimensions. A crafted small PNG file with
huge dimensions (e.g. 30000x30000) causes an allocation on the order of GB and kills the process.

- **Scenario**: a hostile account server returns such a "thumbnail"; or the user pastes a URL to
  such an image. Effect: the application hangs/OOMs (DoS, not RCE, not a leak).
- **Confidence: likely** (the absence of any limit in the code confirmed by grep;
  a real PoC was not run, because it requires a large allocation on the owner's machine).
- **Fix**: decode through `image::ImageReader` with `.limits(Limits)` (an allocation limit and
  max dimensions) for all bytes from the network. The audit of the original listed "decoding of
  hostile images" as a point of attention.

**Status after the fixes** (branch `app-secfix`, commit `efb151c`):
- A new module `modlight_core::image_decode`: `from_bytes`, `from_bytes_as`, `open` with
  `image::Limits` (max 32768 px per side, 512 MiB per image), `fits()` for decoders without
  limits, `animation_limits()` (128 MiB per frame). Note: `ImageReader` already had
  512 MiB of allocation by default, but no dimension limit; the GIF/WebP animation decoders
  had no limit at all and held up to 60 full frames at once.
- All decoding of bytes from outside goes through this module: thumbnails, previews and
  the avatar (`media.rs`; animations decoded frame by frame under the limits and immediately
  reduced), the preview after an upload, task windows and notifications, "copy image",
  uploading a file with image processing (`worker.rs`), the editor: opening/inserting/background
  (`open_image`) and "open from URL". Clipboard: the image is read as PNG/TIFF bytes from
  `NSPasteboard` (`modlight_macos::pasteboard`) and decoded by us, not by arboard.
  `NSImage` (HEIC etc. in the editor) is first measured and rejected above these limits.
- Errors with the original's texts: "The URL does not point to a valid image", "Failed to
  download image: ...", "Failed to load image file. <path>"; downloading from an address in the
  editor is aborted above 256 MiB.
- Tests: `image_decode::tests::a_header_claiming_an_enormous_picture_is_refused_by_its_size`
  (real PNG and JPEG with a header of 30000x30000, 65535x65535, 32769x1),
  `media::tests::pictures_claiming_an_enormous_size_are_refused` (a thumbnail and a GIF animation
  with a canvas of 30000/40000 px),
  `editor_window::tests::a_downloaded_picture_claiming_an_enormous_size_is_not_a_valid_image`.

### [Low] S4: a link to the ShareX project in the "O programie" (About) window
`modlight-app/src/about_window.rs:266-282`: hyperlinks to `github.com/Jaex`,
`github.com/McoreD`, `github.com/ShareX/ShareX` (credit to the team). Opened only
on a click through `system::open_url`, which lets through only http(s). Consistent with PORT.md
("About keeps the team and the project page"). It is not an update source. No action;
I list it for completeness, because a grep for "ShareX/ShareX" hits here.

---

## Rules from the audit of the original (PORT.md section 5): preserved or not

1. **The token only in `x-api-key`, to the account's host, never in a URL / to another host / behind a redirect**
   : **PRESERVED** in the network layer (evidence in the section "Does the application send data outside"). With
   a caveat: the host the token is paired with can be replaced through a backup import
   (W1); this is not a bypass in the network code but in the import.
2. **The update host fixed (build-time), never from the sign-in field**: **PRESERVED**.
   `modlight-core/src/update.rs:27-54` `RELEASE_HOST = option_env!("MODLIGHT_DEFAULT_HOST")`,
   validation of `https://` and no trailing `/` in a `const` (otherwise a compile error); empty
   by default = disabled; `fetch` with `redirect::none()`; `is_acceptable_url` https everywhere, http
   only on loopback (for tests).
3. **Every downloaded executable/model verified with SHA-256 before use; no size/hash
   = failure**: **PRESERVED**. `components/verify.rs:65-98` `verify_or_delete` (deletes on
   a mismatch and on a missing digest); `components/download.rs:94-101` requires
   `expected_size > 0`, never trusts `Content-Length`; `components/mod.rs` pins the URL,
   SHA-256 and size as constants; `tool_release.rs:296-316` `with_published_digest` requires
   the digest from the GitHub API AND from `SHA2-256SUMS` to agree (stricter than the original); the application update
   requires a size and SHA (`update.rs:parse_manifest`, `download`).
4. **Sync sends only fields from the allow-list (no token, AI keys, proxy password,
   paths, positions, cookies)**: **PRESERVED**. `sync/document.rs:19-184`; the token is not
   even in the file; `ProxySettings`/`AIOptions`/paths outside the list or removed by
   `normalize_task_settings`. The test `only_the_allowlist_travels`.
5. **Sign-in loopback: 127.0.0.1, a random port, a CSPRNG `state` in constant time, a single
   shot, CORS only for the server's origin (no `*`, no `Access-Control-Allow-Private-Network`),
   a wrong `state` answered and ignored (does not abort)**: **PRESERVED in full**.
   `modlight-api/src/signin.rs`: `bind(("127.0.0.1", 0))` `:112`; `new_state` from
   `rand::rng()` (CSPRNG) `:62-67`; constant-time `nonce_equals` `:71-84`; a wrong state ->
   log and the loop continues `:142-147`; CORS only for `allowed_origin`, without private-network
   `:196-213,250-256`; a single shot (`wait_for` returns on the first matching state).
   Addition 2026-10-24 (line numbers above predate it): the sign-in also polls the server
   pickup (`POST /api/authorize/pickup`, SKELETON.md 9.44) with a 32-byte CSPRNG verifier
   whose SHA-256 challenge went out in the authorize link; the verifier never appears in a
   URL, the request carries no `x-api-key`, follows no redirects and goes only to the
   sign-in origin. The first of loopback and pickup wins; the `/api/me` check is unchanged.
6. **Child processes: an argument vector, `--` before the user's URL/path, no shell**:
   **PRESERVED**. yt-dlp: `download/run.rs:99-154` a vector, `--` before the URL (`:152`), no
   `--exec`; `download/link.rs:27-50` a URL whitelist (http/https, no userinfo). ffmpeg:
   `recording/ffmpeg.rs:84-96` a vector, no shell; the "custom commands" template
   `recording/args.rs:169-211` through `split_command_line`, not a shell; the pipe input
   always first. External programs `tasks/actions.rs:84-119`: split into a vector, then
   substitution of `$input`/`$output` into whole arguments (the test `placeholders_fill_whole_arguments`).
   `process::command` `components/process.rs:89-98` no shell, its own process group,
   stdin closed.
   Downloaded binaries: SHA-256 verification in the same function that places them
   (`install.rs:201-273`), an atomic write through `.incoming` staging + rename, permissions 0755/0644;
   the order download -> verify -> place -> delete-part closes the TOCTOU window between verification and
   running (the file that is run is the verified one, moved atomically).
7. **YouTube cookies: a local file, only for yt-dlp, only for YouTube URLs, never
   sent**: **PRESERVED**. `download/cookies.rs`: folder 0700 / file 0600 (`write`
   `:106-128`), `file_for` returns Some only for `is_youtube && is_signed_in` (`:244-246`),
   `--cookies` added only when there is a jar (`run.rs:120-123`), it does not go into the config,
   only the number of cookies is logged; WKWebView with a non-persistent store
   (`webview.rs:379`). The test `file_for` for TikTok = None.
8. **Do not carry over ShareX dead code (KeyboardHook, AppVeyor/GitHub ShareX update,
   puush, getsharex.com links)**: **PRESERVED**. There is no `KeyboardHook`/keylogger (no
   global keyboard monitor; `keys.rs` is a local monitor for Cmd+V only), no AppVeyor,
   no ShareX update check. `getsharex.com` only in "omitted" comments.

The substitutions from `MACOS.md` were checked and are consistent: the Keychain through `/usr/bin/security`
(the secret via stdin `security -i`, not via argv: `ps` shows only `/usr/bin/security -i`;
the token hex-encoded, item names validated by `plain()`), `/usr/bin/open` only for http(s)
and with the path after `--`, FSEvents for watch folders, the sign-in loopback as in section 5.

---

## Logic errors (upload to the wrong server / queue / history)

- **Upload to the wrong server after an account change does NOT occur.** `upload/queue.rs:246-255`
  `matches` compares the host (without a trailing `/`, case-insensitively) and the tag; an item for
  account A after signing in to B is skipped with the reason `AccountChanged` (`:587-589`), not
  sent. `item.rs:is_usable` and a comparison of size/modification time before sending. Consistent with
  the original (`ModlightUploadQueue.Matches`).
- **The offline queue and a new account: guarded.** The account slot is updated every frame
  (`uploads.rs:221-239`), and `due` (`queue.rs:575-603`) checks `matches` before every
  send anyway.
- **Another account's history**: `History.db` is shared by the profile (the `Host` column), the Upload
  card combines the history with the session; this is consistent with the original (one local library per machine).
  I found no leak of links between accounts in the read logic.
- **Double send / loss**: the queue is written atomically (`store.rs` tmp+rename), the claim is held until
  the result is stored, `is_worth_queuing` queues only a missing response and 408/429/502/503/504.
- The **fresh-install guard** in sync is preserved (`sync/plan.rs:90-96`): an installation never
  confirmed for the account does not push; the cloud wins. A newer schema -> it does nothing.
- Note: `discard_spool` (W2) deletes a `source_path` from outside the spool folder; described above.

---

## Things checked and fine

- No telemetry/analytics; no disabling of TLS; no `0.0.0.0`.
- The token: `Debug` for `Credential`/`SignedIn`/`QueueAccount`/`CookieData` masks the secret
  (`client.rs:48-55`, `signin.rs:293-301`, `webview.rs:30-38`); the logs contain only the number of
  cookies, item names, hosts, not the token (a grep of the logs). Error texts contain the server's
  RESPONSE headers, not the request's, so `x-api-key` does not get there (`upload.rs:337-362`,
  `uploader.rs:106-162`).
- Keychain: the secret is not in the process arguments (stdin `security -i`); no injection
  (names validated, token hex).
- Unpacking component archives (`components/archive.rs`): entries going outside the
  directory are skipped (`safe_relative`), links/devices never created, the size ceiling
  counted from both the declaration and the bytes actually written, a limit on the number of entries. The input is always
  after SHA-256 verification.
- Loopback, sync, downloading components/updates: as in section 5 above.
- FFI/`unsafe` in `modlight-macos` (37 in `capture.rs`, 19 in `webview.rs`, remnants
  elsewhere): reviewed. The blocks are standard AppKit/ScreenCaptureKit/WebKit calls on the
  main thread, with SAFETY comments, with retain/release through `Retained`/`RcBlock`;
  `copy_frame` (`capture.rs:610-639`) checks `width>=w && height>=h` and copies row by
  row within bounds; `to_rgba` (`:293-350`) cuts the row to `min(bytes.len())` and the column to
  `min(line.len()/4)`. The WKWebView delegate is not released in its own callback (retire on the
  next frame), correct. I found no UAF or overflows.
- `unwrap`/`expect`/indexing on remote data: response parsing goes through serde
  with `Default`/tolerant deserializers (`parse::<T>`, `null_default`, `lenient_time`);
  the indices in `library/folders.rs`/`budget.rs`/`caches.rs` are by positions from the same
  structure (internally consistent), not from the network. I found no panic from server data.
- Tests: the whole `cargo test --workspace` is green (544/0/1).

---

## Summary of severities

- Critical: none.
- High: **W1**: a backup import (`UploadersConfig.json`) rebinds the token to the attacker's host.
- Medium: **W2**: a backup import (`Queue.json`) deletes an arbitrary file; **W3**: a backup import
  (`ApplicationConfig.json`) can insert an automatically run program -> code execution;
  **S3**: no image decoding limits (memory DoS).
- Low: **S4**: a link to the ShareX repo in "O programie" (About) (no action).

The common denominator of W1-W3: `backup::import` accepts files that `backup::export` never
creates (`UploadersConfig.json`, `Queue.json`) and applies the config in full. The simplest
fix for W1+W2: a whitelist of names allowed for import = exactly what the export writes.

Status after the fixes (branch `app-secfix`): W1 `735b6a4`, W2 `735b6a4` + `ce46e09`,
W3 `ece123a` + `da9b31e` (the FFmpeg path), S3 `efb151c`; S4 unchanged (no action).
`cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test
--workspace` are green (558 passed, 0 failed, 1 skipped manual).
