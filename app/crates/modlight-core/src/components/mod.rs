//! External components (`ModlightComponent`, `ModlightComponentCatalog`,
//! `ModlightComponents`): FFmpeg, yt-dlp, the background removal model and ExifTool -
//! what each is for, where its bytes come from, what they must hash to, and where it
//! lives once it is here.
//!
//! Everything in the catalogue is a constant compiled into the app, and that is the
//! point: the digest is not fetched next to the file it vouches for, it ships with the
//! application, so a swapped or truncated artefact fails against a value a human pinned.
//! A version bump is a code change - except for yt-dlp, which goes stale in weeks and
//! follows its publisher under a rule of its own (`updates`).
//!
//! Windows: upstream's pins (`ModlightComponent.cs`) for x64 and arm64 by the build's
//! architecture, except ExifTool, whose upstream artefact is no longer published (the
//! official Windows build of the current release instead). macOS: the publishers' macOS
//! arm64 artefacts. Provenance of both in `docs/handoff/`. The folders are
//! `<profile>/Tools` and `<profile>/Models` of the personal folder
//! (`crate::personal_folder`).

pub mod archive;
pub mod download;
pub mod install;
pub mod net;
pub mod process;
pub mod tool_release;
pub mod updates;
pub mod verify;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::AppConfig;
use crate::l10n::{Key, s};

pub const FFMPEG: &str = "ffmpeg";
pub const YT_DLP: &str = "yt-dlp";
pub const BACKGROUND_MODEL: &str = "background-model";
pub const EXIFTOOL: &str = "exiftool";

/// The folder fetched tools land in, inside the personal folder (`Program.ToolsFolder`).
pub const TOOLS_FOLDER: &str = "Tools";
/// The folder models land in (`Program.ModelsFolder`).
pub const MODELS_FOLDER: &str = "Models";

/// What has to happen to the downloaded file before it is a working component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packaging {
    /// The download IS the component; `executable` sets mode 755.
    File { executable: bool },
    /// A zip holding one file we want, wherever it sits inside; it comes out executable.
    ZipSingleFile,
    /// A gzip'd tarball with one top-level folder; `keep` names what is taken from
    /// inside it, into the folder named by the component's first owned entry.
    /// `executables` are paths inside that folder that get mode 755.
    TarGzFolder {
        keep: &'static [&'static str],
        executables: &'static [&'static str],
    },
    /// A zip with one top-level folder; each `(name inside it, name in the component's
    /// folder)` pair, a file or a whole folder, is taken out and put into the
    /// component's folder under its new name. Upstream's `ZipTree` for an archive that
    /// keeps its files in a folder of their own (ExifTool's Windows build).
    ZipFolder {
        keep: &'static [(&'static str, &'static str)],
    },
}

/// Which of the two folders a component belongs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentFolder {
    Tools,
    Models,
}

/// One external component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    pub id: &'static str,
    /// Read at call time: the language can change while the app is up.
    pub name: Key,
    /// One line saying which feature dies without it.
    pub purpose: Key,
    /// HTTPS only. Enforced again before the request is made.
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the file at `url`.
    pub sha256: &'static str,
    /// Bytes of the download, so "this costs 65 MB" can be said before anybody commits.
    pub download_size: i64,
    /// The file whose presence means "installed", relative to the component's folder.
    pub file_name: &'static str,
    pub folder: ComponentFolder,
    pub packaging: Packaging,
    /// Ceiling for an archive's uncompressed size. Zero for a plain file.
    pub max_uncompressed_size: u64,
    /// Where the artefact and its digest were published. Shown in the UI.
    pub source: &'static str,
    /// Version string, for the log and the settings page.
    pub version: &'static str,
    /// Everything this component owns in its folder, for removal.
    pub owned_entries: &'static [&'static str],
    /// The GitHub repository whose latest release this component follows, or `None`.
    pub update_repository: Option<&'static str>,
    /// The release asset for this platform, matched by exact name.
    pub update_asset: Option<&'static str>,
    /// The publisher's checksum manifest in the same release.
    pub update_checksums: Option<&'static str>,
    /// What to run the executable with to make it print its own version (an argument
    /// vector, never a command line). Empty when there is nothing to ask.
    pub version_arguments: &'static [&'static str],
}

impl Component {
    /// True when this component has somewhere to look for a newer build.
    pub fn tracks_updates(&self) -> bool {
        self.update_repository.is_some() && self.update_asset.is_some()
    }

    pub fn display_name(&self) -> &'static str {
        crate::l10n::t(self.name)
    }

    pub fn purpose_text(&self) -> &'static str {
        crate::l10n::t(self.purpose)
    }

    /// The entries this component owns, or its file name when none are listed.
    pub fn owned(&self) -> Vec<&'static str> {
        if self.owned_entries.is_empty() {
            vec![self.file_name]
        } else {
            self.owned_entries.to_vec()
        }
    }
}

/// Which bytes to fetch: the pinned ones, or a newer published build
/// (`ModlightComponentSource`). An update and a first install are the same code path
/// with a different four fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentSource {
    pub url: String,
    pub sha256: String,
    pub size: i64,
    pub version: String,
}

impl ComponentSource {
    /// What the catalogue pins: the bytes a first install fetches.
    pub fn pinned(component: &Component) -> ComponentSource {
        ComponentSource {
            url: component.url.to_string(),
            sha256: component.sha256.to_string(),
            size: component.download_size,
            version: component.version.to_string(),
        }
    }
}

/// Every component, in the order they are offered.
///
/// PROVENANCE: every digest below was taken from its publisher and recomputed from the
/// real download before being pinned; the table with URLs, sizes, dates and how each was
/// checked is in `docs/handoff/app-3-media.md`.
pub fn catalog() -> &'static [Component] {
    &CATALOG
}

#[cfg(not(windows))]
static CATALOG: [Component; 4] = [
    // Martin Riedl's macOS arm64 build of the version the Windows app pins (8.1): a
    // versioned URL that keeps serving the same bytes, a .sha256 published beside it, a
    // Developer ID signature (team KU3N25YGLU), only system libraries linked. The zip
    // holds the one binary, 62.6 MB unpacked.
    Component {
        id: FFMPEG,
        name: s::Components_FFmpeg,
        purpose: s::Components_FFmpegPurpose,
        version: "8.1",
        url: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1774549676_8.1/ffmpeg.zip",
        sha256: "cc3a7e0cce36c5eca6c17eeb93830984c657637a8e710dc98f19c8051201fa3a",
        download_size: 27_185_234,
        file_name: "ffmpeg",
        folder: ComponentFolder::Tools,
        packaging: Packaging::ZipSingleFile,
        max_uncompressed_size: 512 * 1024 * 1024,
        source: "ffmpeg.martin-riedl.de",
        owned_entries: &["ffmpeg"],
        update_repository: None,
        update_asset: None,
        update_checksums: None,
        version_arguments: &[],
    },
    // The pin is the FLOOR, not the version anybody ends up running: yt-dlp follows its
    // publisher (see `updates`). Same release as the Windows pin, macOS asset.
    Component {
        id: YT_DLP,
        name: s::Components_YtDlp,
        purpose: s::Components_YtDlpPurpose,
        version: "2026.08.19",
        url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos",
        sha256: "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202",
        download_size: 37_146_048,
        file_name: "yt-dlp",
        folder: ComponentFolder::Tools,
        packaging: Packaging::File { executable: true },
        max_uncompressed_size: 0,
        source: "github.com/yt-dlp/yt-dlp",
        owned_entries: &["yt-dlp"],
        update_repository: Some("yt-dlp/yt-dlp"),
        update_asset: Some("yt-dlp_macos"),
        update_checksums: Some("SHA2-256SUMS"),
        // --ignore-config first: a yt-dlp.conf on this machine is read before anything
        // else and one line of it would turn the answer into something else.
        version_arguments: &["--ignore-config", "--version"],
    },
    // Cross-platform: the Windows app's pin, unchanged.
    Component {
        id: BACKGROUND_MODEL,
        name: s::Components_BackgroundModel,
        purpose: s::Components_BackgroundModelPurpose,
        version: "bria-rmbg-2.0",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/bria-rmbg-2.0.onnx",
        sha256: "5b486f08200f513f460da46dd701db5fbb47d79b4be4b708a19444bcd4e79958",
        download_size: 1_024_331_469,
        file_name: "bria-rmbg-2.0.onnx",
        folder: ComponentFolder::Models,
        packaging: Packaging::File { executable: false },
        max_uncompressed_size: 0,
        source: "github.com/danielgatis/rembg",
        owned_entries: &["bria-rmbg-2.0.onnx"],
        update_repository: None,
        update_asset: None,
        update_checksums: None,
        version_arguments: &[],
    },
    // The official Perl distribution (Phil Harvey's checksums.txt publishes the SHA-256;
    // exiftool.org hands its downloads to SourceForge). The Windows pin, 13.29, is no
    // longer published anywhere, so this is the current production release. Only the
    // `exiftool` script and its `lib` folder are installed, run by /usr/bin/perl, in a
    // folder of their own because the script finds its library beside itself.
    Component {
        id: EXIFTOOL,
        name: s::Components_ExifTool,
        purpose: s::Components_ExifToolPurpose,
        version: "13.59",
        url: "https://downloads.sourceforge.net/project/exiftool/Image-ExifTool-13.59.tar.gz",
        sha256: "668ea3acececb7235fbd0f4900e72d5f12c9b07e5c778fd36cb1e9b5828fd65a",
        download_size: 7_916_358,
        file_name: "ExifTool/exiftool",
        folder: ComponentFolder::Tools,
        packaging: Packaging::TarGzFolder {
            keep: &["exiftool", "lib"],
            executables: &["exiftool"],
        },
        max_uncompressed_size: 256 * 1024 * 1024,
        source: "exiftool.org",
        owned_entries: &["ExifTool"],
        update_repository: None,
        update_asset: None,
        update_checksums: None,
        version_arguments: &[],
    },
];

/// Upstream's own catalogue (`ModlightComponentCatalog`), for the architecture this
/// build is for (`RuntimeInformation.ProcessArchitecture == Arm64` upstream).
#[cfg(windows)]
const ARM64: bool = cfg!(target_arch = "aarch64");

#[cfg(windows)]
static CATALOG: [Component; 4] = [
    // ShareX's own FFmpeg build, upstream's pin: one `ffmpeg.exe` in the zip (193 MiB
    // unpacked on x64, 128 MiB on arm64).
    Component {
        id: FFMPEG,
        name: s::Components_FFmpeg,
        purpose: s::Components_FFmpegPurpose,
        version: "8.1",
        url: if ARM64 {
            "https://github.com/ShareX/FFmpeg/releases/download/v8.1/ffmpeg-8.1-win-arm64.zip"
        } else {
            "https://github.com/ShareX/FFmpeg/releases/download/v8.1/ffmpeg-8.1-win-x64.zip"
        },
        sha256: if ARM64 {
            "7ac6c5dbbe61037bcce97482549294238d87400d8b7b1cb559949293c3b06dd5"
        } else {
            "1838932e32f01e0e7b65fde5ca0ebb2ab131cb6a9ff1a84025ee5b784075576c"
        },
        download_size: if ARM64 { 50_657_497 } else { 68_599_421 },
        file_name: "ffmpeg.exe",
        folder: ComponentFolder::Tools,
        packaging: Packaging::ZipSingleFile,
        max_uncompressed_size: 512 * 1024 * 1024,
        source: "github.com/ShareX/FFmpeg",
        owned_entries: &["ffmpeg.exe"],
        update_repository: None,
        update_asset: None,
        update_checksums: None,
        version_arguments: &[],
    },
    // The pin is the FLOOR (see `updates`); installed as `yt-dlp.exe` on both
    // architectures, as upstream.
    Component {
        id: YT_DLP,
        name: s::Components_YtDlp,
        purpose: s::Components_YtDlpPurpose,
        version: "2026.08.19",
        url: if ARM64 {
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_arm64.exe"
        } else {
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe"
        },
        sha256: if ARM64 {
            "05b438997bafc3affdfda9d041353c9d73e04dc842207254b655b0887c4445b0"
        } else {
            "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"
        },
        download_size: if ARM64 { 21_159_286 } else { 17_840_399 },
        file_name: "yt-dlp.exe",
        folder: ComponentFolder::Tools,
        packaging: Packaging::File { executable: true },
        max_uncompressed_size: 0,
        source: "github.com/yt-dlp/yt-dlp",
        owned_entries: &["yt-dlp.exe"],
        update_repository: Some("yt-dlp/yt-dlp"),
        update_asset: Some(if ARM64 {
            "yt-dlp_arm64.exe"
        } else {
            "yt-dlp.exe"
        }),
        update_checksums: Some("SHA2-256SUMS"),
        // --ignore-config first: a yt-dlp.conf on this machine is read before anything
        // else and one line of it would turn the answer into something else.
        version_arguments: &["--ignore-config", "--version"],
    },
    // Upstream's pin, unchanged.
    Component {
        id: BACKGROUND_MODEL,
        name: s::Components_BackgroundModel,
        purpose: s::Components_BackgroundModelPurpose,
        version: "bria-rmbg-2.0",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/bria-rmbg-2.0.onnx",
        sha256: "5b486f08200f513f460da46dd701db5fbb47d79b4be4b708a19444bcd4e79958",
        download_size: 1_024_331_469,
        file_name: "bria-rmbg-2.0.onnx",
        folder: ComponentFolder::Models,
        packaging: Packaging::File { executable: false },
        max_uncompressed_size: 0,
        source: "github.com/danielgatis/rembg",
        owned_entries: &["bria-rmbg-2.0.onnx"],
        update_repository: None,
        update_asset: None,
        update_checksums: None,
        version_arguments: &[],
    },
    // Upstream pins 13.29 from github.com/ShareX/ExifTool, which is gone (the repository
    // answers 404). This is Phil Harvey's own Windows 64-bit build of the current
    // production release, whose SHA-256 exiftool.org publishes in checksums.txt
    // (exiftool.org hands its downloads to SourceForge). Installed as upstream lays it
    // out: `exiftool.exe` (the zip's `exiftool(-k).exe` launcher, renamed so it does not
    // wait for a key) beside its `exiftool_files` folder (Perl and the library). The
    // same x64 build serves arm64, as upstream's win64 pin did.
    Component {
        id: EXIFTOOL,
        name: s::Components_ExifTool,
        purpose: s::Components_ExifToolPurpose,
        version: "13.59",
        url: "https://downloads.sourceforge.net/project/exiftool/exiftool-13.59_64.zip",
        sha256: "44b512b25af500724ba579d0a53c8fc5851628b692dd5e5d94ae4a15c2cba9ec",
        download_size: 11_183_675,
        file_name: "exiftool.exe",
        folder: ComponentFolder::Tools,
        packaging: Packaging::ZipFolder {
            keep: &[
                ("exiftool_files", "exiftool_files"),
                ("exiftool(-k).exe", "exiftool.exe"),
            ],
        },
        max_uncompressed_size: 256 * 1024 * 1024,
        source: "exiftool.org",
        owned_entries: &["exiftool.exe", "exiftool_files"],
        update_repository: None,
        update_asset: None,
        update_checksums: None,
        version_arguments: &[],
    },
];

pub fn find(id: &str) -> Option<&'static Component> {
    catalog().iter().find(|c| c.id.eq_ignore_ascii_case(id))
}

// ------------------------------------------------------------------ where things are

/// `<profile>/Tools` or `<profile>/Models`.
pub fn folder_for(profile: &Path, component: &Component) -> PathBuf {
    match component.folder {
        ComponentFolder::Tools => profile.join(TOOLS_FOLDER),
        ComponentFolder::Models => profile.join(MODELS_FOLDER),
    }
}

/// Where this component would go if it were installed now.
pub fn target_path(profile: &Path, component: &Component) -> PathBuf {
    folder_for(profile, component).join(component.file_name)
}

/// The folder the running executable is in (`Contents/MacOS` in a macOS bundle).
fn beside_executable(file_name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(file_name))
}

/// The component's file, or `None` when it is not on this computer (`InstalledPath`).
///
/// A tool beside the executable counts as installed (`ExternalTools.Locate`), a copy we
/// fetched into the profile too.
pub fn installed_path_of(profile: &Path, component: &Component) -> Option<PathBuf> {
    if component.folder == ComponentFolder::Tools
        && let Some(beside) = beside_executable(component.file_name)
        && beside.is_file()
    {
        return Some(beside);
    }
    let target = target_path(profile, component);
    target.is_file().then_some(target)
}

/// The installed file of the component with this id ("ffmpeg", "yt-dlp", "exiftool",
/// "background-model"), or `None`. What screen recording asks for ffmpeg.
pub fn installed_path(profile: &Path, id: &str) -> Option<PathBuf> {
    find(id).and_then(|component| installed_path_of(profile, component))
}

pub fn is_installed(profile: &Path, component: &Component) -> bool {
    installed_path_of(profile, component).is_some()
}

/// True when the copy in use is one we fetched, and can therefore be removed or replaced.
pub fn is_managed(profile: &Path, component: &Component) -> bool {
    installed_path_of(profile, component).is_some_and(|p| p == target_path(profile, component))
}

// ------------------------------------------------------------------ settings

/// `ApplicationConfig` members of the components.
pub mod settings {
    use super::*;

    /// `ExternalComponentsDeclined`: the components the person said no to.
    pub fn declined(config: &AppConfig) -> Vec<String> {
        config
            .get("ExternalComponentsDeclined")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Stored as the list of components declined, so a component added later starts out
    /// wanted, like a new checkbox on the installer page.
    pub fn is_wanted(config: &AppConfig, component: &Component) -> bool {
        !declined(config)
            .iter()
            .any(|id| id.eq_ignore_ascii_case(component.id))
    }

    /// Returns whether anything changed (the caller saves).
    pub fn set_wanted(config: &mut AppConfig, component: &Component, wanted: bool) -> bool {
        let mut list = declined(config);
        let changed = if wanted {
            let before = list.len();
            list.retain(|id| !id.eq_ignore_ascii_case(component.id));
            list.len() != before
        } else if is_wanted(config, component) {
            list.push(component.id.to_string());
            true
        } else {
            false
        };
        if changed {
            config.set("ExternalComponentsDeclined", json!(list));
        }
        changed
    }

    /// `ExternalComponentsAutoUpdate`, on by default.
    pub fn auto_update(config: &AppConfig) -> bool {
        config
            .get("ExternalComponentsAutoUpdate")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_auto_update(config: &mut AppConfig, value: bool) {
        config.set("ExternalComponentsAutoUpdate", json!(value));
    }

    /// `ExternalComponentsLastUpdateCheck`; `None` for `DateTime.MinValue`.
    pub fn last_update_check(config: &AppConfig) -> Option<chrono::DateTime<chrono::Utc>> {
        config
            .get("ExternalComponentsLastUpdateCheck")
            .and_then(Value::as_str)
            .and_then(|text| crate::dotnet::parse(text, true))
            .filter(|at| !crate::dotnet::is_min(at))
    }

    pub fn set_last_update_check(config: &mut AppConfig, at: chrono::DateTime<chrono::Utc>) {
        config.set(
            "ExternalComponentsLastUpdateCheck",
            json!(crate::dotnet::write_utc(&at)),
        );
    }

    /// `ExternalComponentsVersion`: how far this installation has been carried forward.
    pub fn components_version(config: &AppConfig) -> i64 {
        config
            .get("ExternalComponentsVersion")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }

    pub fn set_components_version(config: &mut AppConfig, value: i64) {
        config.set("ExternalComponentsVersion", json!(value));
    }
}

/// How far `migrate` carries a configuration (`CurrentComponentsVersion`).
pub const CURRENT_COMPONENTS_VERSION: i64 = 1;

/// `MigrateSettings`: once per installation, a custom ffmpeg path that points at nothing
/// (a leftover, or a Windows path that arrived through settings sync) is cleared; a
/// working one is left alone. Returns whether the configuration changed.
pub fn migrate(config: &mut AppConfig) -> bool {
    if settings::components_version(config) >= CURRENT_COMPONENTS_VERSION {
        return false;
    }
    settings::set_components_version(config, CURRENT_COMPONENTS_VERSION);
    let mut tasks = config
        .get("DefaultTaskSettings")
        .cloned()
        .unwrap_or(Value::Null);
    let ffmpeg = tasks
        .get_mut("CaptureSettings")
        .and_then(|c| c.get_mut("FFmpegOptions"));
    if let Some(Value::Object(options)) = ffmpeg {
        let overrides = options
            .get("OverrideCLIPath")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let path = options
            .get("CLIPath")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // Resolved as the recorder resolves it (`GetAbsolutePath`), as upstream checks it.
        let resolved = crate::names::file_helpers::get_absolute_path(&path);
        if overrides && !path.is_empty() && !Path::new(&resolved).is_file() {
            log::info!("Components: dropped a custom ffmpeg path that points at nothing ({path}).");
            options.insert("OverrideCLIPath".into(), json!(false));
            options.insert("CLIPath".into(), json!(""));
            config.set("DefaultTaskSettings", tasks);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_is_pinned_over_https_with_a_size() {
        for component in catalog() {
            assert!(
                net::is_fetchable(component.url),
                "{} is not https",
                component.id
            );
            assert!(
                verify::is_digest(component.sha256),
                "{} has no digest",
                component.id
            );
            assert_eq!(component.sha256, component.sha256.to_lowercase());
            assert!(component.download_size > 0, "{} has no size", component.id);
            assert!(!component.owned_entries.is_empty());
        }
        let ids: Vec<&str> = catalog().iter().map(|c| c.id).collect();
        assert_eq!(ids, [FFMPEG, YT_DLP, BACKGROUND_MODEL, EXIFTOOL]);
    }

    #[test]
    fn only_yt_dlp_follows_its_publisher() {
        let tracking: Vec<&str> = catalog()
            .iter()
            .filter(|c| c.tracks_updates())
            .map(|c| c.id)
            .collect();
        assert_eq!(tracking, [YT_DLP]);
        let yt = find("YT-DLP").unwrap();
        assert_eq!(yt.version_arguments, ["--ignore-config", "--version"]);
        assert!(net::is_artefact_url(yt.url));
    }

    #[test]
    fn installed_means_the_file_is_in_the_profile() {
        let profile = tempfile::tempdir().unwrap();
        let ffmpeg = find(FFMPEG).unwrap();
        assert_eq!(installed_path(profile.path(), FFMPEG), None);
        let target = target_path(profile.path(), ffmpeg);
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        assert_eq!(target, profile.path().join("Tools").join(name));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"x").unwrap();
        assert_eq!(installed_path(profile.path(), "ffmpeg"), Some(target));
        assert!(is_managed(profile.path(), ffmpeg));
        let model = find(BACKGROUND_MODEL).unwrap();
        assert_eq!(
            target_path(profile.path(), model),
            profile.path().join("Models").join("bria-rmbg-2.0.onnx")
        );
        assert_eq!(installed_path(profile.path(), "nothing"), None);
    }

    #[test]
    fn declining_is_remembered_and_undone() {
        let mut config = AppConfig::in_memory();
        let model = find(BACKGROUND_MODEL).unwrap();
        assert!(settings::is_wanted(&config, model));
        assert!(settings::set_wanted(&mut config, model, false));
        assert!(!settings::set_wanted(&mut config, model, false));
        assert!(!settings::is_wanted(&config, model));
        assert_eq!(settings::declined(&config), ["background-model"]);
        assert!(settings::set_wanted(&mut config, model, true));
        assert!(settings::is_wanted(&config, model));
        assert!(settings::auto_update(&config));
        assert_eq!(settings::last_update_check(&config), None);
        config.set(
            "ExternalComponentsLastUpdateCheck",
            json!("0001-01-01T00:00:00"),
        );
        assert_eq!(settings::last_update_check(&config), None);
    }

    #[test]
    fn a_broken_ffmpeg_path_is_cleared_once() {
        let mut config = AppConfig::in_memory();
        config.set(
            "DefaultTaskSettings",
            json!({ "CaptureSettings": { "FFmpegOptions": {
                "OverrideCLIPath": true, "CLIPath": "C:\\Tools\\ffmpeg.exe", "x": 1 } } }),
        );
        assert!(migrate(&mut config));
        let options =
            &config.get("DefaultTaskSettings").unwrap()["CaptureSettings"]["FFmpegOptions"];
        assert_eq!(options["OverrideCLIPath"], json!(false));
        assert_eq!(options["CLIPath"], json!(""));
        assert_eq!(options["x"], json!(1));
        assert!(!migrate(&mut config), "only once");
    }

    /// A working path is left alone, however it is written: the recorder reads it through
    /// `GetAbsolutePath` (folder and environment variables, relative to the app), and so
    /// does upstream's check.
    #[cfg(unix)]
    #[test]
    fn a_working_ffmpeg_path_with_a_variable_is_kept() {
        let Some(tmp) = std::env::var_os("TMPDIR").map(PathBuf::from) else {
            return;
        };
        let dir = tempfile::tempdir_in(&tmp).unwrap();
        std::fs::write(dir.path().join("ffmpeg"), b"#!/bin/sh\n").unwrap();
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let written = format!("%TMPDIR%/{name}/ffmpeg");
        let mut config = AppConfig::in_memory();
        config.set(
            "DefaultTaskSettings",
            json!({ "CaptureSettings": { "FFmpegOptions": {
                "OverrideCLIPath": true, "CLIPath": written } } }),
        );
        assert!(migrate(&mut config));
        let options =
            &config.get("DefaultTaskSettings").unwrap()["CaptureSettings"]["FFmpegOptions"];
        assert_eq!(options["OverrideCLIPath"], json!(true));
        assert_eq!(options["CLIPath"], json!(written));
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_upstreams_pins_for_its_architecture() {
        let ffmpeg = find(FFMPEG).unwrap();
        let yt = find(YT_DLP).unwrap();
        let exiftool = find(EXIFTOOL).unwrap();
        assert_eq!(ffmpeg.file_name, "ffmpeg.exe");
        assert_eq!(yt.file_name, "yt-dlp.exe");
        assert_eq!(exiftool.file_name, "exiftool.exe");
        assert_eq!(exiftool.owned_entries, ["exiftool.exe", "exiftool_files"]);
        if cfg!(target_arch = "aarch64") {
            assert!(ffmpeg.url.ends_with("ffmpeg-8.1-win-arm64.zip"));
            assert_eq!(yt.update_asset, Some("yt-dlp_arm64.exe"));
        } else {
            assert!(ffmpeg.url.ends_with("ffmpeg-8.1-win-x64.zip"));
            assert_eq!(ffmpeg.download_size, 68_599_421);
            assert_eq!(yt.update_asset, Some("yt-dlp.exe"));
            assert_eq!(
                yt.sha256,
                "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"
            );
        }
        assert!(net::is_artefact_url(ffmpeg.url));
        assert!(net::is_artefact_url(yt.url));
    }
}
