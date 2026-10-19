//! `FFmpegOptions` and its enums, with upstream's member names, values and defaults, so a
//! settings document from a Windows installation reads (and writes back) unchanged.

use std::path::PathBuf;

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tasks::settings::named_enum;

macro_rules! described {
    ($name:ident { $($variant:ident => $desc:expr),+ $(,)? }) => {
        impl $name {
            /// The `[Description]` upstream gives the value (shown as is: these are
            /// codec and preset names, not translated upstream either).
            pub fn description(self) -> &'static str {
                match self { $($name::$variant => $desc),+ }
            }
        }
    };
}

named_enum! {
    /// `FFmpegVideoCodec`.
    #[allow(non_camel_case_types)]
    FFmpegVideoCodec {
        libx264, libx265, libvpx, libvpx_vp9, libxvid, h264_nvenc, hevc_nvenc, h264_amf,
        hevc_amf, h264_qsv, hevc_qsv, gif, libwebp, apng
    } default libx264
}

described!(FFmpegVideoCodec {
    libx264 => "H.264 / x264",
    libx265 => "H.265 / x265",
    libvpx => "VP8",
    libvpx_vp9 => "VP9",
    libxvid => "MPEG-4 / Xvid",
    h264_nvenc => "H.264 / NVENC",
    hevc_nvenc => "HEVC / NVENC",
    h264_amf => "H.264 / AMF",
    hevc_amf => "HEVC / AMF",
    h264_qsv => "H.264 / Quick Sync",
    hevc_qsv => "HEVC / Quick Sync",
    gif => "GIF",
    libwebp => "WebP",
    apng => "APNG",
});

impl FFmpegVideoCodec {
    /// The codecs offered on a Mac. NVENC, AMF and Quick Sync are Windows hardware
    /// encoders; a value synced from Windows that names one is encoded with VideoToolbox
    /// instead (see [`FFmpegVideoCodec::encoder`]), but the list does not offer them.
    pub const OFFERED: &'static [FFmpegVideoCodec] = &[
        FFmpegVideoCodec::libx264,
        FFmpegVideoCodec::libx265,
        FFmpegVideoCodec::libvpx,
        FFmpegVideoCodec::libvpx_vp9,
        FFmpegVideoCodec::libxvid,
        FFmpegVideoCodec::gif,
        FFmpegVideoCodec::libwebp,
        FFmpegVideoCodec::apng,
    ];

    /// The ffmpeg encoder that runs on this Mac for the codec.
    pub fn encoder(self) -> &'static str {
        match self {
            FFmpegVideoCodec::libvpx_vp9 => "libvpx-vp9",
            FFmpegVideoCodec::h264_nvenc
            | FFmpegVideoCodec::h264_amf
            | FFmpegVideoCodec::h264_qsv => "h264_videotoolbox",
            FFmpegVideoCodec::hevc_nvenc
            | FFmpegVideoCodec::hevc_amf
            | FFmpegVideoCodec::hevc_qsv => "hevc_videotoolbox",
            other => other.name(),
        }
    }

    pub fn is_hardware(self) -> bool {
        matches!(
            self,
            FFmpegVideoCodec::h264_nvenc
                | FFmpegVideoCodec::hevc_nvenc
                | FFmpegVideoCodec::h264_amf
                | FFmpegVideoCodec::hevc_amf
                | FFmpegVideoCodec::h264_qsv
                | FFmpegVideoCodec::hevc_qsv
        )
    }
}

named_enum! {
    /// `FFmpegAudioCodec`.
    #[allow(non_camel_case_types)]
    FFmpegAudioCodec { libvoaacenc, libopus, libvorbis, libmp3lame } default libvoaacenc
}

described!(FFmpegAudioCodec {
    libvoaacenc => "AAC",
    libopus => "Opus",
    libvorbis => "Vorbis",
    libmp3lame => "MP3",
});

named_enum! {
    /// `FFmpegPreset`.
    #[allow(non_camel_case_types)]
    FFmpegPreset {
        ultrafast, superfast, veryfast, faster, fast, medium, slow, slower, veryslow, placebo
    } default ultrafast
}

described!(FFmpegPreset {
    ultrafast => "Ultra fast",
    superfast => "Super fast",
    veryfast => "Very fast",
    faster => "Faster",
    fast => "Fast",
    medium => "Medium",
    slow => "Slow",
    slower => "Slower",
    veryslow => "Very slow",
    placebo => "Placebo",
});

named_enum! {
    /// `FFmpegTune`.
    #[allow(non_camel_case_types)]
    FFmpegTune { film, animation, grain, stillimage, psnr, ssim, fastdecode, zerolatency } default film
}

named_enum! {
    /// `FFmpegNVENCPreset`.
    #[allow(non_camel_case_types)]
    FFmpegNVENCPreset { p1, p2, p3, p4, p5, p6, p7 } default p4
}

named_enum! {
    /// `FFmpegNVENCTune`.
    #[allow(non_camel_case_types)]
    FFmpegNVENCTune { hq, ll, ull, lossless } default ll
}

named_enum! {
    /// `FFmpegAMFUsage`.
    #[allow(non_camel_case_types)]
    FFmpegAMFUsage {
        transcoding, ultralowlatency, lowlatency, webcam, high_quality, lowlatency_high_quality
    } default lowlatency
}

named_enum! {
    /// `FFmpegAMFQuality`.
    #[allow(non_camel_case_types)]
    FFmpegAMFQuality { speed, balanced, quality } default speed
}

named_enum! {
    /// `FFmpegQSVPreset`.
    #[allow(non_camel_case_types)]
    FFmpegQSVPreset { veryfast, faster, fast, medium, slow, slower, veryslow } default fast
}

named_enum! {
    /// `FFmpegPaletteGenStatsMode`.
    #[allow(non_camel_case_types)]
    FFmpegPaletteGenStatsMode { full, diff, single } default full
}

named_enum! {
    /// `FFmpegPaletteUseDither`.
    #[allow(non_camel_case_types)]
    FFmpegPaletteUseDither {
        none, bayer, heckbert, floyd_steinberg, sierra2, sierra2_4a, sierra3, burkes, atkinson
    } default sierra2_4a
}

/// `ScreenRecordOutput`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScreenRecordOutput {
    FFmpeg,
    Gif,
}

/// `FFmpegCaptureDevice` as the Mac knows it.
///
/// Upstream's screen sources are Windows devices ("GDI grab", "DDA grab",
/// "screen-capture-recorder"); on a Mac the screen comes from ScreenCaptureKit and is
/// piped into ffmpeg. A synced setting naming one of the Windows screen sources means
/// "the screen" here too, so a document from Windows records the same thing.
pub mod capture_device {
    /// "None": no source.
    pub const NONE: &str = "";
    /// The screen through ScreenCaptureKit (the Mac's counterpart of "GDI grab").
    pub const SCREEN: &str = "ScreenCaptureKit";
    /// Upstream's values that mean "the screen" on Windows.
    pub const WINDOWS_SCREEN_SOURCES: [&str; 3] =
        ["GDI grab", "DDA grab", "screen-capture-recorder"];

    /// Whether a `VideoSource` value means the screen.
    pub fn is_screen(value: &str) -> bool {
        value.eq_ignore_ascii_case(SCREEN)
            || WINDOWS_SCREEN_SOURCES
                .iter()
                .any(|s| value.eq_ignore_ascii_case(s))
    }
}

fn string_or_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value.as_str().unwrap_or_default().to_string())
}

/// `FFmpegOptions`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
#[allow(non_snake_case)]
pub struct FFmpegOptions {
    pub OverrideCLIPath: bool,
    #[serde(deserialize_with = "string_or_default")]
    pub CLIPath: String,
    #[serde(deserialize_with = "string_or_default")]
    pub VideoSource: String,
    #[serde(deserialize_with = "string_or_default")]
    pub AudioSource: String,
    pub VideoCodec: FFmpegVideoCodec,
    pub AudioCodec: FFmpegAudioCodec,
    #[serde(deserialize_with = "string_or_default")]
    pub UserArgs: String,
    pub UseCustomCommands: bool,
    #[serde(deserialize_with = "string_or_default")]
    pub CustomCommands: String,
    pub x264_Preset: FFmpegPreset,
    pub x264_CRF: i64,
    pub x264_Use_Bitrate: bool,
    pub x264_Bitrate: i64,
    pub VPx_Bitrate: i64,
    pub XviD_QScale: i64,
    pub NVENC_Preset: FFmpegNVENCPreset,
    pub NVENC_Tune: FFmpegNVENCTune,
    pub NVENC_Bitrate: i64,
    pub GIFStatsMode: FFmpegPaletteGenStatsMode,
    pub GIFDither: FFmpegPaletteUseDither,
    pub GIFBayerScale: i64,
    pub AMF_Usage: FFmpegAMFUsage,
    pub AMF_Quality: FFmpegAMFQuality,
    pub AMF_Bitrate: i64,
    pub QSV_Preset: FFmpegQSVPreset,
    pub QSV_Bitrate: i64,
    pub AAC_Bitrate: i64,
    pub Opus_Bitrate: i64,
    pub Vorbis_QScale: i64,
    pub MP3_QScale: i64,
}

impl Default for FFmpegOptions {
    fn default() -> Self {
        FFmpegOptions {
            OverrideCLIPath: false,
            CLIPath: String::new(),
            VideoSource: capture_device::SCREEN.into(),
            AudioSource: capture_device::NONE.into(),
            VideoCodec: FFmpegVideoCodec::libx264,
            AudioCodec: FFmpegAudioCodec::libvoaacenc,
            UserArgs: String::new(),
            UseCustomCommands: false,
            CustomCommands: String::new(),
            x264_Preset: FFmpegPreset::ultrafast,
            x264_CRF: 28,
            x264_Use_Bitrate: false,
            x264_Bitrate: 3000,
            VPx_Bitrate: 3000,
            XviD_QScale: 10,
            NVENC_Preset: FFmpegNVENCPreset::p4,
            NVENC_Tune: FFmpegNVENCTune::ll,
            NVENC_Bitrate: 3000,
            GIFStatsMode: FFmpegPaletteGenStatsMode::full,
            GIFDither: FFmpegPaletteUseDither::sierra2_4a,
            GIFBayerScale: 2,
            AMF_Usage: FFmpegAMFUsage::lowlatency,
            AMF_Quality: FFmpegAMFQuality::speed,
            AMF_Bitrate: 3000,
            QSV_Preset: FFmpegQSVPreset::fast,
            QSV_Bitrate: 3000,
            AAC_Bitrate: 128,
            Opus_Bitrate: 128,
            Vorbis_QScale: 3,
            MP3_QScale: 4,
        }
    }
}

impl FFmpegOptions {
    pub const X264_MIN: i64 = 0;
    pub const X264_MAX: i64 = 51;
    pub const VP8_MIN: i64 = 4;
    pub const VP8_MAX: i64 = 63;
    pub const XVID_MIN: i64 = 1;
    pub const XVID_MAX: i64 = 31;
    pub const MP3_MIN: i64 = 0;
    pub const MP3_MAX: i64 = 9;

    /// `FFmpegPath`: the overriding path when one is set, else the installed component.
    pub fn ffmpeg_path(&self, installed: Option<PathBuf>) -> Option<PathBuf> {
        if self.OverrideCLIPath && !self.CLIPath.is_empty() {
            return Some(crate::names::file_helpers::get_absolute_path(&self.CLIPath).into());
        }
        installed
    }

    /// `Extension`.
    pub fn extension(&self) -> &'static str {
        if !self.VideoSource.is_empty() {
            return match self.VideoCodec {
                FFmpegVideoCodec::libx264
                | FFmpegVideoCodec::libx265
                | FFmpegVideoCodec::h264_nvenc
                | FFmpegVideoCodec::hevc_nvenc
                | FFmpegVideoCodec::h264_amf
                | FFmpegVideoCodec::hevc_amf
                | FFmpegVideoCodec::h264_qsv
                | FFmpegVideoCodec::hevc_qsv => "mp4",
                FFmpegVideoCodec::libvpx | FFmpegVideoCodec::libvpx_vp9 => "webm",
                FFmpegVideoCodec::libxvid => "avi",
                FFmpegVideoCodec::gif => "gif",
                FFmpegVideoCodec::libwebp => "webp",
                FFmpegVideoCodec::apng => "apng",
            };
        }
        if !self.AudioSource.is_empty() {
            return match self.AudioCodec {
                FFmpegAudioCodec::libvoaacenc => "m4a",
                FFmpegAudioCodec::libopus => "opus",
                FFmpegAudioCodec::libvorbis => "ogg",
                FFmpegAudioCodec::libmp3lame => "mp3",
            };
        }
        "mp4"
    }

    pub fn is_video_source_selected(&self) -> bool {
        !self.VideoSource.is_empty()
    }

    pub fn is_animated_image(&self) -> bool {
        matches!(
            self.VideoCodec,
            FFmpegVideoCodec::gif | FFmpegVideoCodec::libwebp | FFmpegVideoCodec::apng
        )
    }

    pub fn is_audio_source_selected(&self) -> bool {
        !self.AudioSource.is_empty()
            && (!self.is_video_source_selected() || !self.is_animated_image())
    }

    pub fn is_source_selected(&self) -> bool {
        self.is_video_source_selected() || self.is_audio_source_selected()
    }

    pub fn is_even_size_required(&self) -> bool {
        !self.is_animated_image()
    }

    /// `FixSources`: old spellings of "None"; on a Mac also the Windows screen sources.
    pub fn fix_sources(&mut self) {
        if self.VideoSource.eq_ignore_ascii_case("None") {
            self.VideoSource = capture_device::NONE.into();
        } else if capture_device::is_screen(&self.VideoSource) {
            self.VideoSource = capture_device::SCREEN.into();
        }
        if self.AudioSource.eq_ignore_ascii_case("None") {
            self.AudioSource = capture_device::NONE.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_windows_document_reads_with_its_values() {
        let mut options: FFmpegOptions = serde_json::from_value(json!({
            "VideoSource": "GDI grab",
            "AudioSource": "None",
            "VideoCodec": "libvpx_vp9",
            "x264_CRF": 23,
            "GIFDither": "bayer",
            "Unknown": 1
        }))
        .unwrap();
        options.fix_sources();
        assert_eq!(options.VideoSource, capture_device::SCREEN);
        assert_eq!(options.AudioSource, "");
        assert_eq!(options.VideoCodec, FFmpegVideoCodec::libvpx_vp9);
        assert_eq!(options.VideoCodec.encoder(), "libvpx-vp9");
        assert_eq!(options.extension(), "webm");
        assert_eq!(options.x264_CRF, 23);
        assert_eq!(options.GIFDither, FFmpegPaletteUseDither::bayer);
        assert_eq!(options.AAC_Bitrate, 128);
        let written = serde_json::to_value(&options).unwrap();
        assert_eq!(written["VideoCodec"], "libvpx_vp9");
        assert_eq!(written["x264_Preset"], "ultrafast");
    }

    #[test]
    fn extensions_and_sources_follow_upstream() {
        let mut options = FFmpegOptions::default();
        assert_eq!(options.extension(), "mp4");
        options.VideoCodec = FFmpegVideoCodec::gif;
        assert!(options.is_animated_image());
        assert!(!options.is_even_size_required());
        options.AudioSource = "MacBook Pro Microphone".into();
        assert!(!options.is_audio_source_selected(), "no audio in a GIF");
        options.VideoSource.clear();
        assert!(options.is_audio_source_selected());
        options.AudioCodec = FFmpegAudioCodec::libopus;
        assert_eq!(options.extension(), "opus");
        assert_eq!(FFmpegVideoCodec::h264_nvenc.encoder(), "h264_videotoolbox");
        assert_eq!(FFmpegVideoCodec::hevc_qsv.encoder(), "hevc_videotoolbox");
    }
}
