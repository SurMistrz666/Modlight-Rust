//! What should be downloaded, and the yt-dlp arguments that ask for it
//! (`DownloadFormat`, `DownloadMediaKind`, `DownloadQuality`, `DownloadAudioFormat`).
//!
//! A queued row keeps the choice that was on screen when it was queued: the picker is a
//! setting for the next download, and a pause is a kill whose resume must run the
//! identical command. The cap is a SORT (`-S res:1080`), not a filter: `res` is the short
//! side, so a vertical 1080x1920 video is "1080p", and a sort falls back to the nearest
//! rendition instead of matching nothing.

/// Whether a download is meant to be watched or listened to (`DownloadMediaKind`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MediaKind {
    #[default]
    Video,
    Audio,
}

/// How good a video is asked for: a ceiling (`DownloadQuality`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    #[default]
    Best,
    H2160,
    H1440,
    H1080,
    H720,
    H480,
    H360,
}

/// What an audio-only download is turned into (`DownloadAudioFormat`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    /// What YouTube already sends, so the stream is copied rather than re-encoded.
    #[default]
    M4a,
    /// Re-encoded; the one format everything plays.
    Mp3,
}

impl MediaKind {
    pub const ALL: [MediaKind; 2] = [MediaKind::Video, MediaKind::Audio];

    /// The member name Newtonsoft writes (`StringEnumConverter`).
    pub fn name(self) -> &'static str {
        match self {
            MediaKind::Video => "Video",
            MediaKind::Audio => "Audio",
        }
    }
}

impl Quality {
    /// Every quality worth offering, best first (`DownloadFormat.Qualities`).
    pub const ALL: [Quality; 7] = [
        Quality::Best,
        Quality::H2160,
        Quality::H1440,
        Quality::H1080,
        Quality::H720,
        Quality::H480,
        Quality::H360,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Quality::Best => "Best",
            Quality::H2160 => "H2160",
            Quality::H1440 => "H1440",
            Quality::H1080 => "H1080",
            Quality::H720 => "H720",
            Quality::H480 => "H480",
            Quality::H360 => "H360",
        }
    }

    /// The pixel ceiling a quality stands for, or 0 for "no ceiling".
    pub fn resolution(self) -> i32 {
        match self {
            Quality::H2160 => 2160,
            Quality::H1440 => 1440,
            Quality::H1080 => 1080,
            Quality::H720 => 720,
            Quality::H480 => 480,
            Quality::H360 => 360,
            Quality::Best => 0,
        }
    }
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 2] = [AudioFormat::M4a, AudioFormat::Mp3];

    pub fn name(self) -> &'static str {
        match self {
            AudioFormat::M4a => "M4a",
            AudioFormat::Mp3 => "Mp3",
        }
    }

    /// The file extension an audio download ends up with.
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::M4a => "m4a",
            AudioFormat::Mp3 => "mp3",
        }
    }
}

/// One answer to "what should be downloaded".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadFormat {
    pub kind: MediaKind,
    pub quality: Quality,
    pub audio_format: AudioFormat,
}

impl DownloadFormat {
    pub fn new(kind: MediaKind, quality: Quality, audio_format: AudioFormat) -> DownloadFormat {
        DownloadFormat {
            kind,
            quality,
            audio_format,
        }
    }

    pub fn is_audio(&self) -> bool {
        self.kind == MediaKind::Audio
    }

    /// The ceiling on the video's short side, or 0. Always 0 for audio.
    pub fn max_resolution(&self) -> i32 {
        if self.is_audio() {
            0
        } else {
            self.quality.resolution()
        }
    }

    /// Everything yt-dlp needs to be told about WHAT to fetch. `ffmpeg` is the merger, or
    /// `None` when this Mac has none: without it nothing can be joined or converted.
    pub fn arguments(&self, is_youtube: bool, ffmpeg: Option<&str>) -> Vec<String> {
        let can_process = ffmpeg.is_some();
        let mut out = vec!["-f".to_string(), self.selector(can_process).to_string()];
        if let Some(sort) = self.sort(is_youtube) {
            out.push("-S".into());
            out.push(sort);
        }
        if self.is_audio() {
            if can_process {
                out.push("-x".into());
                out.push("--audio-format".into());
                out.push(self.audio_format.extension().into());
                out.push("--audio-quality".into());
                out.push("0".into());
            }
        } else if can_process {
            out.push("--merge-output-format".into());
            out.push("mp4".into());
        }
        if let Some(ffmpeg) = ffmpeg {
            out.push("--ffmpeg-location".into());
            out.push(ffmpeg.to_string());
        }
        out
    }

    /// The `-f` expression: which streams, not which of them is best.
    fn selector(&self, can_process: bool) -> &'static str {
        match (self.is_audio(), can_process) {
            (true, true) => "bestaudio/best",
            (true, false) => "bestaudio[ext=m4a]/bestaudio/best",
            (false, true) => "bestvideo*+bestaudio/best",
            (false, false) => "best",
        }
    }

    /// The `-S` ranking, or `None` when yt-dlp's own is right. `vcodec:h264` off YouTube
    /// (TikTok's H.265 renditions come out silent); never on YouTube, whose 1440p and
    /// 2160p exist only as VP9 and AV1.
    fn sort(&self, is_youtube: bool) -> Option<String> {
        if self.is_audio() {
            return None;
        }
        let mut fields: Vec<String> = Vec::with_capacity(3);
        let cap = self.max_resolution();
        if cap > 0 {
            fields.push(format!("res:{cap}"));
        } else if !is_youtube {
            fields.push("res".into());
        }
        if !is_youtube {
            fields.push("vcodec:h264".into());
        }
        fields.push("ext:mp4:m4a".into());
        Some(fields.join(","))
    }

    /// The decision as yt-dlp reads it, for the tooltip on the picker (with ffmpeg).
    pub fn recipe(&self, is_youtube: bool) -> String {
        let mut text = format!("-f {}", self.selector(true));
        if let Some(sort) = self.sort(is_youtube) {
            text.push_str("  -S ");
            text.push_str(&sort);
        }
        if self.is_audio() {
            text.push_str(&format!(
                "  -x --audio-format {} --audio-quality 0",
                self.audio_format.extension()
            ));
        } else {
            text.push_str("  --merge-output-format mp4");
        }
        text
    }
}

/// The short side of what came down, from yt-dlp's width and height.
pub fn resolution_from(width: i32, height: i32) -> i32 {
    if width > 0 && height > 0 {
        width.min(height)
    } else {
        width.max(height).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(quality: Quality) -> DownloadFormat {
        DownloadFormat::new(MediaKind::Video, quality, AudioFormat::M4a)
    }

    fn audio(format: AudioFormat) -> DownloadFormat {
        DownloadFormat::new(MediaKind::Audio, Quality::Best, format)
    }

    #[test]
    fn best_video_on_youtube_leaves_the_ranking_to_yt_dlp() {
        assert_eq!(
            video(Quality::Best).arguments(true, Some("/x/ffmpeg")),
            [
                "-f",
                "bestvideo*+bestaudio/best",
                "-S",
                "ext:mp4:m4a",
                "--merge-output-format",
                "mp4",
                "--ffmpeg-location",
                "/x/ffmpeg"
            ]
        );
    }

    #[test]
    fn a_ceiling_is_a_sort_and_h264_is_preferred_off_youtube() {
        assert_eq!(
            video(Quality::H1080).arguments(false, Some("/x/ffmpeg"))[..4],
            [
                "-f",
                "bestvideo*+bestaudio/best",
                "-S",
                "res:1080,vcodec:h264,ext:mp4:m4a"
            ]
        );
        assert_eq!(
            video(Quality::H720).arguments(true, Some("/x/ffmpeg"))[3],
            "res:720,ext:mp4:m4a"
        );
        assert_eq!(
            video(Quality::Best).arguments(false, Some("/x/ffmpeg"))[3],
            "res,vcodec:h264,ext:mp4:m4a"
        );
    }

    #[test]
    fn audio_is_extracted_only_with_ffmpeg() {
        assert_eq!(
            audio(AudioFormat::Mp3).arguments(true, Some("/x/ffmpeg")),
            [
                "-f",
                "bestaudio/best",
                "-x",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
                "--ffmpeg-location",
                "/x/ffmpeg"
            ]
        );
        assert_eq!(
            audio(AudioFormat::M4a).arguments(true, None),
            ["-f", "bestaudio[ext=m4a]/bestaudio/best"]
        );
    }

    #[test]
    fn without_ffmpeg_nothing_asks_for_merging() {
        let args = video(Quality::H1080).arguments(true, None);
        assert_eq!(args, ["-f", "best", "-S", "res:1080,ext:mp4:m4a"]);
        assert!(
            !args
                .iter()
                .any(|a| a.contains("merge") || a.contains("ffmpeg"))
        );
    }

    #[test]
    fn the_recipe_names_what_the_entry_maps_to() {
        assert_eq!(
            video(Quality::H1080).recipe(true),
            "-f bestvideo*+bestaudio/best  -S res:1080,ext:mp4:m4a  --merge-output-format mp4"
        );
        assert_eq!(
            audio(AudioFormat::M4a).recipe(true),
            "-f bestaudio/best  -x --audio-format m4a --audio-quality 0"
        );
    }

    #[test]
    fn the_short_side_is_the_resolution() {
        assert_eq!(resolution_from(1080, 1920), 1080);
        assert_eq!(resolution_from(1920, 1080), 1080);
        assert_eq!(resolution_from(0, 720), 720);
        assert_eq!(resolution_from(0, 0), 0);
        assert_eq!(video(Quality::H1080).max_resolution(), 1080);
        assert_eq!(audio(AudioFormat::Mp3).max_resolution(), 0);
    }
}
