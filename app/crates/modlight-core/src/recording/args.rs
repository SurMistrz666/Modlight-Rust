//! `ScreenRecordingOptions.GetFFmpegCommands` / `GetFFmpegArgs` and the encode commands
//! of `ScreenRecorder`, as argument VECTORS (PORT.md section 5): nothing is ever handed
//! to a shell, and every path ffmpeg reads or writes is absolute, so no file name can be
//! mistaken for an option.
//!
//! The screen input is the Mac's own (MACOS.md): raw BGRA frames from ScreenCaptureKit
//! on ffmpeg's standard input, in place of upstream's `gdigrab`/`ddagrab` devices.
//! Microphones and cameras come through ffmpeg's `avfoundation` device, in place of
//! `dshow`.

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

    /// `AppendInputDevice` for avfoundation.
    fn device_input(args: &mut Vec<String>, audio: bool) {
        push(args, &["-f", "avfoundation", "-thread_queue_size", "1024"]);
        if audio {
            // The counterpart of dshow's -audio_buffer_size: avfoundation has none; the
            // queue size above is what keeps audio from dropping.
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
        let ffmpeg = &self.ffmpeg;
        if self.is_recording
            && !ffmpeg.is_video_source_selected()
            && !ffmpeg.is_audio_source_selected()
        {
            return None;
        }
        let mut args = Vec::new();
        let framerate = self.fps.to_string();

        if self.is_recording {
            if ffmpeg.is_video_source_selected() {
                if capture_device::is_screen(&ffmpeg.VideoSource) {
                    self.pipe_input(&mut args, &framerate);
                    if ffmpeg.is_audio_source_selected() {
                        Self::device_input(&mut args, true);
                        args.push("-i".into());
                        args.push(format!(":{}", ffmpeg.AudioSource));
                    }
                } else {
                    // A camera: one avfoundation input carrying video and audio.
                    Self::device_input(&mut args, ffmpeg.is_audio_source_selected());
                    args.push("-framerate".into());
                    args.push(framerate.clone());
                    args.push("-i".into());
                    if ffmpeg.is_audio_source_selected() {
                        args.push(format!("{}:{}", ffmpeg.VideoSource, ffmpeg.AudioSource));
                    } else {
                        args.push(format!("{}:none", ffmpeg.VideoSource));
                    }
                }
            } else if ffmpeg.is_audio_source_selected() {
                Self::device_input(&mut args, true);
                args.push("-i".into());
                args.push(format!(":{}", ffmpeg.AudioSource));
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
                        // NVENC is Windows hardware; VideoToolbox takes its bitrate.
                        args.push("-b:v".into());
                        args.push(format!("{}k", ffmpeg.NVENC_Bitrate));
                        push(&mut args, &["-movflags", "+faststart"]);
                    }
                    FFmpegVideoCodec::h264_amf | FFmpegVideoCodec::hevc_amf => {
                        args.push("-b:v".into());
                        args.push(format!("{}k", ffmpeg.AMF_Bitrate));
                        push(&mut args, &["-pix_fmt", "yuv420p"]);
                    }
                    FFmpegVideoCodec::h264_qsv | FFmpegVideoCodec::hevc_qsv => {
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
            args.push(duration_text(self.duration));
        }

        args.push("-y".into());
        let extension = if self.is_lossless {
            "mp4"
        } else {
            ffmpeg.extension()
        };
        args.push(
            change_extension(&self.output_path, extension)
                .to_string_lossy()
                .into_owned(),
        );
        Some(args)
    }
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

/// The device list command (`GetDirectShowDevices` for avfoundation).
pub fn list_devices_args() -> Vec<String> {
    [
        "-hide_banner",
        "-f",
        "avfoundation",
        "-list_devices",
        "true",
        "-i",
        "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Cameras and microphones, read from the device list's output. The "Capture screen"
/// entries are left out: the screen is recorded through ScreenCaptureKit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Devices {
    pub video: Vec<String>,
    pub audio: Vec<String>,
}

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
        assert!(text.contains("-c:v h264_videotoolbox -r 30 -b:v 3000k -movflags +faststart"));
    }

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
}
