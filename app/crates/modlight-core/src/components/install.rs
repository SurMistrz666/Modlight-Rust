//! Getting a component onto this Mac: download, verify, unpack, put it where the rest
//! of the app looks for it; removing it again; tidying up after crashes
//! (`ModlightComponents` installing and plumbing).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::download::{self, DownloadOutcome, DownloadProgress};
use super::{Component, ComponentSource, Packaging, archive, folder_for, target_path, verify};
use crate::format::local_size;

/// Staging folder, inside the destination folder so a finished file is a move.
pub const STAGING_FOLDER: &str = ".incoming";

/// What a replaced file is renamed to on its way out, followed by a timestamp.
pub const RETIRED_SUFFIX: &str = ".old-";

/// Headroom demanded on top of the download itself, for the unpack.
pub const SPACE_HEADROOM: u64 = 64 * 1024 * 1024;

/// Part files older than this are a version nobody will ask for again.
pub const PART_LIFETIME: Duration = Duration::from_secs(14 * 24 * 3600);

/// One download at a time, whoever asked.
static GATE: Semaphore = Semaphore::const_new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    /// Another component is being installed.
    Queued,
    Downloading,
    Verifying,
    Installing,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstallProgress {
    pub stage: Stage,
    pub received: i64,
    pub total: i64,
    pub bytes_per_second: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed,
    AlreadyInstalled,
    Cancelled,
    /// Nothing answered.
    Offline,
    /// It arrived and it was not what was promised. Nothing was installed.
    ChecksumFailed,
    /// Not enough room on the disk for it.
    NoSpace,
    /// Downloaded, but unpacking or moving it into place failed.
    InstallFailed,
    /// The server said no, or the body stopped short.
    DownloadFailed,
    /// The URL was not https, so nothing was requested.
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallResult {
    pub outcome: InstallOutcome,
    pub message: String,
}

impl InstallResult {
    pub fn new(outcome: InstallOutcome, message: impl Into<String>) -> InstallResult {
        InstallResult {
            outcome,
            message: message.into(),
        }
    }

    pub fn success(&self) -> bool {
        matches!(
            self.outcome,
            InstallOutcome::Installed | InstallOutcome::AlreadyInstalled
        )
    }
}

pub type ProgressSink = Arc<dyn Fn(InstallProgress) + Send + Sync>;

fn report(sink: &Option<ProgressSink>, stage: Stage, received: i64, total: i64, speed: f64) {
    if let Some(sink) = sink {
        sink(InstallProgress {
            stage,
            received,
            total,
            bytes_per_second: speed,
        });
    }
}

/// `InstallAsync`: the pinned build, unless it is already here.
pub async fn install(
    profile: &Path,
    component: &Component,
    progress: Option<ProgressSink>,
    cancel: &CancellationToken,
) -> InstallResult {
    fetch(
        profile,
        component,
        &ComponentSource::pinned(component),
        false,
        progress,
        cancel,
    )
    .await
}

/// `ReplaceAsync`: a specific published build in place of whatever is there now.
pub async fn replace(
    profile: &Path,
    component: &Component,
    source: &ComponentSource,
    progress: Option<ProgressSink>,
    cancel: &CancellationToken,
) -> InstallResult {
    fetch(profile, component, source, true, progress, cancel).await
}

async fn fetch(
    profile: &Path,
    component: &Component,
    source: &ComponentSource,
    replace: bool,
    progress: Option<ProgressSink>,
    cancel: &CancellationToken,
) -> InstallResult {
    if !replace && super::is_installed(profile, component) {
        return InstallResult::new(InstallOutcome::AlreadyInstalled, "");
    }
    report(&progress, Stage::Queued, 0, source.size, 0.0);
    let _permit = tokio::select! {
        _ = cancel.cancelled() => return InstallResult::new(InstallOutcome::Cancelled, ""),
        permit = GATE.acquire() => match permit {
            Ok(permit) => permit,
            Err(error) => return InstallResult::new(InstallOutcome::InstallFailed, error.to_string()),
        },
    };
    install_core(profile, component, source, progress, cancel).await
}

/// A part file's name: the digest in it makes a leftover from an older pin impossible to
/// mistake for a resumable download of this one.
pub fn part_name(component: &Component, sha256: &str) -> String {
    let short: String = sha256.chars().take(12).collect();
    format!("{}-{short}.part", component.id)
}

async fn install_core(
    profile: &Path,
    component: &Component,
    source: &ComponentSource,
    progress: Option<ProgressSink>,
    cancel: &CancellationToken,
) -> InstallResult {
    let folder = folder_for(profile, component);
    let staging = folder.join(STAGING_FOLDER);
    if let Err(error) = std::fs::create_dir_all(&staging) {
        return InstallResult::new(InstallOutcome::InstallFailed, error.to_string());
    }
    if let Some(space) = check_space(&folder, source.size.max(0) as u64 + SPACE_HEADROOM) {
        return space;
    }
    let part = staging.join(part_name(component, &source.sha256));
    discard_other_parts(&staging, component, &part);
    log::info!(
        "Components: fetching {} {} from {}",
        component.id,
        source.version,
        source.url
    );
    report(&progress, Stage::Downloading, 0, source.size, 0.0);

    let client = match super::net::file_client() {
        Ok(client) => client,
        Err(error) => return InstallResult::new(InstallOutcome::DownloadFailed, error),
    };
    let sink = progress.clone();
    let on_progress = move |value: DownloadProgress| {
        report(
            &sink,
            Stage::Downloading,
            value.received,
            value.total,
            value.bytes_per_second,
        );
    };
    let result = download::to_file(
        &client,
        &source.url,
        &part,
        source.size,
        download::STALL_TIMEOUT,
        &on_progress,
        cancel,
    )
    .await;
    if !result.completed() {
        let outcome = match result.outcome {
            DownloadOutcome::Cancelled => InstallOutcome::Cancelled,
            DownloadOutcome::Offline => InstallOutcome::Offline,
            DownloadOutcome::WriteFailed => InstallOutcome::InstallFailed,
            DownloadOutcome::Refused => InstallOutcome::Refused,
            _ => InstallOutcome::DownloadFailed,
        };
        return InstallResult::new(outcome, result.message);
    }

    report(&progress, Stage::Verifying, source.size, source.size, 0.0);
    let check_part = part.clone();
    let expected = source.sha256.clone();
    let verification =
        match tokio::task::spawn_blocking(move || verify::verify_or_delete(&check_part, &expected))
            .await
        {
            Ok(verification) => verification,
            Err(error) => {
                return InstallResult::new(InstallOutcome::InstallFailed, error.to_string());
            }
        };
    if !verification.matches {
        log::warn!(
            "Components: REJECTED {}. Expected {}, got {}. {}",
            component.id,
            source.sha256,
            if verification.actual.is_empty() {
                "nothing"
            } else {
                &verification.actual
            },
            verification.error.clone().unwrap_or_default()
        );
        return InstallResult::new(InstallOutcome::ChecksumFailed, verification.actual);
    }
    log::info!(
        "Components: checksum verified for {} ({}).",
        component.id,
        verification.actual
    );

    report(&progress, Stage::Installing, source.size, source.size, 0.0);
    let (place_profile, place_component, place_part) =
        (profile.to_path_buf(), component.clone(), part.clone());
    let placed =
        tokio::task::spawn_blocking(move || place(&place_profile, &place_component, &place_part))
            .await
            .unwrap_or_else(|error| {
                InstallResult::new(InstallOutcome::InstallFailed, error.to_string())
            });
    if placed.success() {
        report(&progress, Stage::Done, source.size, source.size, 0.0);
        log::info!(
            "Components: installed {} {} to {}",
            component.id,
            source.version,
            target_path(profile, component).display()
        );
        sweep_retired(profile, component);
    }
    placed
}

/// Move what was downloaded into the folder the app reads (`Place`).
pub fn place(profile: &Path, component: &Component, part: &Path) -> InstallResult {
    let folder = folder_for(profile, component);
    let staging = folder.join(STAGING_FOLDER);
    let unpack = staging.join(format!("{}-unpack", component.id));
    let target = target_path(profile, component);
    let outcome = (|| -> Result<Option<InstallResult>, String> {
        match component.packaging {
            Packaging::File { executable } => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                set_mode(part, if executable { 0o755 } else { 0o644 });
                move_into_place(part, &target)?;
            }
            Packaging::ZipSingleFile => {
                if let Some(space) = check_space(&folder, zip_size(part) + SPACE_HEADROOM) {
                    return Ok(Some(space));
                }
                reset(&unpack)?;
                let name = Path::new(component.file_name)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let extracted = archive::extract_zip_single(
                    part,
                    &name,
                    &unpack,
                    component.max_uncompressed_size,
                )?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                move_into_place(&extracted, &target)?;
            }
            Packaging::TarGzFolder { keep, executables } => {
                if let Some(space) = check_space(
                    &folder,
                    component.max_uncompressed_size.min(1 << 30) + SPACE_HEADROOM,
                ) {
                    return Ok(Some(space));
                }
                reset(&unpack)?;
                let Some(owned) = component.owned_entries.first() else {
                    return Err("no folder to install into".into());
                };
                let built = unpack.join(owned);
                archive::extract_tar_gz_folder(
                    part,
                    keep,
                    executables,
                    &built,
                    component.max_uncompressed_size,
                )?;
                move_folder_into_place(&built, &folder.join(owned))?;
            }
        }
        Ok(None)
    })();
    // The unpacked copy always goes: it duplicates something installed or abandoned.
    let _ = std::fs::remove_dir_all(&unpack);
    match outcome {
        Ok(Some(refusal)) => return refusal,
        Ok(None) => {}
        Err(error) => {
            log::warn!(
                "Components: could not put {} in place: {error}",
                component.id
            );
            return InstallResult::new(InstallOutcome::InstallFailed, error);
        }
    }
    // Say it worked only if the file is actually there.
    if !target.is_file() {
        return InstallResult::new(
            InstallOutcome::InstallFailed,
            format!("{} is not there afterwards", component.file_name),
        );
    }
    // Only now: everything that can fail after the download is worth retrying with the
    // verified bytes still in the staging folder.
    verify::try_delete(part);
    InstallResult::new(InstallOutcome::Installed, "")
}

/// Delete a component we fetched. Never touches a copy beside the executable.
pub fn remove(profile: &Path, component: &Component) -> bool {
    let folder = folder_for(profile, component);
    let mut removed = false;
    for name in component.owned() {
        let path = folder.join(name);
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path).map(|_| true)
        } else if path.exists() {
            std::fs::remove_file(&path).map(|_| true)
        } else {
            Ok(false)
        };
        match result {
            Ok(gone) => removed |= gone,
            Err(error) => {
                log::warn!("Components: could not remove {}: {error}", component.id);
                return false;
            }
        }
    }
    if removed {
        log::info!(
            "Components: removed {} from {}",
            component.id,
            folder.display()
        );
    }
    removed
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

fn stamp() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M%S%3f").to_string()
}

/// Put a verified file where the app reads it, over whatever is there. The old one is
/// renamed out of the way first, so the folder never ends up holding neither.
pub fn move_into_place(source: &Path, destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return std::fs::rename(source, destination).map_err(|e| e.to_string());
    }
    let retired = retired_name(destination);
    if std::fs::rename(destination, &retired).is_err() {
        // Try the plain replace and let it be the error if it is one.
        return std::fs::rename(source, destination).map_err(|e| e.to_string());
    }
    if let Err(error) = std::fs::rename(source, destination) {
        if let Err(restore) = std::fs::rename(&retired, destination) {
            log::warn!("Components: could not put the old file back: {restore}");
        }
        return Err(error.to_string());
    }
    verify::try_delete(&retired);
    Ok(())
}

/// The same for a folder (ExifTool's script and library).
fn move_folder_into_place(source: &Path, destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return std::fs::rename(source, destination).map_err(|e| e.to_string());
    }
    let retired = retired_name(destination);
    std::fs::rename(destination, &retired).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::rename(source, destination) {
        let _ = std::fs::rename(&retired, destination);
        return Err(error.to_string());
    }
    let _ = std::fs::remove_dir_all(&retired);
    Ok(())
}

fn retired_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(RETIRED_SUFFIX);
    name.push(stamp());
    PathBuf::from(name)
}

/// Clear what a replacement renamed out of the way: by exact name (an owned entry, the
/// suffix, digits), and only while the live one is there.
pub fn sweep_retired(profile: &Path, component: &Component) {
    let folder = folder_for(profile, component);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for owned in component.owned() {
        if !folder.join(owned).exists() {
            continue;
        }
        let prefix = format!("{owned}{RETIRED_SUFFIX}");
        for name in &names {
            let Some(tail) = name.strip_prefix(&prefix) else {
                continue;
            };
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                let path = folder.join(name);
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    verify::try_delete(&path);
                }
            }
        }
    }
}

/// `SweepStaging`: unpacked folders go every time, part files once a fortnight old.
pub fn sweep_staging(profile: &Path) {
    for folder in [
        profile.join(super::TOOLS_FOLDER),
        profile.join(super::MODELS_FOLDER),
    ] {
        let staging = folder.join(STAGING_FOLDER);
        let Ok(entries) = std::fs::read_dir(&staging) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else if path.extension().is_some_and(|e| e == "part") {
                let old = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| SystemTime::now().duration_since(m).ok())
                    .is_some_and(|age| age > PART_LIFETIME);
                if old {
                    verify::try_delete(&path);
                }
            }
        }
    }
}

fn discard_other_parts(staging: &Path, component: &Component, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(staging) else {
        return;
    };
    let prefix = format!("{}-", component.id);
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name.ends_with(".part") && path != keep {
            verify::try_delete(&path);
        }
    }
}

fn reset(directory: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(directory);
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())
}

fn zip_size(path: &Path) -> u64 {
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return 0;
    };
    (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|e| e.size()))
        .sum()
}

/// Free bytes on the volume holding `folder` (the nearest existing ancestor).
pub fn free_space(folder: &Path) -> Option<u64> {
    let mut probe = folder.to_path_buf();
    while !probe.exists() {
        if !probe.pop() {
            return None;
        }
    }
    volume_free_bytes(&probe)
}

#[cfg(unix)]
fn volume_free_bytes(probe: &Path) -> Option<u64> {
    let path = std::ffi::CString::new(probe.as_os_str().as_encoded_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid NUL-terminated string and `stat` a writable struct.
    let result = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
    if result != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// The bytes this user may still write there (`GetDiskFreeSpaceEx`, quotas included).
#[cfg(windows)]
fn volume_free_bytes(probe: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available = 0u64;
    // SAFETY: a NUL-terminated wide path and one writable counter; the other two
    // counters are not asked for.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

/// A drive that cannot be measured is not a drive that is full.
fn check_space(folder: &Path, needed: u64) -> Option<InstallResult> {
    let free = free_space(folder)?;
    (free < needed)
        .then(|| InstallResult::new(InstallOutcome::NoSpace, local_size(needed as i64, false, 2)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::find;

    #[test]
    fn a_replaced_file_never_leaves_the_folder_empty() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("yt-dlp");
        std::fs::write(&live, b"old").unwrap();
        let fresh = dir.path().join("new.part");
        std::fs::write(&fresh, b"new").unwrap();
        move_into_place(&fresh, &live).unwrap();
        assert_eq!(std::fs::read(&live).unwrap(), b"new");
        assert!(!fresh.exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    }

    #[test]
    fn retired_copies_go_only_by_their_exact_name() {
        let profile = tempfile::tempdir().unwrap();
        let tools = profile.path().join("Tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("yt-dlp"), b"live").unwrap();
        std::fs::write(tools.join("yt-dlp.old-20260101120000123"), b"old").unwrap();
        std::fs::write(tools.join("yt-dlp.old-notes"), b"mine").unwrap();
        sweep_retired(profile.path(), find("yt-dlp").unwrap());
        assert!(!tools.join("yt-dlp.old-20260101120000123").exists());
        assert!(tools.join("yt-dlp.old-notes").exists());
        assert!(tools.join("yt-dlp").exists());
    }

    #[test]
    fn a_retired_copy_stays_when_it_is_the_only_one() {
        let profile = tempfile::tempdir().unwrap();
        let tools = profile.path().join("Tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("yt-dlp.old-20260101120000123"), b"old").unwrap();
        sweep_retired(profile.path(), find("yt-dlp").unwrap());
        assert!(tools.join("yt-dlp.old-20260101120000123").exists());
    }

    #[test]
    fn staging_keeps_young_parts_and_drops_unpacked_folders() {
        let profile = tempfile::tempdir().unwrap();
        let staging = profile.path().join("Tools").join(STAGING_FOLDER);
        std::fs::create_dir_all(staging.join("ffmpeg-unpack")).unwrap();
        std::fs::write(staging.join("ffmpeg-abc.part"), b"half").unwrap();
        sweep_staging(profile.path());
        assert!(!staging.join("ffmpeg-unpack").exists());
        assert!(staging.join("ffmpeg-abc.part").exists());
    }

    #[test]
    fn removal_takes_every_owned_entry() {
        let profile = tempfile::tempdir().unwrap();
        let exiftool = find("exiftool").unwrap();
        let folder = profile.path().join("Tools").join("ExifTool");
        std::fs::create_dir_all(folder.join("lib")).unwrap();
        std::fs::write(folder.join("exiftool"), b"#!").unwrap();
        assert!(crate::components::is_installed(profile.path(), exiftool));
        assert!(remove(profile.path(), exiftool));
        assert!(!folder.exists());
        assert!(!remove(profile.path(), exiftool));
    }

    #[test]
    fn the_part_name_carries_the_digest() {
        let yt = find("yt-dlp").unwrap();
        assert_eq!(
            part_name(yt, "abcdef0123456789"),
            "yt-dlp-abcdef012345.part"
        );
    }

    #[test]
    fn free_space_is_measured() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(&dir.path().join("not/yet/there")).unwrap_or(0) > 0);
    }
}
