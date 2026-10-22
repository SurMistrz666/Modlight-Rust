//! `ScreenRecordingOptions.GetFFmpegCommands` / `GetFFmpegArgs` and the encode commands
//! of `ScreenRecorder`, as argument VECTORS (PORT.md section 5): nothing is ever handed
//! to a shell, and every path ffmpeg reads or writes is absolute, so no file name can be
//! mistaken for an option.
//!
//! The screen input is the platform's: raw BGRA frames (GDI's on Windows,
//! ScreenCaptureKit's on a Mac) on ffmpeg's standard input, in place of upstream's
//! `gdigrab`/`ddagrab` devices. Microphones and cameras are upstream's on Windows,
//! ffmpeg's `dshow` device with upstream's buffer options (`video=`, `audio=`); on a Mac
//! they come through ffmpeg's `avfoundation` device. The hardware encoders get
//! upstream's options on Windows and become VideoToolbox on a Mac.

use std::path::{Path, PathBuf};

use super::options::{
    FFmpegAudioCodec, FFmpegOptions, FFmpegPaletteGenStatsMode, FFmpegPaletteUseDither,
    FFmpegPreset, FFmpegTune, FFmpegVideoCodec, capture_device,
};
use crate::capture::geometry::IRect;

/// `ScreenRecordingOptions`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenRecordingOptions {
    pub is_recording: bool,
    pub is_lossless: bool,
    pub input_path: Option<PathBuf>,
    pub output_path: PathBuf,
    pub fps: i64,
    /// The recorded area of the desktop, in points.
    pub capture_area: IRect,
    /// The size of the frames ScreenCaptureKit delivers for that area, in pixels.
    pub frame_size: (u32, u32),
    /// Seconds; zero records until stopped.
    pub duration: f64,
    pub draw_cursor: bool,
    pub ffmpeg: FFmpegOptions,
}

impl Default for ScreenRecordingOptions {
    fn default() -> Self {
        ScreenRecordingOptions {
            is_recording: false,
            is_lossless: false,
            input_path: None,
            output_path: PathBuf::new(),
            fps: 30,
            capture_area: IRect::default(),
            frame_size: (0, 0),
            duration: 0.0,
            draw_cursor: true,
            ffmpeg: FFmpegOptions::default(),
        }
    }
}

/// Where ffmpeg's frames come from while recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingInput {
    /// Raw frames on standard input (the screen).
    Pipe,
    /// An avfoundation device only (a camera and/or a microphone): standard input is
    /// free, and `q` on it stops the recording as upstream does.
    Devices,
    /// Not recording (an encode of a file).
    File,
}

/// `Path.ChangeExtension`.
pub fn change_extension(path: &Path, extension: &str) -> PathBuf {
    path.with_extension(extension)
}

/// `$duration$` and `-t` use one decimal and the invariant culture.
fn duration_text(seconds: f64) -> String {
    format!("{seconds:.1}")
}

/// Split a command line the way upstream's string reached `Process.Start`
/// (`CommandLineToArgvW`): whitespace separates, double quotes group, `\"` is a quote.
pub fn split_command_line(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                // Backslashes are literal unless they precede a quote.
                let mut count = 0;
                while i < chars.len() && chars[i] == '\\' {
                    count += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    current.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        current.push('"');
                        i += 1;
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', count));
                }
                has_token = true;
                continue;
            }
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
        i += 1;
    }
    if has_token {
        args.push(current);
    }
    args
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|s| s.to_string()));
}

impl ScreenRecordingOptions {
    /// Which input this recording reads.
    pub fn input(&self) -> RecordingInput {
        if !self.is_recording {
            RecordingInput::File
        } else if self.ffmpeg.is_video_source_selected()
            && capture_device::is_screen(&self.ffmpeg.VideoSource)
        {
            RecordingInput::Pipe
        } else {
            RecordingInput::Devices
        }
    }

    /// The input arguments for the screen pipe.
    fn pipe_input(&self, args: &mut Vec<String>, framerate: &str) {
        let (w, h) = self.frame_size;
        push(args, &["-f", "rawvideo", "-pix_fmt", "bgra"]);
        args.push("-video_size".into());
        args.push(format!("{w}x{h}"));
        args.push("-framerate".into());
        args.push(framerate.into());
        push(args, &["-thread_queue_size", "1024", "-i", "pipe:0"]);
    }

    /// `AppendInputDevice(args, "dshow", audio)`: the queue, a 256 MB real-time buffer
    /// and, for a microphone, an 80 ms device buffer.
    #[cfg(windows)]
    fn device_input(args: &mut Vec<String>, audio: bool) {
        push(
            args,
            &[
                "-f",
                "dshow",
                "-thread_queue_size",
                "1024",
                "-rtbufsize",
                "256M",
            ],
        );
        if audio {
            push(args, &["-audio_buffer_size", "80"]);
        }
    }

    /// `AppendInputDevice` for avfoundation.
    #[cfg(not(windows))]
    fn device_input(args: &mut Vec<String>, audio: bool) {
        push(args, &["-f", "avfoundation", "-thread_queue_size", "1024"]);
        if audio {
            // The counterpart of dshow's -audio_buffer_size: avfoundation has none; the
            // queue size above is what keeps audio from dropping.
        }
    }

    /// The input name of a microphone alone: `audio=<name>` (dshow), `:<name>`
    /// (avfoundation). One argument whatever the name holds.
    fn audio_device(name: &str) -> String {
        if cfg!(windows) {
            format!("audio={name}")
        } else {
            format!(":{name}")
        }
    }

    /// The input name of a camera, with a microphone or without: `video=<camera>` and
    /// `:audio=<microphone>` (dshow), `<camera>:<microphone>` or `<camera>:none`
    /// (avfoundation).
    fn camera_device(camera: &str, microphone: Option<&str>) -> String {
        if cfg!(windows) {
            match microphone {
                Some(microphone) => format!("video={camera}:audio={microphone}"),
                None => format!("video={camera}"),
            }
        } else {
            format!("{camera}:{}", microphone.unwrap_or("none"))
        }
    }

    /// `GetFFmpegCommands`.
    pub fn ffmpeg_commands(&self) -> Option<Vec<String>> {
        if !self.is_lossless
            && self.ffmpeg.UseCustomCommands
            && !self.ffmpeg.CustomCommands.trim().is_empty()
        {
            let output = change_extension(&self.output_path, self.ffmpeg.extension());
            let text =
                replace_ignore_case(&self.ffmpeg.CustomCommands, "$fps$", &self.fps.to_string());
            let text = replace_ignore_case(&text, "$area_x$", &self.capture_area.x.to_string());
            let text = replace_ignore_case(&text, "$area_y$", &self.capture_area.y.to_string());
            let text =
                replace_ignore_case(&text, "$area_width$", &self.capture_area.width.to_string());
            let text = replace_ignore_case(
                &text,
                "$area_height$",
                &self.capture_area.height.to_string(),
            );
            let text =
                replace_ignore_case(&text, "$cursor$", if self.draw_cursor { "1" } else { "0" });
            let text = replace_ignore_case(&text, "$duration$", &duration_text(self.duration));
            // The output is one argument whatever it contains, so it is put in place
            // after the split rather than spliced into the text.
            let mut args = Vec::new();
            if self.input() == RecordingInput::Pipe {
                self.pipe_input(&mut args, &self.fps.to_string());
            }
            for arg in split_command_line(&text) {
                if arg.eq_ignore_ascii_case("$output$") {
                    args.push(output.to_string_lossy().into_owned());
                } else {
                    args.push(replace_ignore_case(
                        &arg,
                        "$output$",
                        &output.to_string_lossy(),
                    ));
                }
            }
            return Some(args);
        }
        self.ffmpeg_args()
    }

    /// `GetFFmpegArgs`.
    pub fn ffmpeg_args(&self) -> Option<Vec<String>> {
        self.build_args(false)
    }

    /// `GetFFmpegArgs()` as the text the options window shows, and `GetFFmpegArgs(true)`,
    /// the template "Use custom commands" starts from: `$fps$`, `$duration$` and
    /// `$output$` in place of the values. The screen's pipe input is not part of a
    /// template (a custom command always gets it first), so the template starts after it.
    pub fn ffmpeg_args_text(&self, custom: bool) -> Option<String> {
        self.build_args(custom).map(|args| join_command_line(&args))
    }

    fn build_args(&self, custom: bool) -> Option<Vec<String>> {
        let ffmpeg = &self.ffmpeg;
        if self.is_recording
            && !ffmpeg.is_video_source_selected()
            && !ffmpeg.is_audio_source_selected()
        {
            return None;
        }
        let mut args = Vec::new();
        let framerate = if custom {
            "$fps$".to_string()
        } else {
            self.fps.to_string()
        };

        if self.is_recording {
            if ffmpeg.is_video_source_selected() {
                if capture_device::is_screen(&ffmpeg.VideoSource) {
                    if !custom {
                        self.pipe_input(&mut args, &framerate);
                    }
                    if ffmpeg.is_audio_source_selected() {
                        Self::device_input(&mut args, true);
                        args.push("-i".into());
                        args.push(Self::audio_device(&ffmpeg.AudioSource));
                    }
                } else {
                    // A camera: one device input carrying video and audio.
                    Self::device_input(&mut args, ffmpeg.is_audio_source_selected());
                    args.push("-framerate".into());
                    args.push(framerate.clone());
                    args.push("-i".into());
                    let microphone = ffmpeg
                        .is_audio_source_selected()
                        .then_some(ffmpeg.AudioSource.as_str());
                    args.push(Self::camera_device(&ffmpeg.VideoSource, microphone));
                }
            } else if ffmpeg.is_audio_source_selected() {
                Self::device_input(&mut args, true);
                args.push("-i".into());
                args.push(Self::audio_device(&ffmpeg.AudioSource));
            }
        } else {
            args.push("-i".into());
            args.push(
                self.input_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }

        if !ffmpeg.UserArgs.trim().is_empty() {
            args.extend(split_command_line(&ffmpeg.UserArgs));
        }

        if ffmpeg.is_video_source_selected() {
            let codec = ffmpeg.VideoCodec;
            if self.is_lossless || codec != FFmpegVideoCodec::apng {
                let encoder = if self.is_lossless {
                    FFmpegVideoCodec::libx264.encoder()
                } else {
                    codec.encoder()
                };
                push(&mut args, &["-c:v", encoder]);
                args.push("-r".into());
                args.push(framerate.clone());
            }

            if self.is_lossless {
                push(
                    &mut args,
                    &[
                        "-preset",
                        FFmpegPreset::ultrafast.name(),
                        "-tune",
                        FFmpegTune::zerolatency.name(),
                        "-qp",
                        "0",
                    ],
                );
            } else {
                match codec {
                    FFmpegVideoCodec::libx264 | FFmpegVideoCodec::libx265 => {
                        push(&mut args, &["-preset", ffmpeg.x264_Preset.name()]);
                        if self.is_recording {
                            push(&mut args, &["-tune", FFmpegTune::zerolatency.name()]);
                        }
                        if ffmpeg.x264_Use_Bitrate {
                            args.push("-b:v".into());
                            args.push(format!("{}k", ffmpeg.x264_Bitrate));
                        } else {
                            args.push("-crf".into());
                            args.push(ffmpeg.x264_CRF.to_string());
                        }
                        push(
                            &mut args,
                            &["-pix_fmt", "yuv420p", "-movflags", "+faststart"],
                        );
                    }
                    FFmpegVideoCodec::libvpx | FFmpegVideoCodec::libvpx_vp9 => {
                        if self.is_recording {
                            push(&mut args, &["-deadline", "realtime"]);
                        }
                        args.push("-b:v".into());
                        args.push(format!("{}k", ffmpeg.VPx_Bitrate));
                        push(&mut args, &["-pix_fmt", "yuv420p"]);
                    }
                    FFmpegVideoCodec::libxvid => {
                        args.push("-qscale:v".into());
                        args.push(ffmpeg.XviD_QScale.to_string());
                    }
                    FFmpegVideoCodec::h264_nvenc | FFmpegVideoCodec::hevc_nvenc => {
                        // NVENC is Windows hardware; on a Mac VideoToolbox takes its
                        // bitrate and nothing else.
                        if cfg!(windows) {
                            push(&mut args, &["-preset", ffmpeg.NVENC_Preset.name()]);
                            push(&mut args, &["-tune", ffmpeg.NVENC_Tune.name()]);
                        }
                        args.push("-b:v".into());
                        args.push(format!("{}k", ffmpeg.NVENC_Bitrate));
                        push(&mut args, &["-movflags", "+faststart"]);
                    }
                    FFmpegVideoCodec::h264_amf | FFmpegVideoCodec::hevc_amf => {
                        if cfg!(windows) {
                            push(&mut args, &["-usage", ffmpeg.AMF_Usage.name()]);
                            push(&mut args, &["-quality", ffmpeg.AMF_Quality.name()]);
                        }
                        args.push("-b:v".into());
                        args.push(format!("{}k", ffmpeg.AMF_Bitrate));
                        push(&mut args, &["-pix_fmt", "yuv420p"]);
                    }
                    FFmpegVideoCodec::h264_qsv | FFmpegVideoCodec::hevc_qsv => {
                        if cfg!(windows) {
                            push(&mut args, &["-preset", ffmpeg.QSV_Preset.name()]);
                        }
                        args.push("-b:v".into());
                        args.push(format!("{}k", ffmpeg.QSV_Bitrate));
                    }
                    FFmpegVideoCodec::libwebp => {
                        push(
                            &mut args,
                            &["-lossless", "0", "-preset", "default", "-loop", "0"],
                        );
                    }
                    FFmpegVideoCodec::apng => {
                        push(&mut args, &["-f", "apng", "-plays", "0"]);
                    }
                    FFmpegVideoCodec::gif => {}
                }
                if matches!(
                    codec,
                    FFmpegVideoCodec::libx265
                        | FFmpegVideoCodec::hevc_nvenc
                        | FFmpegVideoCodec::hevc_amf
                        | FFmpegVideoCodec::hevc_qsv
                ) {
                    push(&mut args, &["-tag:v", "hvc1"]);
                }
            }
        }

        if ffmpeg.is_audio_source_selected() {
            match ffmpeg.AudioCodec {
                FFmpegAudioCodec::libvoaacenc => {
                    push(&mut args, &["-c:a", "aac", "-ac", "2", "-b:a"]);
                    args.push(format!("{}k", ffmpeg.AAC_Bitrate));
                }
                FFmpegAudioCodec::libopus => {
                    push(&mut args, &["-c:a", "libopus", "-b:a"]);
                    args.push(format!("{}k", ffmpeg.Opus_Bitrate));
                }
                FFmpegAudioCodec::libvorbis => {
                    push(&mut args, &["-c:a", "libvorbis", "-qscale:a"]);
                    args.push(ffmpeg.Vorbis_QScale.to_string());
                }
                FFmpegAudioCodec::libmp3lame => {
                    push(&mut args, &["-c:a", "libmp3lame", "-qscale:a"]);
                    args.push(ffmpeg.MP3_QScale.to_string());
                }
            }
        }

        if self.duration > 0.0 {
            args.push("-t".into());
            args.push(if custom {
                "$duration$".into()
            } else {
                duration_text(self.duration)
            });
        }

        args.push("-y".into());
        let extension = if self.is_lossless {
            "mp4"
        } else {
            ffmpeg.extension()
        };
        args.push(if custom {
            "$output$".into()
        } else {
            change_extension(&self.output_path, extension)
                .to_string_lossy()
                .into_owned()
        });
        Some(args)
    }
}

/// An argument vector as one line that [`split_command_line`] reads back: arguments
/// with spaces, quotes or nothing in them are quoted (`Helpers.EscapeCLIText`).
pub fn join_command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if !arg.is_empty() && !arg.chars().any(|c| c.is_whitespace() || c == '"') {
                return arg.clone();
            }
            let mut quoted = String::from("\"");
            let mut backslashes = 0;
            for c in arg.chars() {
                match c {
                    '\\' => backslashes += 1,
                    '"' => {
                        quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                        quoted.push('"');
                        backslashes = 0;
                    }
                    c => {
                        quoted.extend(std::iter::repeat_n('\\', backslashes));
                        quoted.push(c);
                        backslashes = 0;
                    }
                }
            }
            quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
            quoted.push('"');
            quoted
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `ScreenRecorder.FFmpegEncodeAsGIF`: the two-filter palette pass.
pub fn gif_encode_args(input: &Path, output: &Path, options: &FFmpegOptions) -> Vec<String> {
    let mut lavfi = format!(
        "palettegen=stats_mode={}[palette],[0:v][palette]paletteuse=dither={}",
        options.GIFStatsMode.name(),
        options.GIFDither.name()
    );
    if options.GIFDither == FFmpegPaletteUseDither::bayer {
        lavfi.push_str(&format!(":bayer_scale={}", options.GIFBayerScale));
    }
    if options.GIFStatsMode == FFmpegPaletteGenStatsMode::single {
        lavfi.push_str(":new=1");
    }
    vec![
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-lavfi".into(),
        lavfi,
        "-y".into(),
        output.to_string_lossy().into_owned(),
    ]
}

/// `FFmpegCLIManager.ConcatenateVideos`: the list file's text and the arguments.
pub fn concat_args(inputs: &[PathBuf], list_file: &Path, output: &Path) -> (String, Vec<String>) {
    let contents = inputs
        .iter()
        .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join("\n");
    let args = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_file.to_string_lossy().into_owned(),
        "-c".into(),
        "copy".into(),
        output.to_string_lossy().into_owned(),
    ];
    (contents, args)
}

/// The device list command: upstream's `-list_devices true -f dshow -i dummy` on
/// Windows (`GetDirectShowDevices`), the avfoundation listing on a Mac.
pub fn list_devices_args() -> Vec<String> {
    let args: &[&str] = if cfg!(windows) {
        &["-list_devices", "true", "-f", "dshow", "-i", "dummy"]
    } else {
        &[
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ]
    };
    args.iter().map(|s| s.to_string()).collect()
}

/// Cameras and microphones, read from the device list's output. On a Mac the "Capture
/// screen" entries are left out: the screen is recorded through ScreenCaptureKit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Devices {
    pub video: Vec<String>,
    pub audio: Vec<String>,
}

/// `GetDirectShowDevices`' reading of ffmpeg's output: a `[dshow @ ...]` or `[in#0 @
/// ...]` line with a quoted name is a device; the section headers of older ffmpeg
/// builds ("DirectShow video devices", "DirectShow audio devices") and the "(video)" or
/// "(audio)" a newer one prints after the name say which kind. An "Alternative name"
/// line is not a device.
#[cfg(windows)]
pub fn parse_devices(output: &str) -> Devices {
    let mut devices = Devices::default();
    let mut audio = false;
    for line in output.lines() {
        if line.contains("] DirectShow video devices") {
            audio = false;
            continue;
        }
        if line.contains("] DirectShow audio devices") {
            audio = true;
            continue;
        }
        // \[(?:dshow|in#0) @ \w+\] +"(.+)"
        let Some(start) = line.find("[dshow @ ").or_else(|| line.find("[in#0 @ ")) else {
            continue;
        };
        let rest = &line[start..];
        let Some(close) = rest.find(']') else {
            continue;
        };
        let address = rest[..close].rsplit(' ').next().unwrap_or("");
        if address.is_empty() || !address.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let after = &rest[close + 1..];
        let trimmed = after.trim_start_matches(' ');
        if trimmed.len() == after.len() || !trimmed.starts_with('"') {
            continue;
        }
        // `(.+)"`: greedy, up to the last quote of the line.
        let Some(end) = trimmed.rfind('"').filter(|end| *end > 1) else {
            continue;
        };
        let name = trimmed[1..end].to_string();
        if line.ends_with("\" (video)") {
            audio = false;
        } else if line.ends_with("\" (audio)") {
            audio = true;
        }
        if audio {
            devices.audio.push(name);
        } else {
            devices.video.push(name);
        }
    }
    devices
}

#[cfg(not(windows))]
pub fn parse_devices(output: &str) -> Devices {
    let mut devices = Devices::default();
    let mut audio = false;
    for line in output.lines() {
        if line.contains("AVFoundation video devices") {
            audio = false;
            continue;
        }
        if line.contains("AVFoundation audio devices") {
            audio = true;
            continue;
        }
        // "[AVFoundation indev @ 0x...] [0] FaceTime HD Camera"
        let Some(rest) = line.split_once("] [").map(|(_, rest)| rest) else {
            continue;
        };
        let Some((index, name)) = rest.split_once("] ") else {
            continue;
        };
        if index.parse::<u32>().is_err() {
            continue;
        }
        let name = name.trim().to_string();
        if audio {
            devices.audio.push(name);
        } else if !name.starts_with("Capture screen") {
            devices.video.push(name);
        }
    }
    devices
}

fn replace_ignore_case(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let lower = text.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut start = 0;
    while let Some(found) = lower[start..].find(&needle) {
        let at = start + found;
        out.push_str(&text[last..at]);
        out.push_str(to);
        last = at + from.len();
        start = last;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> ScreenRecordingOptions {
        ScreenRecordingOptions {
            is_recording: true,
            output_path: PathBuf::from("/Users/q/Screenshots/2026-10/clip.mp4"),
            fps: 30,
            capture_area: IRect::new(100, 50, 640, 360),
            frame_size: (1280, 720),
            ..ScreenRecordingOptions::default()
        }
    }

    #[test]
    fn a_screen_recording_reads_frames_from_the_pipe() {
        let args = recording().ffmpeg_commands().unwrap();
        assert_eq!(
            args,
            [
                "-f",
                "rawvideo",
                "-pix_fmt",
                "bgra",
                "-video_size",
                "1280x720",
                "-framerate",
                "30",
                "-thread_queue_size",
                "1024",
                "-i",
                "pipe:0",
                "-c:v",
                "libx264",
                "-r",
                "30",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-crf",
                "28",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "-y",
                "/Users/q/Screenshots/2026-10/clip.mp4"
            ]
        );
        assert_eq!(recording().input(), RecordingInput::Pipe);
    }

    #[test]
    fn the_custom_template_has_placeholders_and_no_pipe() {
        let mut options = recording();
        options.duration = 5.0;
        let template = options.ffmpeg_args_text(true).unwrap();
        assert_eq!(
            template,
            "-c:v libx264 -r $fps$ -preset ultrafast -tune zerolatency -crf 28 -pix_fmt yuv420p -movflags +faststart -t $duration$ -y $output$"
        );
        // Used as custom commands, the template records the same as the defaults.
        options.ffmpeg.UseCustomCommands = true;
        options.ffmpeg.CustomCommands = template;
        let custom = options.ffmpeg_commands().unwrap();
        options.ffmpeg.UseCustomCommands = false;
        assert_eq!(custom, options.ffmpeg_args().unwrap());
    }

    #[test]
    fn a_joined_command_line_splits_back() {
        let args: Vec<String> = [
            "-i",
            ":MacBook Pro Microphone",
            "",
            "a\\\"b",
            "C:\\x\\",
            "plain",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let text = join_command_line(&args);
        assert!(text.starts_with("-i \":MacBook Pro Microphone\" \"\""));
        assert_eq!(split_command_line(&text), args);
    }

    #[cfg(not(windows))]
    #[test]
    fn microphone_bitrate_duration_and_hevc_tag() {
        let mut options = recording();
        options.ffmpeg.AudioSource = "MacBook Pro Microphone".into();
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::libx265;
        options.ffmpeg.x264_Use_Bitrate = true;
        options.ffmpeg.x264_Bitrate = 5000;
        options.duration = 3.0;
        let args = options.ffmpeg_args().unwrap();
        let text = args.join(" ");
        assert!(text.contains(
            "-i pipe:0 -f avfoundation -thread_queue_size 1024 -i :MacBook Pro Microphone"
        ));
        assert!(text.contains("-c:v libx265"));
        assert!(text.contains("-b:v 5000k"));
        assert!(!text.contains("-crf"));
        assert!(text.contains("-tag:v hvc1"));
        assert!(text.contains("-c:a aac -ac 2 -b:a 128k"));
        assert!(text.ends_with("-t 3.0 -y /Users/q/Screenshots/2026-10/clip.mp4"));
        // The device name is ONE argument, spaces and all.
        assert!(args.iter().any(|a| a == ":MacBook Pro Microphone"));
    }

    #[test]
    fn lossless_first_pass_then_gif_palette() {
        let mut options = recording();
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::gif;
        options.is_lossless = true;
        options.output_path = PathBuf::from("/tmp/x/clip.gif");
        let args = options.ffmpeg_args().unwrap();
        let text = args.join(" ");
        assert!(text.contains("-c:v libx264 -r 30 -preset ultrafast -tune zerolatency -qp 0"));
        assert!(text.ends_with("-y /tmp/x/clip.mp4"));
        let gif = gif_encode_args(
            Path::new("/tmp/x/clip.mp4"),
            Path::new("/tmp/x/clip.gif"),
            &options.ffmpeg,
        );
        assert_eq!(
            gif,
            [
                "-i",
                "/tmp/x/clip.mp4",
                "-lavfi",
                "palettegen=stats_mode=full[palette],[0:v][palette]paletteuse=dither=sierra2_4a",
                "-y",
                "/tmp/x/clip.gif"
            ]
        );
        options.ffmpeg.GIFDither = FFmpegPaletteUseDither::bayer;
        options.ffmpeg.GIFStatsMode = FFmpegPaletteGenStatsMode::single;
        let gif = gif_encode_args(Path::new("/a.mp4"), Path::new("/a.gif"), &options.ffmpeg);
        assert!(gif[3].ends_with("dither=bayer:bayer_scale=2:new=1"));
    }

    #[test]
    fn encoding_a_file_and_the_codecs_sections() {
        let mut options = ScreenRecordingOptions {
            input_path: Some(PathBuf::from("/in/a b.mp4")),
            output_path: PathBuf::from("/out/a b.mp4"),
            ..ScreenRecordingOptions::default()
        };
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::libvpx_vp9;
        let args = options.ffmpeg_args().unwrap();
        assert_eq!(&args[..2], ["-i", "/in/a b.mp4"]);
        let text = args.join(" ");
        assert!(text.contains("-c:v libvpx-vp9 -r 30 -b:v 3000k -pix_fmt yuv420p"));
        assert!(!text.contains("realtime"), "only a recording is real time");
        assert!(text.ends_with("/out/a b.webm"));
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::apng;
        let text = options.ffmpeg_args().unwrap().join(" ");
        assert!(!text.contains("-c:v"));
        assert!(text.contains("-f apng -plays 0"));
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::h264_nvenc;
        let text = options.ffmpeg_args().unwrap().join(" ");
        #[cfg(not(windows))]
        assert!(text.contains("-c:v h264_videotoolbox -r 30 -b:v 3000k -movflags +faststart"));
        #[cfg(windows)]
        assert!(
            text.contains(
                "-c:v h264_nvenc -r 30 -preset p4 -tune ll -b:v 3000k -movflags +faststart"
            )
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn no_source_records_nothing() {
        let mut options = recording();
        options.ffmpeg.VideoSource.clear();
        options.ffmpeg.AudioSource.clear();
        assert!(options.ffmpeg_args().is_none());
        options.ffmpeg.AudioSource = "Mic".into();
        let args = options.ffmpeg_args().unwrap();
        assert_eq!(
            &args[..6],
            [
                "-f",
                "avfoundation",
                "-thread_queue_size",
                "1024",
                "-i",
                ":Mic"
            ]
        );
        assert_eq!(options.input(), RecordingInput::Devices);
        assert!(args.last().unwrap().ends_with("clip.m4a"));
    }

    #[test]
    fn user_args_and_custom_commands_are_split_not_shelled() {
        let mut options = recording();
        options.ffmpeg.UserArgs = r#"-vf "scale=640:-2, fps=30" -metadata title="a \"b\"""#.into();
        let args = options.ffmpeg_args().unwrap();
        assert!(args.iter().any(|a| a == "scale=640:-2, fps=30"));
        assert!(args.iter().any(|a| a == r#"title=a "b""#));
        options.ffmpeg.UseCustomCommands = true;
        options.ffmpeg.CustomCommands =
            "-c:v libx264 -r $FPS$ -t $duration$ -vf crop=$area_width$:$area_height$ -y $output$; rm -rf /".into();
        options.duration = 2.5;
        let args = options.ffmpeg_commands().unwrap();
        assert_eq!(&args[..12][10..], ["-i", "pipe:0"]);
        let text = args.join(" ");
        assert!(
            text.contains(
                "-r 30 -t 2.5 -vf crop=640:360 -y /Users/q/Screenshots/2026-10/clip.mp4;"
            )
        );
        // Nothing runs a shell: "rm" is just another argument ffmpeg will refuse.
        assert!(args.iter().any(|a| a == "rm"));
        options.is_lossless = true;
        assert!(
            !options
                .ffmpeg_commands()
                .unwrap()
                .join(" ")
                .contains("crop")
        );
    }

    #[test]
    fn command_lines_split_like_windows() {
        assert_eq!(split_command_line(r#"a "b c" d"#), ["a", "b c", "d"]);
        assert_eq!(split_command_line(r#""" x"#), ["", "x"]);
        assert_eq!(
            split_command_line(r#"C:\path\ "q\"uote""#),
            [r"C:\path\", "q\"uote"]
        );
        assert_eq!(split_command_line("   "), Vec::<String>::new());
    }

    #[cfg(not(windows))]
    #[test]
    fn concat_list_and_devices() {
        let (contents, args) = concat_args(
            &[
                PathBuf::from("/a/x-concat.mp4"),
                PathBuf::from("/a/it's.mp4"),
            ],
            Path::new("/a/x-temp.mp4.txt"),
            Path::new("/a/x-temp.mp4"),
        );
        assert_eq!(contents, "file '/a/x-concat.mp4'\nfile '/a/it'\\''s.mp4'");
        assert_eq!(
            args[..6],
            ["-f", "concat", "-safe", "0", "-i", "/a/x-temp.mp4.txt"]
        );
        let output = "[AVFoundation indev @ 0x1] AVFoundation video devices:\n\
            [AVFoundation indev @ 0x1] [0] FaceTime HD Camera\n\
            [AVFoundation indev @ 0x1] [1] Capture screen 0\n\
            [AVFoundation indev @ 0x1] AVFoundation audio devices:\n\
            [AVFoundation indev @ 0x1] [0] MacBook Pro Microphone\n\
            [in#0 @ 0x2] Error opening input: Input/output error";
        let devices = parse_devices(output);
        assert_eq!(devices.video, ["FaceTime HD Camera"]);
        assert_eq!(devices.audio, ["MacBook Pro Microphone"]);
    }

    #[cfg(windows)]
    #[test]
    fn microphone_through_dshow_with_upstreams_buffers() {
        let mut options = recording();
        options.ffmpeg.AudioSource = "Microphone (Realtek(R) Audio)".into();
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::libx265;
        options.ffmpeg.x264_Use_Bitrate = true;
        options.ffmpeg.x264_Bitrate = 5000;
        options.duration = 3.0;
        let args = options.ffmpeg_args().unwrap();
        let text = args.join(" ");
        assert!(text.contains(
            "-i pipe:0 -f dshow -thread_queue_size 1024 -rtbufsize 256M -audio_buffer_size 80 -i audio=Microphone (Realtek(R) Audio)"
        ));
        assert!(text.contains("-c:v libx265"));
        assert!(text.contains("-b:v 5000k"));
        assert!(text.contains("-tag:v hvc1"));
        assert!(text.contains("-c:a aac -ac 2 -b:a 128k"));
        // The device name is ONE argument, spaces and parentheses included.
        assert!(
            args.iter()
                .any(|a| a == "audio=Microphone (Realtek(R) Audio)")
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_camera_and_a_microphone_are_one_dshow_input() {
        let mut options = recording();
        options.ffmpeg.VideoSource = "Integrated Camera".into();
        options.ffmpeg.AudioSource = "Microphone Array".into();
        let args = options.ffmpeg_args().unwrap();
        assert_eq!(options.input(), RecordingInput::Devices);
        assert_eq!(
            &args[..12],
            [
                "-f",
                "dshow",
                "-thread_queue_size",
                "1024",
                "-rtbufsize",
                "256M",
                "-audio_buffer_size",
                "80",
                "-framerate",
                "30",
                "-i",
                "video=Integrated Camera:audio=Microphone Array"
            ]
        );
        options.ffmpeg.AudioSource.clear();
        let args = options.ffmpeg_args().unwrap();
        assert_eq!(
            &args[..10],
            [
                "-f",
                "dshow",
                "-thread_queue_size",
                "1024",
                "-rtbufsize",
                "256M",
                "-framerate",
                "30",
                "-i",
                "video=Integrated Camera"
            ]
        );
        // A microphone alone.
        options.ffmpeg.VideoSource.clear();
        options.ffmpeg.AudioSource = "Mic".into();
        let args = options.ffmpeg_args().unwrap();
        assert_eq!(args[8..10], ["-i", "audio=Mic"]);
        assert!(args.last().unwrap().ends_with("clip.m4a"));
    }

    #[cfg(windows)]
    #[test]
    fn hardware_encoders_get_upstreams_options() {
        let mut options = recording();
        let line = |options: &ScreenRecordingOptions| {
            let args = options.ffmpeg_args().unwrap();
            let start = args.iter().position(|a| a == "-c:v").unwrap();
            let end = args.iter().position(|a| a == "-y").unwrap();
            args[start..end].join(" ")
        };
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::hevc_nvenc;
        options.ffmpeg.NVENC_Preset = super::super::options::FFmpegNVENCPreset::p6;
        options.ffmpeg.NVENC_Tune = super::super::options::FFmpegNVENCTune::hq;
        assert_eq!(
            line(&options),
            "-c:v hevc_nvenc -r 30 -preset p6 -tune hq -b:v 3000k -movflags +faststart -tag:v hvc1"
        );
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::h264_amf;
        assert_eq!(
            line(&options),
            "-c:v h264_amf -r 30 -usage lowlatency -quality speed -b:v 3000k -pix_fmt yuv420p"
        );
        options.ffmpeg.VideoCodec = FFmpegVideoCodec::hevc_qsv;
        options.ffmpeg.QSV_Bitrate = 8000;
        assert_eq!(
            line(&options),
            "-c:v hevc_qsv -r 30 -preset fast -b:v 8000k -tag:v hvc1"
        );
    }

    #[cfg(windows)]
    #[test]
    fn directshow_devices_are_read_like_upstream() {
        assert_eq!(
            list_devices_args(),
            ["-list_devices", "true", "-f", "dshow", "-i", "dummy"]
        );
        // ffmpeg 5 and later: the kind after the name, an alternative name per device.
        let newer = "[dshow @ 000001c4c1b0e5c0] \"Integrated Camera\" (video)\r\n\
            [dshow @ 000001c4c1b0e5c0]   Alternative name \"@device_pnp_\\\\?\\usb#vid_04f2\"\r\n\
            [dshow @ 000001c4c1b0e5c0] \"OBS Virtual Camera\" (none)\r\n\
            [dshow @ 000001c4c1b0e5c0] \"Microphone Array (Realtek(R) Audio)\" (audio)\r\n\
            [dshow @ 000001c4c1b0e5c0]   Alternative name \"@device_cm_{33D9A762}\\wave_{A1B2}\"\r\n\
            [in#0 @ 000001c4c1b0d2c0] Error opening input: Immediate exit requested\r\n\
            dummy: Immediate exit requested\r\n";
        let devices = parse_devices(newer);
        assert_eq!(devices.video, ["Integrated Camera", "OBS Virtual Camera"]);
        assert_eq!(devices.audio, ["Microphone Array (Realtek(R) Audio)"]);
        // Older builds: section headers.
        let older = "[dshow @ 0000020f] DirectShow video devices (some may be both video and audio devices)\n\
            [dshow @ 0000020f]  \"HD Webcam\"\n\
            [dshow @ 0000020f]     Alternative name \"@device_pnp_x\"\n\
            [dshow @ 0000020f] DirectShow audio devices\n\
            [dshow @ 0000020f]  \"Line In (USB)\"\n";
        let devices = parse_devices(older);
        assert_eq!(devices.video, ["HD Webcam"]);
        assert_eq!(devices.audio, ["Line In (USB)"]);
        assert_eq!(parse_devices("ffmpeg version 8.1"), Devices::default());
    }
}
