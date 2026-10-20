//! `FFmpegCLIManager`: one ffmpeg process, its output, the moment encoding starts and
//! the encode progress. Arguments go in as a vector; nothing passes through a shell.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the process said and how far it got.
#[derive(Debug, Default)]
pub struct FFmpegState {
    pub output: String,
    pub is_encoding: bool,
    pub video_duration: Option<Duration>,
    pub encode_time: Option<Duration>,
    /// 0 to 100 while an encode with a known length runs.
    pub percentage: Option<f32>,
}

/// Something that wants to hear when encoding starts or moves on.
pub trait FFmpegListener: Send + Sync {
    fn encode_started(&self) {}
    fn encode_progress(&self, _percentage: f32) {}
}

impl FFmpegListener for () {}

/// One run of ffmpeg.
pub struct FFmpegRun {
    child: Child,
    stdin: Option<ChildStdin>,
    state: Arc<Mutex<FFmpegState>>,
    reader: Option<std::thread::JoinHandle<()>>,
    close_tries: u32,
    stdin_is_frames: bool,
}

/// `HH:MM:SS.ff` as ffmpeg prints it.
pub fn parse_time(text: &str) -> Option<Duration> {
    let mut parts = text.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || h < 0.0 || m < 0.0 || s < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + s))
}

/// `UpdateEncodeProgress`: the input's length from `Duration:`, then `time=` against it.
pub fn track_line(state: &mut FFmpegState, line: &str) -> Option<f32> {
    if state.video_duration.is_none() {
        // "  Duration: 00:00:15.32, start: 0.000000, bitrate: 1095 kb/s"
        if let Some(rest) = line.trim_start().strip_prefix("Duration:")
            && let Some((time, after)) = rest.split_once(',')
            && after.trim_start().starts_with("start:")
        {
            state.video_duration = parse_time(time);
        }
        return None;
    }
    // "frame=  942 fps=187 q=35.0 size=    3072kB time=00:00:38.10 bitrate= 660.5kbits/s"
    let at = line.find("time=")?;
    let rest = line[at + 5..].trim_start();
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if !rest[end..].trim_start().starts_with("bitrate=") {
        return None;
    }
    let time = parse_time(&rest[..end])?;
    state.encode_time = Some(time);
    let total = state.video_duration?.as_secs_f64();
    if total <= 0.0 {
        return None;
    }
    let percentage = (time.as_secs_f64() / total * 100.0) as f32;
    state.percentage = Some(percentage);
    Some(percentage)
}

impl FFmpegRun {
    /// Start ffmpeg. `frames_on_stdin`: standard input carries raw frames, so stopping
    /// closes it instead of typing `q`.
    pub fn start(
        ffmpeg: &Path,
        args: &[String],
        frames_on_stdin: bool,
        track_progress: bool,
        listener: Arc<dyn FFmpegListener>,
    ) -> std::io::Result<FFmpegRun> {
        let mut child = Command::new(ffmpeg)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        log::info!("ffmpeg started: {} {}", ffmpeg.display(), args.join(" "));
        let stderr = child.stderr.take();
        let state = Arc::new(Mutex::new(FFmpegState::default()));
        let shared = state.clone();
        let reader = stderr.map(|stderr| {
            std::thread::spawn(move || {
                // ffmpeg ends progress lines with '\r'; split on both.
                let mut reader = BufReader::new(stderr);
                let mut line_bytes = Vec::new();
                loop {
                    let available = match reader.fill_buf() {
                        Ok(bytes) if !bytes.is_empty() => bytes.to_vec(),
                        _ => break,
                    };
                    reader.consume(available.len());
                    for byte in available {
                        if byte != b'\n' && byte != b'\r' {
                            line_bytes.push(byte);
                            continue;
                        }
                        if line_bytes.is_empty() {
                            continue;
                        }
                        let line = String::from_utf8_lossy(&line_bytes).into_owned();
                        line_bytes.clear();
                        let (started, progress) = {
                            let mut state = shared.lock().unwrap();
                            state.output.push_str(&line);
                            state.output.push('\n');
                            let started = !state.is_encoding
                                && line.to_ascii_lowercase().contains("press [q] to stop");
                            if started {
                                state.is_encoding = true;
                            }
                            let progress = if track_progress {
                                track_line(&mut state, &line)
                            } else {
                                None
                            };
                            (started, progress)
                        };
                        if started {
                            listener.encode_started();
                        }
                        if let Some(progress) = progress {
                            listener.encode_progress(progress);
                        }
                    }
                }
                if !line_bytes.is_empty() {
                    let mut state = shared.lock().unwrap();
                    state.output.push_str(&String::from_utf8_lossy(&line_bytes));
                    state.output.push('\n');
                }
            })
        });
        Ok(FFmpegRun {
            stdin: child.stdin.take(),
            child,
            state,
            reader,
            close_tries: 0,
            stdin_is_frames: frames_on_stdin,
        })
    }

    /// Standard input, for the frame writer.
    pub fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }

    pub fn state(&self) -> Arc<Mutex<FFmpegState>> {
        self.state.clone()
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// `Close`: ask ffmpeg to finish (end of frames, or `q`), and kill it on the third ask.
    pub fn close(&mut self) {
        if !self.is_running() {
            return;
        }
        if self.close_tries >= 2 {
            let _ = self.child.kill();
            return;
        }
        if self.stdin_is_frames {
            self.stdin = None;
        } else if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(b"q");
            let _ = stdin.flush();
        }
        self.close_tries += 1;
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Wait for the end; true when ffmpeg exited with 0.
    pub fn wait(mut self) -> (bool, String) {
        self.stdin = None;
        let status = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let output = self.state.lock().unwrap().output.clone();
        let ok = status.map(|s| s.success()).unwrap_or(false);
        if !ok {
            log::warn!("ffmpeg failed:\n{output}");
        }
        (ok, output)
    }
}

/// Run ffmpeg to the end with no input of ours.
pub fn run(
    ffmpeg: &Path,
    args: &[String],
    track_progress: bool,
    listener: Arc<dyn FFmpegListener>,
) -> (bool, String) {
    match FFmpegRun::start(ffmpeg, args, false, track_progress, listener) {
        Ok(run) => run.wait(),
        Err(error) => (false, error.to_string()),
    }
}

/// `GetDirectShowDevices` on a Mac: the cameras and microphones ffmpeg's avfoundation
/// input lists. ffmpeg exits with an error after listing (there is no input); the list
/// is read from what it printed. Listing asks macOS for no permission.
pub fn list_devices(ffmpeg: &Path) -> std::io::Result<super::args::Devices> {
    let output = std::process::Command::new(ffmpeg)
        .args(super::args::list_devices_args())
        .stdin(std::process::Stdio::null())
        .output()?;
    let text = String::from_utf8_lossy(&output.stderr);
    if !text.contains("AVFoundation video devices") && !text.contains("AVFoundation audio devices")
    {
        return Err(std::io::Error::other(
            text.lines().last().unwrap_or("").to_string(),
        ));
    }
    Ok(super::args::parse_devices(&text))
}

/// `ConcatenateVideos`.
pub fn concatenate(ffmpeg: &Path, inputs: &[PathBuf], output: &Path, delete_inputs: bool) -> bool {
    let list = PathBuf::from(format!("{}.txt", output.display()));
    let (contents, args) = super::args::concat_args(inputs, &list, output);
    if std::fs::write(&list, contents).is_err() {
        return false;
    }
    let (ok, _) = run(ffmpeg, &args, false, Arc::new(()));
    if ok && delete_inputs {
        for input in inputs {
            let _ = std::fs::remove_file(input);
        }
    }
    let _ = std::fs::remove_file(&list);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_the_time_against_the_duration() {
        let mut state = FFmpegState::default();
        assert_eq!(
            track_line(&mut state, "frame= 1 time=00:00:01.00 bitrate= 1k"),
            None
        );
        track_line(
            &mut state,
            "  Duration: 00:00:10.00, start: 0.000000, bitrate: 1095 kb/s",
        );
        assert_eq!(state.video_duration, Some(Duration::from_secs(10)));
        let p = track_line(
            &mut state,
            "frame=  942 fps=187 q=35.0 size=    3072kB time=00:00:02.50 bitrate= 660.5kbits/s speed=7.55x",
        );
        assert_eq!(p, Some(25.0));
        assert_eq!(
            parse_time("01:02:03.5"),
            Some(Duration::from_secs_f64(3723.5))
        );
        assert_eq!(parse_time("N/A"), None);
    }
}
