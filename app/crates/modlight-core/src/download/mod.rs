//! The Download tab's engine (`ShareX/Presentation/Download`): addresses, formats, the
//! yt-dlp run, its errors, the YouTube cookie jar, finding the tools, and the queue's
//! rows. The window side is `modlight-app::download_tab`.

pub mod cookies;
pub mod error;
pub mod format;
pub mod item;
pub mod link;
pub mod run;
pub mod tool;
pub mod version_check;

use serde_json::{Value, json};

use crate::config::AppConfig;
pub use format::{AudioFormat, DownloadFormat, MediaKind, Quality};
pub use link::DownloadLink;

/// Where a finished download ends up (`DownloadDestination`). One setting for the tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Destination {
    /// Keep the file on this Mac and do nothing else. The default.
    #[default]
    Disk,
    /// Upload it and put the link on the clipboard, the way a capture goes.
    Modlight,
    /// Both: the file stays here and a copy goes up.
    Both,
}

impl Destination {
    pub const ALL: [Destination; 3] = [Destination::Disk, Destination::Modlight, Destination::Both];

    pub fn name(self) -> &'static str {
        match self {
            Destination::Disk => "Disk",
            Destination::Modlight => "Modlight",
            Destination::Both => "Both",
        }
    }
}

/// An enum member as Newtonsoft may have written it: its name, or its number.
fn read_enum<T: Copy>(
    config: &AppConfig,
    member: &str,
    all: &[T],
    name: fn(T) -> &'static str,
) -> Option<T> {
    match config.get(member)? {
        Value::String(text) => all
            .iter()
            .copied()
            .find(|v| name(*v).eq_ignore_ascii_case(text)),
        Value::Number(number) => number.as_u64().and_then(|i| all.get(i as usize).copied()),
        _ => None,
    }
}

/// The `ApplicationConfig` members of the Download tab.
pub mod settings {
    use super::*;

    pub fn destination(config: &AppConfig) -> Destination {
        read_enum(
            config,
            "DownloadDestination",
            &Destination::ALL,
            Destination::name,
        )
        .unwrap_or_default()
    }

    pub fn set_destination(config: &mut AppConfig, value: Destination) {
        config.set("DownloadDestination", json!(value.name()));
    }

    pub fn media_kind(config: &AppConfig) -> MediaKind {
        read_enum(
            config,
            "DownloadMediaKind",
            &MediaKind::ALL,
            MediaKind::name,
        )
        .unwrap_or_default()
    }

    pub fn set_media_kind(config: &mut AppConfig, value: MediaKind) {
        config.set("DownloadMediaKind", json!(value.name()));
    }

    pub fn video_quality(config: &AppConfig) -> Quality {
        read_enum(config, "DownloadVideoQuality", &Quality::ALL, Quality::name).unwrap_or_default()
    }

    pub fn set_video_quality(config: &mut AppConfig, value: Quality) {
        config.set("DownloadVideoQuality", json!(value.name()));
    }

    pub fn audio_format(config: &AppConfig) -> AudioFormat {
        read_enum(
            config,
            "DownloadAudioFormat",
            &AudioFormat::ALL,
            AudioFormat::name,
        )
        .unwrap_or_default()
    }

    pub fn set_audio_format(config: &mut AppConfig, value: AudioFormat) {
        config.set("DownloadAudioFormat", json!(value.name()));
    }

    /// The folder every download goes to, or empty for "ask me".
    pub fn folder(config: &AppConfig) -> String {
        config
            .get("DownloadFolder")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    pub fn set_folder(config: &mut AppConfig, value: &str) {
        config.set("DownloadFolder", json!(value.trim()));
    }

    /// `BinaryUnits`: local sizes in KiB and MiB.
    pub fn binary_units(config: &AppConfig) -> bool {
        config
            .get("BinaryUnits")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// What a row queued right now would be asked to fetch (`CurrentFormat`).
    pub fn current_format(config: &AppConfig) -> DownloadFormat {
        DownloadFormat::new(
            media_kind(config),
            video_quality(config),
            audio_format(config),
        )
    }
}

/// Where a Modlight-only download's copy lives until the upload has had it.
pub fn scratch_folder() -> std::path::PathBuf {
    std::env::temp_dir().join(crate::APP_NAME).join("Downloads")
}

/// How long such a copy may linger (`scratchLifetime`).
pub const SCRATCH_LIFETIME: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Yesterday's Modlight-only downloads, whose uploads finished long ago.
pub fn sweep_scratch(folder: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
            .is_some_and(|age| age > SCRATCH_LIFETIME);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_names_and_numbers_and_write_names() {
        let mut config = AppConfig::in_memory();
        assert_eq!(settings::destination(&config), Destination::Disk);
        assert_eq!(settings::video_quality(&config), Quality::Best);
        config.set("DownloadDestination", json!(2));
        assert_eq!(settings::destination(&config), Destination::Both);
        settings::set_destination(&mut config, Destination::Modlight);
        assert_eq!(config.get("DownloadDestination"), Some(&json!("Modlight")));
        settings::set_video_quality(&mut config, Quality::H720);
        assert_eq!(settings::video_quality(&config), Quality::H720);
        settings::set_media_kind(&mut config, MediaKind::Audio);
        settings::set_audio_format(&mut config, AudioFormat::Mp3);
        assert_eq!(
            settings::current_format(&config),
            DownloadFormat::new(MediaKind::Audio, Quality::H720, AudioFormat::Mp3)
        );
        settings::set_folder(&mut config, "  /Users/alex/Movies ");
        assert_eq!(settings::folder(&config), "/Users/alex/Movies");
        config.set("DownloadMediaKind", json!("Nonsense"));
        assert_eq!(settings::media_kind(&config), MediaKind::Video);
    }
}
