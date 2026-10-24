//! The video converter (`VideoConverterViewModel`, `VideoConverterOptions`,
//! `TaskHelpers.RunVideoConversionAsync`): which controls apply to an encoder, the output
//! path, and ffmpeg's arguments, as a vector (nothing passes through a shell).
//!
//! NVENC, AMF and Quick Sync are Windows hardware encoders. On Windows the list offers
//! them as upstream does, with upstream's arguments. As in the screen recorder, the list
//! does not offer them on a Mac; a value synced from Windows that names one is encoded
//! with VideoToolbox and keeps only its bitrate.

use std::path::{Path, PathBuf};

use crate::settings::{ConverterVideoCodecs as C, VideoConverterOptions};

/// The encoders the list offers on Windows: all of upstream's, in its order.
#[cfg(windows)]
pub const OFFERED: &[C] = &[
    C::X264,
    C::X265,
    C::H264Nvenc,
    C::HevcNvenc,
    C::H264Amf,
    C::HevcAmf,
    C::H264Qsv,
    C::HevcQsv,
    C::Vp8,
    C::Vp9,
    C::Av1,
    C::Xvid,
    C::Gif,
    C::Webp,
    C::Apng,
];

/// The encoders the list offers on a Mac, in upstream's order.
#[cfg(not(windows))]
pub const OFFERED: &[C] = &[
    C::X264,
    C::X265,
    C::Vp8,
    C::Vp9,
    C::Av1,
    C::Xvid,
    C::Gif,
    C::Webp,
    C::Apng,
];

/// `FFmpegCLIManager` quality limits.
pub const X264_MIN: i64 = 0;
pub const X264_MAX: i64 = 51;

/// `AnimationOnlyExtensions`.
const ANIMATION_ONLY: [&str; 4] = [".gif", ".webp", ".png", ".apng"];

/// The window's state (`VideoConverterViewModel`'s properties).
#[derive(Clone, Debug, PartialEq)]
pub struct Conversion {
    pub input: String,
    pub output_folder: String,
    pub output_name: String,
    pub codec: C,
    pub use_bitrate: bool,
    pub quality: f64,
    pub bitrate: i64,
    pub auto_open_folder: bool,
}

impl Conversion {
    /// The view model's constructor: the saved options, then an input file if one came.
    pub fn from_options(options: &VideoConverterOptions, input: Option<&str>) -> Conversion {
        let mut conversion = Conversion {
            input: options.input_file_path.clone().unwrap_or_default(),
            output_folder: options.output_folder_path.clone().unwrap_or_default(),
            output_name: options.output_file_name.clone().unwrap_or_default(),
            codec: options.video_codec,
            use_bitrate: options.video_quality_use_bitrate,
            quality: options.video_quality as f64,
            bitrate: options.video_quality_bitrate,
            auto_open_folder: options.auto_open_folder,
        };
        if let Some(input) = input.filter(|i| !i.trim().is_empty()) {
            conversion.load_input(input);
        }
        conversion
    }

    /// `PersistSettings`.
    pub fn write_options(&self, options: &mut VideoConverterOptions) {
        options.input_file_path = Some(self.input.clone());
        options.output_folder_path = Some(self.output_folder.clone());
        options.output_file_name = Some(self.output_name.clone());
        options.video_codec = self.codec;
        options.video_quality_use_bitrate = self.use_bitrate;
        options.video_quality = self.quality.round() as i64;
        options.video_quality_bitrate = self.bitrate;
        options.auto_open_folder = self.auto_open_folder;
    }

    /// `LoadInput`: the folder and the name follow the input when they are empty.
    pub fn load_input(&mut self, path: &str) {
        self.input = path.to_string();
        let file = Path::new(path);
        if self.output_folder.trim().is_empty() {
            self.output_folder = file
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if self.output_name.trim().is_empty() {
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.output_name = format!("{stem}-output");
        }
    }

    /// `ShowsQualityControls`.
    pub fn shows_quality_controls(&self) -> bool {
        !matches!(self.codec, C::Gif | C::Webp | C::Apng)
    }

    /// `CanChooseRateControl`.
    pub fn can_choose_rate_control(&self) -> bool {
        matches!(
            self.codec,
            C::X264 | C::X265 | C::Vp8 | C::Vp9 | C::Av1 | C::Xvid
        )
    }

    /// `ShowsQualitySlider`.
    pub fn shows_quality_slider(&self) -> bool {
        self.can_choose_rate_control() && !self.use_bitrate
    }

    /// `ShowsBitrate`.
    pub fn shows_bitrate(&self) -> bool {
        self.shows_quality_controls() && (!self.can_choose_rate_control() || self.use_bitrate)
    }

    /// `QualityMinimum`.
    pub fn quality_minimum(&self) -> f64 {
        match self.codec {
            C::Vp8 => 4.0,
            C::Xvid => 1.0,
            _ => 0.0,
        }
    }

    /// `QualityMaximum`.
    pub fn quality_maximum(&self) -> f64 {
        match self.codec {
            C::Vp8 | C::Vp9 | C::Av1 => 63.0,
            C::Xvid => 31.0,
            _ => 51.0,
        }
    }

    /// `OnSelectedCodecChanged`: the quality is kept inside the new encoder's range.
    pub fn set_codec(&mut self, codec: C) {
        self.codec = codec;
        self.quality = self
            .quality
            .clamp(self.quality_minimum(), self.quality_maximum());
    }

    /// `GetFileExtension`.
    pub fn extension(&self) -> &'static str {
        match self.codec {
            C::Vp8 | C::Vp9 => "webm",
            C::Av1 => "mkv",
            C::Xvid => "avi",
            C::Gif => "gif",
            C::Webp => "webp",
            C::Apng => "apng",
            _ => "mp4",
        }
    }

    /// `OutputFilePath`: folder and name, the encoder's extension when the name has none.
    pub fn output_path(&self) -> String {
        if self.output_folder.trim().is_empty() || self.output_name.trim().is_empty() {
            return String::new();
        }
        let path = PathBuf::from(&self.output_folder).join(&self.output_name);
        // `Path.HasExtension`: a dot with nothing after it is no extension.
        let has_extension = Path::new(&self.output_name)
            .extension()
            .is_some_and(|e| !e.is_empty());
        let path = if has_extension {
            path
        } else {
            path.with_extension(self.extension())
        };
        path.to_string_lossy().into_owned()
    }

    /// `BuildArguments`, as a vector.
    pub fn arguments(&self) -> Vec<String> {
        let mut args: Vec<String> = vec!["-i".into(), self.input.clone()];
        let quality = (self.quality.round() as i64)
            .clamp(self.quality_minimum() as i64, self.quality_maximum() as i64);
        let bitrate = self.bitrate.max(0);
        let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));
        let rate = |use_bitrate: bool, crf: &str| -> Vec<String> {
            if use_bitrate {
                vec!["-b:v".into(), format!("{bitrate}k")]
            } else {
                vec![crf.into(), quality.to_string()]
            }
        };
        match self.codec {
            C::X264 => {
                push(&["-c:v", "libx264", "-preset", "medium"]);
                args.extend(rate(self.use_bitrate, "-crf"));
                args.extend(["-pix_fmt", "yuv420p", "-movflags", "+faststart"].map(String::from));
            }
            C::X265 => {
                push(&["-c:v", "libx265", "-preset", "medium"]);
                args.extend(rate(self.use_bitrate, "-crf"));
            }
            #[cfg(windows)]
            C::H264Nvenc => {
                push(&["-c:v", "h264_nvenc", "-preset", "p4", "-tune", "hq"]);
                push(&["-profile:v", "high"]);
                args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
            }
            #[cfg(windows)]
            C::HevcNvenc => {
                push(&["-c:v", "hevc_nvenc", "-preset", "p4", "-tune", "hq"]);
                push(&["-profile:v", "main"]);
                args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
            }
            #[cfg(windows)]
            C::H264Amf | C::HevcAmf => {
                let encoder = if self.codec == C::H264Amf {
                    "h264_amf"
                } else {
                    "hevc_amf"
                };
                push(&["-c:v", encoder, "-usage", "transcoding", "-profile", "main"]);
                push(&["-quality", "balanced"]);
                args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
            }
            #[cfg(windows)]
            C::H264Qsv | C::HevcQsv => {
                let encoder = if self.codec == C::H264Qsv {
                    "h264_qsv"
                } else {
                    "hevc_qsv"
                };
                push(&["-c:v", encoder, "-preset", "medium"]);
                args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
            }
            #[cfg(not(windows))]
            C::H264Nvenc | C::H264Amf | C::H264Qsv => {
                push(&["-c:v", "h264_videotoolbox"]);
                args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
            }
            #[cfg(not(windows))]
            C::HevcNvenc | C::HevcAmf | C::HevcQsv => {
                push(&["-c:v", "hevc_videotoolbox"]);
                args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
            }
            C::Vp8 => {
                push(&["-c:v", "libvpx"]);
                if self.use_bitrate {
                    args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
                } else {
                    args.extend([
                        "-crf".to_string(),
                        quality.to_string(),
                        "-b:v".into(),
                        "100M".into(),
                    ]);
                }
            }
            C::Vp9 => {
                push(&["-c:v", "libvpx-vp9"]);
                if self.use_bitrate {
                    args.extend(["-b:v".to_string(), format!("{bitrate}k")]);
                } else {
                    args.extend([
                        "-crf".to_string(),
                        quality.to_string(),
                        "-b:v".into(),
                        "0".into(),
                    ]);
                }
            }
            C::Av1 => {
                push(&["-c:v", "libsvtav1"]);
                args.extend(rate(self.use_bitrate, "-crf"));
            }
            C::Xvid => {
                push(&["-c:v", "libxvid"]);
                args.extend(rate(self.use_bitrate, "-q:v"));
            }
            C::Gif => push(&[
                "-lavfi",
                "palettegen=stats_mode=full[palette],[0:v][palette]paletteuse=dither=sierra2_4a",
            ]),
            C::Webp => push(&[
                "-c:v",
                "libwebp",
                "-lossless",
                "0",
                "-preset",
                "default",
                "-loop",
                "0",
            ]),
            C::Apng => push(&["-f", "apng", "-plays", "0"]),
        }
        if matches!(self.codec, C::X265 | C::HevcNvenc | C::HevcAmf | C::HevcQsv) {
            args.extend(["-tag:v", "hvc1"].map(String::from));
        }
        let animation_only = ANIMATION_ONLY
            .iter()
            .any(|e| self.input.to_ascii_lowercase().ends_with(e));
        if !animation_only {
            let audio: &[&str] = match self.codec {
                C::X264
                | C::X265
                | C::H264Nvenc
                | C::HevcNvenc
                | C::H264Amf
                | C::HevcAmf
                | C::H264Qsv
                | C::HevcQsv => &["-c:a", "aac", "-b:a", "128k"],
                C::Vp8 | C::Vp9 => &["-c:a", "libvorbis", "-q:a", "3"],
                C::Av1 => &["-c:a", "libopus", "-b:a", "128k"],
                C::Xvid => &["-c:a", "libmp3lame", "-q:a", "4"],
                _ => &[],
            };
            args.extend(audio.iter().map(|s| s.to_string()));
        }
        args.push("-y".into());
        args.push(self.output_path());
        args
    }
}

/// The last line ffmpeg printed (`ffmpeg.Output ... LastOrDefault()`), the error shown.
pub fn last_line(output: &str) -> Option<String> {
    output
        .split(['\r', '\n'])
        .rfind(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture path as this system writes it: "/v/a" is "C:\\v\\a" on Windows.
    fn native(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_string()
        }
    }

    /// `text` with every fixture path in it written as this system writes it.
    fn natives(text: &str) -> String {
        text.split(' ')
            .map(|word| {
                if word.starts_with("/v") {
                    native(word)
                } else {
                    word.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn conversion(codec: C) -> Conversion {
        let mut c = Conversion::from_options(
            &VideoConverterOptions::default(),
            Some(&native("/v/clip.mov")),
        );
        c.set_codec(codec);
        c
    }

    #[test]
    fn x264_by_quality() {
        let c = conversion(C::X264);
        assert_eq!(c.output_folder, native("/v"));
        assert_eq!(c.output_name, "clip-output");
        assert_eq!(c.output_path(), native("/v/clip-output.mp4"));
        assert_eq!(
            c.arguments().join(" "),
            natives(
                "-i /v/clip.mov -c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p -movflags +faststart -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
    }

    #[test]
    fn bitrates_and_quality_ranges() {
        let mut c = conversion(C::Vp9);
        c.quality = 70.0;
        c.set_codec(C::Vp9);
        assert_eq!(c.quality, 63.0);
        assert_eq!(
            c.arguments().join(" "),
            natives(
                "-i /v/clip.mov -c:v libvpx-vp9 -crf 63 -b:v 0 -c:a libvorbis -q:a 3 -y /v/clip-output.webm"
            )
        );
        c.use_bitrate = true;
        assert!(c.shows_bitrate() && !c.shows_quality_slider());
        assert_eq!(
            c.arguments().join(" "),
            natives(
                "-i /v/clip.mov -c:v libvpx-vp9 -b:v 3000k -c:a libvorbis -q:a 3 -y /v/clip-output.webm"
            )
        );
        let mut c = conversion(C::Xvid);
        c.quality = 0.0;
        assert_eq!(
            c.arguments().join(" "),
            natives(
                "-i /v/clip.mov -c:v libxvid -q:v 1 -c:a libmp3lame -q:a 4 -y /v/clip-output.avi"
            )
        );
        let c = conversion(C::Vp8);
        assert!(
            c.arguments()
                .join(" ")
                .contains("-c:v libvpx -crf 23 -b:v 100M")
        );
    }

    #[test]
    fn animations() {
        let mut c = conversion(C::Gif);
        assert!(!c.shows_quality_controls() && !c.shows_bitrate());
        c.load_input(&native("/v/anim.webp"));
        assert_eq!(
            c.arguments(),
            vec![
                "-i".to_string(),
                native("/v/anim.webp"),
                "-lavfi".to_string(),
                "palettegen=stats_mode=full[palette],[0:v][palette]paletteuse=dither=sierra2_4a"
                    .to_string(),
                "-y".to_string(),
                native("/v/clip-output.gif")
            ]
        );
        // An animated input loses the audio encoder.
        let mut c = conversion(C::X265);
        c.load_input(&native("/v/a.gif"));
        let text = c.arguments().join(" ");
        assert!(text.contains("-tag:v hvc1"));
        assert!(!text.contains("-c:a"));
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_hardware_encoders_become_videotoolbox() {
        let c = conversion(C::HevcNvenc);
        assert!(c.shows_bitrate() && !c.can_choose_rate_control());
        assert_eq!(
            c.arguments().join(" "),
            "-i /v/clip.mov -c:v hevc_videotoolbox -b:v 3000k -tag:v hvc1 -c:a aac -b:a 128k -y /v/clip-output.mp4"
        );
        assert!(!OFFERED.contains(&C::H264Qsv));
    }

    #[test]
    fn names_with_an_extension_keep_it() {
        let mut c = conversion(C::X264);
        c.output_name = "final.mov".into();
        assert_eq!(c.output_path(), native("/v/final.mov"));
        c.output_name = " ".into();
        assert_eq!(c.output_path(), "");
        let mut options = VideoConverterOptions::default();
        c.quality = 27.6;
        c.write_options(&mut options);
        assert_eq!(options.video_quality, 28);
        assert_eq!(options.output_file_name.as_deref(), Some(" "));
    }

    /// `Path.HasExtension` is false for a name ending in a dot, and `ChangeExtension`
    /// then puts the encoder's extension after it ("clip." becomes "clip.mp4"; ffmpeg
    /// cannot choose a format for "clip.").
    #[test]
    fn a_name_ending_in_a_dot_gets_the_extension() {
        let mut c = conversion(C::X264);
        c.output_name = "final.".into();
        assert_eq!(c.output_path(), native("/v/final.mp4"));
        c.output_name = "final.mkv".into();
        assert_eq!(c.output_path(), native("/v/final.mkv"));
    }

    #[test]
    fn the_error_line() {
        assert_eq!(
            last_line("a\r\nUnknown encoder 'libxvid'\r\n").as_deref(),
            Some("Unknown encoder 'libxvid'")
        );
        assert_eq!(last_line(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_hardware_encoders_as_upstream() {
        assert_eq!(OFFERED.len(), 15);
        assert_eq!(OFFERED[2], C::H264Nvenc);
        assert_eq!(OFFERED[7], C::HevcQsv);
        let line = |codec: C| {
            let c = conversion(codec);
            assert!(c.shows_bitrate() && !c.can_choose_rate_control());
            c.arguments().join(" ")
        };
        assert_eq!(
            line(C::H264Nvenc),
            natives(
                "-i /v/clip.mov -c:v h264_nvenc -preset p4 -tune hq -profile:v high -b:v 3000k -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
        assert_eq!(
            line(C::HevcNvenc),
            natives(
                "-i /v/clip.mov -c:v hevc_nvenc -preset p4 -tune hq -profile:v main -b:v 3000k -tag:v hvc1 -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
        assert_eq!(
            line(C::H264Amf),
            natives(
                "-i /v/clip.mov -c:v h264_amf -usage transcoding -profile main -quality balanced -b:v 3000k -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
        assert_eq!(
            line(C::HevcAmf),
            natives(
                "-i /v/clip.mov -c:v hevc_amf -usage transcoding -profile main -quality balanced -b:v 3000k -tag:v hvc1 -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
        assert_eq!(
            line(C::H264Qsv),
            natives(
                "-i /v/clip.mov -c:v h264_qsv -preset medium -b:v 3000k -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
        assert_eq!(
            line(C::HevcQsv),
            natives(
                "-i /v/clip.mov -c:v hevc_qsv -preset medium -b:v 3000k -tag:v hvc1 -c:a aac -b:a 128k -y /v/clip-output.mp4"
            )
        );
    }
}
