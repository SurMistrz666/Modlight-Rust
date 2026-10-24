//! One screen recording from start to file (`ScreenRecordManager.StartRecording`'s
//! background half and `ScreenRecorder`), with the controls the recording window and the
//! hotkeys work (`ScreenRecordWindow`: start/stop, pause/resume, restart, abort).
//!
//! The window never touches ffmpeg: it flips [`RecorderControl`], and the session thread
//! reads it, exactly as upstream's form and its `RecordResetEvent` do.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::args::{RecordingInput, ScreenRecordingOptions, gif_encode_args};
use super::ffmpeg::{FFmpegListener, FFmpegRun, concatenate, run};
use super::options::{FFmpegOptions, FFmpegVideoCodec};
use crate::capture::geometry::IRect;

/// `ScreenRecordingStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingStatus {
    Waiting,
    Working,
    Recording,
    Paused,
    Stopped,
    Aborted,
}

/// `ScreenRecordState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordState {
    Waiting,
    BeforeStart,
    AfterStart,
    AfterRecordingStart,
    RecordingEnd,
    Encoding,
}

/// A stopwatch that can be paused (`Stopwatch`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stopwatch {
    started: Option<Instant>,
    elapsed: Duration,
}

impl Stopwatch {
    pub fn start(&mut self) {
        if self.started.is_none() {
            self.started = Some(Instant::now());
        }
    }

    pub fn stop(&mut self) {
        if let Some(started) = self.started.take() {
            self.elapsed += started.elapsed();
        }
    }

    pub fn reset(&mut self) {
        self.started = None;
        self.elapsed = Duration::ZERO;
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed + self.started.map(|s| s.elapsed()).unwrap_or_default()
    }

    pub fn is_running(&self) -> bool {
        self.started.is_some()
    }
}

/// What the window shows and the session reads.
#[derive(Clone, Debug)]
pub struct ControlState {
    pub status: RecordingStatus,
    pub state: RecordState,
    pub timer: Stopwatch,
    pub is_countdown: bool,
    pub countdown: Duration,
    /// The recording's fixed length, seconds (zero: until stopped).
    pub duration: f64,
    /// Encoding progress, 0 to 100.
    pub progress: Option<i32>,
    pub restart_requested: bool,
    /// The recorded area, which the timer handle may move before recording starts.
    pub region: IRect,
    /// The session is over (the window closes).
    pub finished: bool,
    /// Abort was asked for. Final: a status the session sets afterwards (ffmpeg saying it
    /// has started) does not undo it.
    pub aborted: bool,
}

/// The switches between the recording window and the session thread.
pub struct RecorderControl {
    state: Mutex<ControlState>,
    /// `RecordResetEvent`.
    reset: Mutex<bool>,
    signal: Condvar,
    /// How often ffmpeg has been asked to finish (`StopRequested`) since the current part
    /// began: each request is one `FFmpegCLIManager.Close`, and the third kills it.
    stop: AtomicU32,
}

/// Seconds from a settings file as a `Duration`: one too large for a timer is the
/// longest there is, and anything that is not a positive number is none.
fn seconds(value: f64) -> Duration {
    if value > 0.0 {
        Duration::try_from_secs_f64(value).unwrap_or(Duration::MAX)
    } else {
        Duration::ZERO
    }
}

/// The start delay as upstream waits it: `(int)(ScreenRecordStartDelay * 1000)`
/// milliseconds, and none when that is not a positive `int`.
fn start_delay(value: f64) -> Duration {
    let milliseconds = value * 1000.0;
    if milliseconds >= 1.0 && milliseconds <= i32::MAX as f64 {
        Duration::from_millis(milliseconds as u64)
    } else {
        Duration::ZERO
    }
}

impl RecorderControl {
    pub fn new(region: IRect, duration: f64) -> Arc<RecorderControl> {
        Arc::new(RecorderControl {
            state: Mutex::new(ControlState {
                status: RecordingStatus::Waiting,
                state: RecordState::Waiting,
                timer: Stopwatch::default(),
                is_countdown: false,
                countdown: Duration::ZERO,
                duration,
                progress: None,
                restart_requested: false,
                region,
                finished: false,
                aborted: false,
            }),
            reset: Mutex::new(false),
            signal: Condvar::new(),
            stop: AtomicU32::new(0),
        })
    }

    pub fn snapshot(&self) -> ControlState {
        self.state.lock().unwrap().clone()
    }

    pub fn status(&self) -> RecordingStatus {
        self.state.lock().unwrap().status
    }

    fn set_status(&self, status: RecordingStatus) {
        self.state.lock().unwrap().status = status;
    }

    fn set_event(&self) {
        *self.reset.lock().unwrap() = true;
        self.signal.notify_all();
    }

    fn reset_event(&self) {
        *self.reset.lock().unwrap() = false;
    }

    /// Wait for the event; with a timeout, false when it ran out.
    fn wait_event(&self, timeout: Option<Duration>) -> bool {
        let mut set = self.reset.lock().unwrap();
        match timeout {
            None => {
                while !*set {
                    set = self.signal.wait(set).unwrap();
                }
                true
            }
            Some(timeout) => {
                let deadline = Instant::now() + timeout;
                while !*set {
                    let now = Instant::now();
                    if now >= deadline {
                        return false;
                    }
                    set = self.signal.wait_timeout(set, deadline - now).unwrap().0;
                }
                true
            }
        }
    }

    fn request_stop(&self) {
        self.stop.fetch_add(1, Ordering::SeqCst);
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_requests() > 0
    }

    /// How often ffmpeg was asked to finish since the current part began.
    pub fn stop_requests(&self) -> u32 {
        self.stop.load(Ordering::SeqCst)
    }

    /// Whether Abort was pressed (or its hotkey), whatever the status says now.
    pub fn is_aborted(&self) -> bool {
        self.state.lock().unwrap().aborted
    }

    /// The part the session is about to record begins, when the status still allows
    /// it (`Waiting` or `Paused`, and no abort): `ChangeState(AfterStart)` and a fresh
    /// count of stop requests, in one step, so an abort pressed at that very moment is
    /// not undone by it. Returns the status the part began from.
    fn begin_part(&self) -> Option<RecordingStatus> {
        let mut state = self.state.lock().unwrap();
        let from = state.status;
        if state.aborted || !matches!(from, RecordingStatus::Waiting | RecordingStatus::Paused) {
            return None;
        }
        state.state = RecordState::AfterStart;
        state.status = RecordingStatus::Working;
        self.stop.store(0, Ordering::SeqCst);
        Some(from)
    }

    /// `StartStopRecording`.
    pub fn start_stop(&self) {
        match self.status() {
            RecordingStatus::Working => self.abort(),
            RecordingStatus::Recording => {
                self.set_status(RecordingStatus::Stopped);
                self.request_stop();
            }
            RecordingStatus::Paused => {
                self.set_status(RecordingStatus::Stopped);
                self.set_event();
            }
            _ => self.set_event(),
        }
    }

    /// `PauseResumeRecording`.
    pub fn pause_resume(&self) {
        if self.status() == RecordingStatus::Recording {
            self.set_status(RecordingStatus::Paused);
            self.reset_event();
            self.request_stop();
        } else {
            self.set_event();
        }
    }

    /// `AbortRecording`.
    pub fn abort(&self) {
        {
            let mut state = self.state.lock().unwrap();
            state.status = RecordingStatus::Aborted;
            state.aborted = true;
        }
        self.request_stop();
        self.set_event();
    }

    /// `RestartRecording`.
    pub fn restart(&self) {
        let stop_active = {
            let mut state = self.state.lock().unwrap();
            if !matches!(
                state.status,
                RecordingStatus::Recording | RecordingStatus::Paused
            ) {
                return;
            }
            let active = state.status == RecordingStatus::Recording;
            state.restart_requested = true;
            state.status = RecordingStatus::Waiting;
            state.timer.reset();
            state.is_countdown = false;
            active
        };
        if stop_active {
            self.request_stop();
        }
        self.set_event();
    }

    fn consume_restart(&self) -> bool {
        std::mem::take(&mut self.state.lock().unwrap().restart_requested)
    }

    fn restart_requested(&self) -> bool {
        self.state.lock().unwrap().restart_requested
    }

    /// Move the recorded area (the window's timer handle, before recording starts).
    pub fn move_region(&self, region: IRect) {
        let mut state = self.state.lock().unwrap();
        if matches!(
            state.status,
            RecordingStatus::Waiting | RecordingStatus::Paused
        ) {
            state.region = region;
        }
    }

    /// `StartCountdown`.
    fn start_countdown(&self, delay: Duration) {
        let mut state = self.state.lock().unwrap();
        state.is_countdown = true;
        state.countdown = delay;
        state.timer.start();
    }

    /// `ChangeState`.
    pub fn change_state(&self, new_state: RecordState) {
        let mut state = self.state.lock().unwrap();
        state.state = new_state;
        match new_state {
            RecordState::AfterStart => state.status = RecordingStatus::Working,
            RecordState::AfterRecordingStart => {
                state.status = RecordingStatus::Recording;
                // `StartRecordingTimer`.
                if state.is_countdown {
                    state.timer.reset();
                    state.is_countdown = false;
                }
                if state.duration > 0.0 {
                    state.is_countdown = true;
                    state.countdown = seconds(state.duration);
                }
                state.timer.start();
            }
            RecordState::RecordingEnd => state.timer.stop(),
            _ => {}
        }
    }

    fn set_progress(&self, progress: i32) {
        self.state.lock().unwrap().progress = Some(progress);
    }

    fn finish(&self) {
        self.state.lock().unwrap().finished = true;
    }

    /// The timer text, `mm:ss:ff` (`UpdateTimer`).
    pub fn timer_text(state: &ControlState) -> String {
        let value = if state.is_countdown {
            state.countdown.saturating_sub(state.timer.elapsed())
        } else {
            state.timer.elapsed()
        };
        let total_ms = value.as_millis() as u64;
        let minutes = (total_ms / 60_000) % 60;
        let seconds = (total_ms / 1000) % 60;
        let hundredths = (total_ms % 1000) / 10;
        format!("{minutes:02}:{seconds:02}:{hundredths:02}")
    }
}

/// Frames of the screen for the pipe (GDI on Windows, ScreenCaptureKit on a Mac, or a
/// fake in QA).
pub trait FrameSource: Send {
    /// The frame size in pixels.
    fn size(&self) -> (u32, u32);
    /// Start delivering frames.
    fn start(&mut self) -> Result<(), String>;
    /// The newest frame, BGRA, `width * height * 4` bytes; `None` until the first one.
    fn latest(&mut self) -> Option<Arc<Vec<u8>>>;
    fn stop(&mut self);
}

/// Makes a frame source for an area of the desktop.
pub type FrameSourceFactory =
    Arc<dyn Fn(IRect, bool) -> Result<Box<dyn FrameSource>, String> + Send + Sync>;

/// Everything a session needs, decided before it starts.
#[derive(Clone)]
pub struct SessionParams {
    pub ffmpeg_path: PathBuf,
    pub ffmpeg: FFmpegOptions,
    pub fps: i64,
    pub duration: f64,
    pub draw_cursor: bool,
    pub two_pass: bool,
    pub auto_start: bool,
    pub start_delay: f64,
    /// The recording's file (its extension is the codec's, or mp4 before a second pass).
    pub path: PathBuf,
    /// Where the second pass writes (`ProcessTwoPassEncoding`'s output).
    pub two_pass_output: PathBuf,
    pub frames: FrameSourceFactory,
}

/// How a session ended.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionOutcome {
    /// The finished file, when there is one.
    pub path: Option<PathBuf>,
    pub aborted: bool,
    /// ffmpeg's output when it failed (shown in the output box upstream).
    pub error: Option<String>,
}

struct Listener {
    control: Arc<RecorderControl>,
}

impl FFmpegListener for Listener {
    fn encode_started(&self) {
        self.control.change_state(RecordState::AfterRecordingStart);
    }

    fn encode_progress(&self, percentage: f32) {
        self.control.set_progress(percentage as i32);
    }
}

/// `FileHelpers.AppendTextToFileName`.
pub fn append_to_file_name(path: &Path, text: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem}{text}.{}", ext.to_string_lossy()),
        None => format!("{stem}{text}"),
    };
    path.with_file_name(name)
}

/// One ffmpeg run of a recording, feeding frames when the input is the pipe.
fn record_once(
    params: &SessionParams,
    options: &ScreenRecordingOptions,
    control: &Arc<RecorderControl>,
) -> Result<(), String> {
    let args = options
        .ffmpeg_commands()
        .ok_or_else(|| "no video or audio source".to_string())?;
    let listener: Arc<dyn FFmpegListener> = Arc::new(Listener {
        control: control.clone(),
    });
    let pipe = options.input() == RecordingInput::Pipe;
    let mut source = if pipe {
        let mut source = (params.frames)(options.capture_area, options.draw_cursor)?;
        source.start()?;
        Some(source)
    } else {
        None
    };
    let mut ffmpeg = match FFmpegRun::start(&params.ffmpeg_path, &args, pipe, false, listener) {
        Ok(ffmpeg) => ffmpeg,
        Err(error) => {
            // The capture started for it must not go on (a ScreenCaptureKit stream is
            // not stopped by letting go of it).
            if let Some(source) = source.as_mut() {
                source.stop();
            }
            return Err(error.to_string());
        }
    };

    let mut written = 0u64;
    if let Some(source) = source.as_mut() {
        let interval = Duration::from_secs_f64(1.0 / options.fps.max(1) as f64);
        let frame_limit = if options.duration > 0.0 {
            (options.fps as f64 * options.duration).round() as u64
        } else {
            0
        };
        let expected = (source.size().0 as usize) * (source.size().1 as usize) * 4;
        let waited_from = Instant::now();
        let mut next = Instant::now();
        let mut stdin = ffmpeg.take_stdin();
        while !control.stop_requested() && (frame_limit == 0 || written < frame_limit) {
            let Some(frame) = source.latest() else {
                if waited_from.elapsed() > FIRST_FRAME_TIMEOUT {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            if frame.len() != expected {
                break;
            }
            let Some(pipe) = stdin.as_mut() else { break };
            if std::io::Write::write_all(pipe, &frame).is_err() {
                break;
            }
            written += 1;
            if written == 1 && control.status() == RecordingStatus::Working {
                // ffmpeg reading frames from its standard input does not print "Press [q]
                // to stop": the first frame it accepted is when the recording started.
                control.change_state(RecordState::AfterRecordingStart);
            }
            next += interval;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                // Behind: carry on without sleeping, as upstream does with FPS drops.
                next = now;
            }
        }
        drop(stdin);
        source.stop();
    } else {
        // ffmpeg's own camera and microphone input: it stops when `q` is typed, and only
        // then (the end of its standard input does not stop it). If the app exits first,
        // it is asked to finish then.
        ffmpeg.stop_at_exit();
        // One `Close` per request to stop, as upstream's: `q`, `q` again, then a kill.
        // Not more: after the `q` ffmpeg still writes the end of the file, which for an
        // MP4 moves its index to the front and takes a while for a long recording.
        let mut closed = 0;
        while ffmpeg.is_running() {
            let requests = control.stop_requests();
            while closed < requests {
                ffmpeg.close();
                closed += 1;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let (ok, output) = ffmpeg.wait();
    if !ok {
        return Err(output);
    }
    if pipe && written == 0 && !control.stop_requested() {
        // Not one frame arrived: an empty video is a failure, not a recording.
        return Err(output);
    }
    Ok(())
}

/// How long the screen is given to deliver the recording's first frame.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// `ScreenRecordManager.StartRecording`'s task. Blocks until the recording is done.
pub fn run_session(params: SessionParams, control: Arc<RecorderControl>) -> SessionOutcome {
    let path = params.path.clone();
    let concat_path = append_to_file_name(&path, "-concat");
    let temp_path = append_to_file_name(&path, "-temp");
    let _ = std::fs::remove_file(&concat_path);
    let _ = std::fs::remove_file(&temp_path);
    let mut abort_requested = false;
    let mut recorded = false;
    let mut error = None;

    while !abort_requested
        && matches!(
            control.status(),
            RecordingStatus::Waiting | RecordingStatus::Paused
        )
    {
        control.change_state(RecordState::BeforeStart);
        if control.status() == RecordingStatus::Paused || !params.auto_start {
            control.wait_event(None);
        } else {
            let delay = start_delay(params.start_delay);
            if !delay.is_zero() {
                control.start_countdown(delay);
                control.wait_event(Some(delay));
            }
        }

        if control.is_aborted() {
            abort_requested = true;
        }

        if control.consume_restart() {
            recorded = false;
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&concat_path);
            let _ = std::fs::remove_file(&temp_path);
        }

        if let Some(from) = control.begin_part() {
            if from == RecordingStatus::Paused && path.exists() {
                let _ = std::fs::rename(&path, &concat_path);
            }
            let region = control.snapshot().region;
            let options = ScreenRecordingOptions {
                is_recording: true,
                is_lossless: params.two_pass,
                input_path: None,
                output_path: path.clone(),
                fps: params.fps,
                capture_area: region,
                frame_size: (0, 0),
                duration: params.duration,
                draw_cursor: params.draw_cursor,
                ffmpeg: params.ffmpeg.clone(),
            };
            // The frame size is the source's; ask it once before building the arguments.
            let options = match (params.frames)(region, params.draw_cursor) {
                Ok(probe) => ScreenRecordingOptions {
                    frame_size: probe.size(),
                    ..options
                },
                Err(message) if options.input() == RecordingInput::Pipe => {
                    error = Some(message);
                    abort_requested = true;
                    break;
                }
                Err(_) => options,
            };
            match record_once(&params, &options, &control) {
                Ok(()) => recorded = true,
                Err(output) => {
                    if !control.stop_requested() {
                        error = Some(output);
                    } else {
                        recorded = path.exists();
                    }
                }
            }
            control.change_state(RecordState::RecordingEnd);
            if control.is_aborted() {
                abort_requested = true;
            }
            if control.restart_requested() {
                continue;
            }
            if error.is_some() {
                abort_requested = true;
                break;
            }
        }

        if concat_path.exists()
            && path.exists()
            && concatenate(
                &params.ffmpeg_path,
                &[concat_path.clone(), path.clone()],
                &temp_path,
                true,
            )
        {
            let _ = std::fs::rename(&temp_path, &path);
        }
    }

    // Abort pressed after the last look (while the parts were being joined, say) is
    // still an abort: upstream read it only inside the loop and delivered the file.
    if control.is_aborted() {
        abort_requested = true;
    }
    let mut final_path = path.clone();
    if params.two_pass && !abort_requested && recorded && path.exists() {
        control.change_state(RecordState::Encoding);
        let (output, failure) = two_pass_encode(&params, &path, &control);
        final_path = output;
        if failure.is_some() {
            error = failure;
        }
        // The abort hotkey works while the window is hidden for the second pass.
        if control.is_aborted() {
            abort_requested = true;
        }
    }

    if abort_requested {
        let _ = std::fs::remove_file(&final_path);
        let _ = std::fs::remove_file(&path);
    }
    let _ = std::fs::remove_file(&concat_path);
    let _ = std::fs::remove_file(&temp_path);
    control.finish();

    let exists = final_path.exists();
    SessionOutcome {
        path: (!abort_requested && error.is_none() && exists).then_some(final_path),
        aborted: abort_requested && error.is_none(),
        error,
    }
}

/// `ProcessTwoPassEncoding`: GIF through the palette filters, anything else re-encoded
/// with the chosen codec; the lossless first pass is deleted. Returns the finished file
/// and, when the pass failed, ffmpeg's output for the output box (upstream's
/// `ShowError`).
fn two_pass_encode(
    params: &SessionParams,
    input: &Path,
    control: &Arc<RecorderControl>,
) -> (PathBuf, Option<String>) {
    let output = params.two_pass_output.clone();
    // The first pass and the finished file share a name when the codec's container is
    // MP4 and the name pattern gives the same name twice (both are named when the
    // recording starts; upstream named the second at the end). ffmpeg cannot write over
    // the file it reads, so the first pass moves aside to the session's scratch name.
    let mut input = input.to_path_buf();
    if input == output {
        let aside = append_to_file_name(&input, "-temp");
        let _ = std::fs::remove_file(&aside);
        if std::fs::rename(&input, &aside).is_ok() {
            input = aside;
        }
    }
    let listener: Arc<dyn FFmpegListener> = Arc::new(Listener {
        control: control.clone(),
    });
    let (ok, ffmpeg_output) = if params.ffmpeg.VideoCodec == FFmpegVideoCodec::gif {
        run(
            &params.ffmpeg_path,
            &gif_encode_args(&input, &output, &params.ffmpeg),
            true,
            listener,
        )
    } else {
        let options = ScreenRecordingOptions {
            is_recording: false,
            is_lossless: false,
            input_path: Some(input.clone()),
            output_path: output.clone(),
            fps: params.fps,
            duration: 0.0,
            ffmpeg: params.ffmpeg.clone(),
            ..ScreenRecordingOptions::default()
        };
        match options.ffmpeg_commands() {
            Some(args) => run(&params.ffmpeg_path, &args, true, listener),
            None => (false, String::new()),
        }
    };
    if input != output && input.exists() {
        let _ = std::fs::remove_file(&input);
    }
    if ok {
        (output, None)
    } else {
        log::warn!("The second pass of the recording failed.");
        (output, Some(ffmpeg_output))
    }
}

/// A frame source that repeats one picture (QA, tests): BGRA from an RGBA image.
pub struct StillFrames {
    frame: Arc<Vec<u8>>,
    size: (u32, u32),
    started: bool,
}

impl StillFrames {
    pub fn new(image: &image::RgbaImage) -> StillFrames {
        let mut bytes = image.as_raw().clone();
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        StillFrames {
            frame: Arc::new(bytes),
            size: (image.width(), image.height()),
            started: false,
        }
    }
}

impl FrameSource for StillFrames {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn start(&mut self) -> Result<(), String> {
        self.started = true;
        Ok(())
    }

    fn latest(&mut self) -> Option<Arc<Vec<u8>>> {
        self.started.then(|| self.frame.clone())
    }

    fn stop(&mut self) {
        self.started = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_controls_follow_the_recording_window() {
        let control = RecorderControl::new(IRect::new(0, 0, 64, 64), 0.0);
        assert_eq!(control.status(), RecordingStatus::Waiting);
        control.change_state(RecordState::AfterStart);
        assert_eq!(control.status(), RecordingStatus::Working);
        control.change_state(RecordState::AfterRecordingStart);
        assert_eq!(control.status(), RecordingStatus::Recording);
        control.pause_resume();
        assert_eq!(control.status(), RecordingStatus::Paused);
        assert!(control.stop_requested());
        control.restart();
        assert_eq!(control.status(), RecordingStatus::Waiting);
        assert!(control.consume_restart());
        control.change_state(RecordState::AfterRecordingStart);
        control.start_stop();
        assert_eq!(control.status(), RecordingStatus::Stopped);
        let control = RecorderControl::new(IRect::default(), 3.0);
        control.change_state(RecordState::AfterRecordingStart);
        let text = RecorderControl::timer_text(&control.snapshot());
        assert!(text.starts_with("00:02:") || text == "00:03:00", "{text}");
        control.abort();
        assert_eq!(control.status(), RecordingStatus::Aborted);
    }

    #[test]
    fn a_fixed_length_too_long_for_a_timer_does_not_panic() {
        // A settings file can hold any number; the countdown saturates instead.
        let control = RecorderControl::new(IRect::new(0, 0, 8, 8), 1e300);
        control.change_state(RecordState::AfterRecordingStart);
        assert_eq!(control.status(), RecordingStatus::Recording);
        let text = RecorderControl::timer_text(&control.snapshot());
        assert_eq!(text.len(), 8, "{text}");
        control.change_state(RecordState::RecordingEnd);
        assert_eq!(start_delay(1e300), Duration::ZERO);
        assert_eq!(start_delay(-3.0), Duration::ZERO);
        assert_eq!(start_delay(2.5), Duration::from_millis(2500));
    }

    #[test]
    fn an_abort_is_never_undone() {
        // Pressed just before a part begins: the part does not begin.
        let control = RecorderControl::new(IRect::new(0, 0, 8, 8), 0.0);
        control.abort();
        assert_eq!(control.begin_part(), None);
        assert_eq!(control.status(), RecordingStatus::Aborted);
        // Pressed while ffmpeg was starting, which then says it has started: the status
        // reads Recording, the abort stands.
        let control = RecorderControl::new(IRect::new(0, 0, 8, 8), 0.0);
        assert_eq!(control.begin_part(), Some(RecordingStatus::Waiting));
        assert_eq!(control.status(), RecordingStatus::Working);
        control.start_stop();
        control.change_state(RecordState::AfterRecordingStart);
        assert!(control.is_aborted());
        assert_eq!(control.stop_requests(), 1);
        control.abort();
        assert_eq!(control.stop_requests(), 2);
    }

    #[test]
    fn appended_names_keep_the_extension() {
        assert_eq!(
            append_to_file_name(Path::new("/a/b.c/clip.mp4"), "-concat"),
            PathBuf::from("/a/b.c/clip-concat.mp4")
        );
        assert_eq!(
            append_to_file_name(Path::new("/a/clip"), "-temp"),
            PathBuf::from("/a/clip-temp")
        );
    }
}
