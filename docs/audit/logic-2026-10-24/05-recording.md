# Logic review 2026-10-24, area 05: screen recording, ffmpeg options, components, Download tab

Branch `logic-05-recording`, from main `28916b4`. Reviewed against upstream
(`ShareX/ScreenRecordManager.cs`, `ShareX.ScreenCaptureLib` ScreenRecorder*, FFmpeg*,
`Presentation/{ScreenRecording,FFmpegOptions}`, `ShareX.HelpersLib` FFmpegCLIManager,
`ShareX/Modlight/ModlightComponent*.cs`, `ShareX/Presentation/{Download,Components}`).
Files: `modlight-core/src/{recording,components,download}/*`, `modlight-app/src/{recording_ui,
ffmpeg_window,components_window,download_tab}.rs`, `modlight-macos/src/webview.rs`.

Every finding below was reproduced by a test that failed before the fix (except F2, whose
mechanism was checked by hand: ffmpeg 9.0.1 with standard input at EOF keeps running), and
the test is kept. Line numbers are those of `28916b4`.

## Findings

| Id | Severity | Where | What was wrong | How it was reproduced | Status |
|---|---|---|---|---|---|
| F1 | High | `modlight-core/src/recording/session.rs:447-452` | A camera or microphone recording (ffmpeg's own avfoundation input, stopped with `q`) was killed about 100 ms after Stop: the session called `FFmpegRun::close` every 50 ms while a stop was pending, and the third call kills. ffmpeg had not finished the file yet (an MP4 moves its index to the front at the end): the recording came out corrupt or missing. | Stand-in ffmpeg that needs 600 ms after `q` to write its file: no file. Second stand-in that ignores `q`: killed after one request. | Fixed `50c4925`: one `Close` per request to stop, as upstream (`q`, `q`, kill on the third). Tests `a_camera_recording_is_given_the_time_to_finish_its_file`, `a_camera_recording_that_will_not_stop_is_killed_on_the_third_request`. |
| F2 | High | `modlight-core/src/recording/ffmpeg.rs:99`, `session.rs:446-452` | Quitting Modlight during a camera or microphone recording left ffmpeg running on its own, still recording: it stops on `q` or a signal, never at the end of standard input (screen recordings are fine, their frames arrive on stdin). Upstream has the same orphan (for every recording). | `ffmpeg -re -f lavfi -i testsrc -f null - < /dev/null` still running after 3 s; test with a stand-in that, like ffmpeg, finishes on SIGINT. | Fixed `9a6446a` (macOS/unix): such an ffmpeg is registered and sent SIGINT (ffmpeg writes the end of the file and exits) by an `atexit` hook, which also runs on `process::exit`. Test `a_device_recording_is_asked_to_finish_when_the_app_exits`. Windows not changed (see below). |
| F3 | Medium | `session.rs:592-632` (`two_pass_encode`), `:629` | A failed second pass (every GIF, two-pass video) was only logged; the first pass is deleted either way, so the recording vanished without a word. Upstream shows ffmpeg's output (`ShowError`). | GIF with `GIFDither=bayer`, `GIFBayerScale=99` (a value from a settings file; the window offers 0 to 5): no error in the outcome. | Fixed `6c4d28b`: ffmpeg's output is the session's error, shown in the output box. Test `a_second_pass_that_fails_says_why`. |
| F4 | Medium | `modlight-app/src/recording_ui.rs:589`, `session.rs:592` | Both names of a two-pass recording are taken when it starts (upstream took the finished one at the end). With an MP4 codec and a name pattern without a random part they are the same file: ffmpeg refuses to write over its input ("Output same as Input"), and the lossless 4:4:4 first pass (QuickTime and browsers cannot play it) was delivered as the recording. | Two-pass libx264 session with equal paths: the delivered file had x264's `qp=0` settings, not `crf=28.0`. | Fixed `6c4d28b`: the first pass moves aside to the session's scratch name before the second pass; the finished file keeps the pattern's name. Test `a_second_pass_never_writes_over_its_own_input`. |
| F5 | Medium | `session.rs:486`, `:504`, `:543`; `change_state(AfterRecordingStart)` | Abort was read only twice inside the loop, from the status, which later steps overwrite. Abort pressed while the paused parts were being joined, during the second pass (its hotkey; the window is hidden then) or while ffmpeg was starting (its "Press [q]" sets Recording) was lost and the recording was delivered and uploaded. Upstream has the same holes. | Stand-in that delays the concat (and the palette pass) by 1.5 s, Abort during it: the file was delivered. | Fixed `af2db08`: a sticky `aborted` flag; a part begins only through `begin_part` (status, flag and the stop counter in one locked step); the flag is read again after the loop and after the second pass. MACOS.md "Behaviour that differs on purpose" updated. Tests `an_abort_while_the_parts_are_joined_leaves_nothing`, `an_abort_during_the_second_pass_leaves_nothing`, `an_abort_is_never_undone`. |
| F6 | Medium | `session.rs:399-400` | When ffmpeg could not start (a custom path to a file that is not a program), `record_once` returned without stopping the frame source it had just started; on a Mac a ScreenCaptureKit stream, which dropping does not stop (`MacFrames` has no `Drop`). | Counting frame source, non-executable ffmpeg: started 1, stopped 0. | Fixed `b0244ee`. Test `the_screen_is_let_go_when_ffmpeg_cannot_start`. |
| F7 | Medium | `modlight-app/src/capture_flow.rs:607` (`FinishRecording`) | The name typed in the after-capture window for a recording was applied with `std::fs::rename`, which silently replaces a file of that name in the screenshots folder (upstream's `File.Move` refuses and says "Rename file error"). | `rename_recording(clip, "holiday")` with `holiday.mp4` present: the other video was overwritten. | Fixed `4a7e1e0`: the rename refuses, upstream's message names the file, the recording keeps its name. Rename moved to `recording_ui::rename_recording` (one call left in `capture_flow.rs`). Test `a_new_name_never_replaces_another_file`. |
| F8 | Low | `session.rs:413-419`, `:455` | When the screen delivered no frame within 5 s, ffmpeg wrote an empty MP4, exited 0, and the empty video went to the after-capture tasks and the upload. | Frame source that never delivers: an empty `clip.mp4` came back as the result. | Fixed `b0244ee`: no frame and no stop is a failure (ffmpeg's "Output file is empty" in the output box). Test `a_recording_without_a_single_frame_is_a_failure`. |
| F9 | Low | `session.rs:276`, `:479` | `ScreenRecordDuration` / `ScreenRecordStartDelay` too large for a `Duration` (any number can be in a settings file or arrive by sync) panicked: the start delay in the session thread (the recording window stayed forever, no new recording possible), the fixed length inside the control's lock (poisoned: the toolbar's next frame crashed the app). | Start delay 1e300: panic in `time.rs`; `RecorderControl::new(_, 1e300)` + `AfterRecordingStart`: panic. | Fixed `168345b`: the delay is waited as upstream (`(int)(s * 1000)` ms, none when it does not fit), the countdown saturates. Tests `an_impossible_start_delay_starts_at_once`, `a_fixed_length_too_long_for_a_timer_does_not_panic`. |
| F10 | Low | `modlight-core/src/components/mod.rs:559` (`migrate`) | The one-time clean-up of a broken custom ffmpeg path tested the raw string, while the recorder (and upstream's check) resolve it with `GetAbsolutePath`: a working path written with a variable (`%HOME%/bin/ffmpeg`, `%TMPDIR%...`) or relative to the app was dropped. | `%TMPDIR%/<dir>/ffmpeg` with the file present: `OverrideCLIPath` reset to false. | Fixed `4d8b410`. Test `a_working_ffmpeg_path_with_a_variable_is_kept`. |
| F11 | Low | `modlight-app/src/ffmpeg_window.rs:158-166` | The FFmpeg options window took a custom path raw (no `GetAbsolutePath`): with such a path the recorder worked but the window listed no cameras or microphones and said ffmpeg was not found. | Window with `%TMPDIR%/tools/ffmpeg`: path kept the `%`. | Fixed `2f4bcbf`: the window asks `FFmpegOptions::ffmpeg_path` like the recorder. Test `the_window_uses_the_ffmpeg_the_recorder_uses`. |
| F12 | Low | `recording_ui.rs:395` | The region recording hotkey checked only for an open overlay, not for one being prepared (`captures.busy`), which every other way to open the overlay waits for: a double press took the desktop twice and replaced the first overlay (a selection already begun was lost). | Two presses with a counting capture source: two desktop pictures. | Fixed `aa2d31f`. Test `a_second_press_while_the_area_is_prepared_asks_nothing_more`. |
| F13 | Low | `recording_ui.rs:277-281` | Wrong text keys on the toolbar: while recording upstream's `UpdateUI` shows `Strings.Pause` ("Pauza"), the port `ScreenRecordWindow_Pause` ("Wstrzymaj"); Start used the window's key instead of `ScreenRecordForm_Start` (same words). | Unit test on the labels in Polish. | Fixed `227ad7d`; snapshots `recording-toolbar/{dark,light}-pl` updated. Test `the_buttons_say_what_upstream_says`. |
| F14 | Low | `recording_ui.rs:669` | "Action completed" was played once, after the second pass and only when a file was delivered; upstream's loop plays it when each part ends (a pause, the stop, an abort of the delay), so a GIF sounds when recording ends, not after the encode. | Session counting ended passes: none existed. | Fixed `274f7b1`: `ControlState.parts_ended` at upstream's place; the window plays the sound for each. Test `every_part_that_ends_is_counted_for_the_sound`. |

Shared files touched outside the area: `app/docs/MACOS.md` (one bullet under "Behaviour
that differs on purpose", F5) and `app/crates/modlight-app/src/capture_flow.rs` (F7: the
rename block replaced by one call to `recording_file_named`).

## Not fixed

- **Windows**: a dshow camera or microphone recording is still left running when the app
  exits (F2's hook is unix-only). This Mac cannot build the Windows target (C dependencies
  need MSVC), so nothing was changed there; the job-object `GroupGuard` of
  `components::process` would be the place (it kills rather than finishes the file).
- **Abort during the second pass** does not interrupt the encode: it runs to its end and
  everything is then deleted (upstream sent `q` and delivered the partial file).
- **`version_check`** keeps an "Unknown" standing for 10 minutes after its 20 s deadline;
  upstream's exception path cached nothing. This is the documented "could not find out for
  ten minutes" rule, so it was left.
- **StopScreenRecording hotkey** acts only while recording; upstream called `Close` on every
  press (a third press killed a stuck ffmpeg). Abort still escalates after F1 (each abort is
  one `Close`).
- **"Install what is missing"**: cancelling one row stops the rest of the chain here
  (`components_window::install_done`); upstream's loop goes on to the next component. Left
  for the owner to decide (stopping looks intended; a test would need a network-free
  install harness, and the chain's installs reach the publishers).

## Checked and OK

- Argument vectors (`args.rs`) for every codec, preset, CRF/bitrate, xvid, VP8/VP9, hardware
  codecs mapped to VideoToolbox on a Mac, webp/apng, lossless first pass, audio codecs, `-t`
  in the invariant culture, `-tag:v hvc1`; user arguments and custom commands split as
  `CommandLineToArgvW`, `$output$` one argument; pipe input size from the frame source (even,
  at the display's scale); camera/microphone inputs; GIF palette filter identical to
  `FFmpegEncodeAsGIF`; concat list quoting; avfoundation device list parsing; `FixSources`;
  extensions; audio off for animated formats.
- `RecorderControl` against `ScreenRecordWindow` (StartStop, PauseResume, Restart, Abort,
  the manual-reset `RecordResetEvent`, timer text); pause/resume parts joined and their inputs
  removed; restart deletes path, concat and temp; region and its even size; QA frames.
- FFmpeg options window: numeric ranges and lists equal to the `.axaml`, write-back on close
  (upstream writes back after its modal dialog too), custom-command template, device status
  texts.
- Components: resumable download (Range, 206 start checked, 416 restarts once, long body
  deleted, short body kept for resume, no size = failure, 60 s stall watchdog, connect
  timeout), SHA-256 gate (delete on mismatch and on no digest), staging and part names,
  space checks (as upstream, without counting the part already downloaded), rename-aside
  replacement, retired and staging sweeps, archive limits and traversal, release parsing
  (draft/prerelease refused, both digests, github.com only), version probe with timeout and
  group kill, schedule with backoff, stamp only after a completed check, Components window
  rows, chains, cancel on close.
- Download tab: `DownloadLink` whitelist, format selector and sort, yt-dlp argument vector
  (`--` before the URL, no `--exec`, cookies only for YouTube), progress lines (NA,
  estimates, finished = working), error classification (same markers and order as upstream;
  yt-dlp's own messages are English), last-error choice, file resolution order, queue state
  machine (one at a time, oldest first, pause/resume/cancel/retry/remove), retry after a
  captured sign-in, update-and-retry, delivery for Disk/Modlight/Both, scratch sweep, text
  keys against upstream's (only the documented gaps: WebView2 runtime, tool missing at run
  time).
- YouTube cookies and WKWebView: Netscape jar format, folder 0700 / file 0600 written
  atomically, names-only reading with the cache, sign-out, non-persistent data store,
  signed-in-page rule, 800 ms settle, harvest and close without releasing the delegate in
  its own callback.

## Verification

`cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean,
and `cargo clippy --target x86_64-pc-windows-msvc -p modlight-core -p modlight-app
--all-targets -- -D warnings` clean too (with a no-op C compiler: checked, not built);
`cargo test -p modlight-core --lib --test download_flow --test media_net --test recording
--test behind_proxy` and `cargo test -p modlight-app --lib --test snapshots` green (the
recording tests use Homebrew's ffmpeg 9.0.1; download_flow uploads to and deletes from the
reference server). Not run here: other areas' server tests (`task_flow`, `capture_upload`,
`reference_app`).
