//! Keeping yt-dlp current (`ModlightComponentUpdates`).
//!
//! Only yt-dlp, because ffmpeg can sit still for a year and yt-dlp cannot sit still for a
//! month. The schedule is a day (`ExternalComponentsLastUpdateCheck`), 45 seconds after
//! start; a failed check waits half an hour, in memory only. Nothing here ever stands in
//! front of a download. What is fetched is checked: the build comes from
//! [`tool_release::fetch`], which will not hand back anything without BOTH published
//! digests, and it is installed through the same staged, hashed path as a first install.
//!
//! The settings live in the app's configuration, which the window owns; this module
//! answers "is it due", runs the check, and hands back what the window records.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use super::install::{self, InstallOutcome, ProgressSink};
use super::tool_release::{self, FetchResult, FetchStatus};
use super::{Component, ComponentSource, is_installed, is_managed, net};

/// How long a check is good for.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// How long a failed check waits before it is worth trying again.
pub const FAILURE_BACKOFF: Duration = Duration::from_secs(30 * 60);
/// How long after launch the first check happens.
pub const STARTUP_DELAY: Duration = Duration::from_secs(45);
/// Long enough for a cold PyInstaller binary to unpack itself and print one line.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// A newer build is installed now.
    Updated,
    /// The newest published build is already the one here.
    AlreadyCurrent,
    /// The check did not complete, or answered with something we would not act on.
    CheckFailed,
    /// There is a newer build and it could not be put in place.
    InstallFailed,
    /// This component does not follow a publisher.
    NotTracked,
    /// It is not installed, or the copy in use is not one we may replace.
    NotOurs,
    /// What is on the disk could not be identified, so nothing was compared.
    VersionUnknown,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateResult {
    pub outcome: UpdateOutcome,
    pub version: String,
    pub detail: String,
}

impl UpdateResult {
    fn new(outcome: UpdateOutcome, version: &str, detail: &str) -> UpdateResult {
        UpdateResult {
            outcome,
            version: version.to_string(),
            detail: detail.to_string(),
        }
    }

    pub fn installed(&self) -> bool {
        self.outcome == UpdateOutcome::Updated
    }
}

/// What each binary answered, against path, length and write time.
static VERSIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
/// The newest build each publisher had, as of the last check this run. Memory only.
static LATEST: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
/// One check at a time, whoever asked.
static CHECK_GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// The newest build the publisher had when we last asked, or empty.
pub fn latest_known(component: &Component) -> String {
    LATEST
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(component.id).cloned()))
        .unwrap_or_default()
}

pub fn remember_latest(component: &Component, version: &str) {
    if let Ok(mut map) = LATEST.lock() {
        map.get_or_insert_with(HashMap::new)
            .insert(component.id.to_string(), version.to_string());
    }
}

/// Drop the remembered versions: a file has just been replaced.
pub fn forget_versions() {
    if let Ok(mut map) = VERSIONS.lock() {
        *map = None;
    }
}

/// True when this component has a publisher to follow and a copy we may replace.
pub fn can_update(profile: &Path, component: &Component) -> bool {
    component.tracks_updates() && is_installed(profile, component) && is_managed(profile, component)
}

/// Ask the publisher what the newest build is, and remember the answer.
pub async fn check(component: &Component) -> FetchResult {
    let (Some(repository), Some(asset)) = (component.update_repository, component.update_asset)
    else {
        return FetchResult::malformed("not tracked");
    };
    let origin = net::github_api_origin();
    let fetch = tool_release::fetch(
        repository,
        asset,
        component.update_checksums.unwrap_or(""),
        origin.as_deref(),
    )
    .await;
    match (&fetch.status, &fetch.build) {
        (FetchStatus::Ok, Some(build)) => remember_latest(component, &build.version_text),
        _ => log::info!(
            "Components: could not check {} ({}).",
            component.id,
            fetch.detail
        ),
    }
    fetch
}

/// Which build is actually on this Mac, by asking the binary (`InstalledVersionAsync`).
/// Cached against the file itself; anything that is not a version is no answer.
pub async fn installed_version(profile: &Path, component: &Component) -> String {
    if component.version_arguments.is_empty() {
        // Nothing to ask, so the pin is the answer.
        return if is_installed(profile, component) {
            component.version.to_string()
        } else {
            String::new()
        };
    }
    let Some(path) = super::installed_path_of(profile, component) else {
        return String::new();
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return String::new();
    };
    let stamp = format!(
        "{}|{}|{}",
        path.display(),
        meta.len(),
        meta.modified()
            .ok()
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    if let Some(cached) = VERSIONS
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(&stamp).cloned()))
    {
        return cached;
    }
    let arguments: Vec<String> = component
        .version_arguments
        .iter()
        .map(|a| a.to_string())
        .collect();
    let version = ask_version(&path, &arguments, VERSION_TIMEOUT).await;
    if let Ok(mut map) = VERSIONS.lock() {
        map.get_or_insert_with(HashMap::new)
            .insert(stamp, version.clone());
    }
    version
}

/// Run `path` with `arguments` and read the first line of what it prints, if that line
/// is a date-shaped version. Killed (with its group) after `timeout`.
pub async fn ask_version(path: &Path, arguments: &[String], timeout: Duration) -> String {
    let mut command = super::process::command(path, arguments);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            log::info!(
                "Components: could not ask {} for its version: {error}",
                path.display()
            );
            return String::new();
        }
    };
    let guard = child.id().map(super::process::GroupGuard::register);
    let output = tokio::time::timeout(timeout, child.wait_with_output()).await;
    let output = match output {
        Ok(Ok(output)) => output,
        _ => {
            if let Some(guard) = &guard {
                guard.kill();
            }
            return String::new();
        }
    };
    drop(guard);
    if !output.status.success() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.split('\n') {
        let candidate = line.trim();
        if !candidate.is_empty() {
            return if tool_release::parse_version(candidate).is_some() {
                candidate.to_string()
            } else {
                String::new()
            };
        }
    }
    String::new()
}

/// Ask the publisher, and install a newer build if there is one (`UpdateAsync`).
///
/// `only_when_newer` is true for anything automatic: a build is installed only when the
/// version on the disk can be read AND the published one is strictly newer. The manual
/// button passes false.
pub async fn update(
    profile: &Path,
    component: &Component,
    progress: Option<ProgressSink>,
    only_when_newer: bool,
    cancel: &CancellationToken,
) -> UpdateResult {
    if !component.tracks_updates() {
        return UpdateResult::new(UpdateOutcome::NotTracked, "", "");
    }
    if !is_installed(profile, component) {
        return UpdateResult::new(UpdateOutcome::NotOurs, "", "");
    }
    if !is_managed(profile, component) {
        let path = super::installed_path_of(profile, component)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        return UpdateResult::new(UpdateOutcome::NotOurs, "", &path);
    }
    let installed = installed_version(profile, component).await;
    let fetch = check(component).await;
    let Some(build) = fetch.build.filter(|_| fetch.status == FetchStatus::Ok) else {
        return UpdateResult::new(UpdateOutcome::CheckFailed, &installed, &fetch.detail);
    };
    if installed.is_empty() {
        if only_when_newer {
            log::info!(
                "Components: {} did not say what version it is, so nothing was replaced automatically.",
                component.id
            );
            return UpdateResult::new(UpdateOutcome::VersionUnknown, "", &build.version_text);
        }
    } else if !tool_release::is_newer(&build.version_text, &installed) {
        return UpdateResult::new(UpdateOutcome::AlreadyCurrent, &installed, "");
    }
    log::info!(
        "Components: updating {} from {} to {}, digest from {}.",
        component.id,
        if installed.is_empty() {
            "an unknown build"
        } else {
            &installed
        },
        build.version_text,
        build.digest_source
    );
    let source = ComponentSource {
        url: build.url.clone(),
        sha256: build.sha256.clone(),
        size: build.size,
        version: build.version_text.clone(),
    };
    let result = install::replace(profile, component, &source, progress, cancel).await;
    if result.outcome == InstallOutcome::Cancelled {
        return UpdateResult::new(UpdateOutcome::Cancelled, &installed, "");
    }
    if !result.success() {
        return UpdateResult::new(UpdateOutcome::InstallFailed, &installed, &result.message);
    }
    forget_versions();
    crate::download::version_check::forget();
    UpdateResult::new(UpdateOutcome::Updated, &build.version_text, "")
}

/// When the last check failed, in memory only (`lastAttemptUtc`).
static LAST_ATTEMPT: Mutex<Option<Instant>> = Mutex::new(None);

/// Whether it is time to ask again. A stamp in the future is due rather than trusted.
pub fn is_due(last_check: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    if LAST_ATTEMPT
        .lock()
        .ok()
        .and_then(|a| *a)
        .is_some_and(|at| at.elapsed() < FAILURE_BACKOFF)
    {
        return false;
    }
    is_due_by_stamp(last_check, now)
}

/// The stamp half of [`is_due`].
pub fn is_due_by_stamp(last_check: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match last_check {
        None => true,
        Some(last) if last > now => true,
        Some(last) => (now - last).to_std().unwrap_or_default() >= CHECK_INTERVAL,
    }
}

/// What the scheduled run needs from the settings.
#[derive(Clone, Debug)]
pub struct ScheduledPlan {
    pub auto_update: bool,
    pub last_check: Option<DateTime<Utc>>,
    /// Ids of the components the person wants.
    pub wanted: Vec<String>,
}

/// What a scheduled run found: whether a check completed (stamp the settings), and the
/// outcome per component.
#[derive(Clone, Debug, Default)]
pub struct ScheduledReport {
    pub ran: bool,
    pub checked_any: bool,
    pub results: Vec<(String, UpdateOutcome, String)>,
}

/// `RunScheduledAsync`: if it is due, ask for every tracked, wanted, installed component;
/// install a newer build when automatic updates are on and the copy is ours.
pub async fn run_scheduled(profile: &Path, plan: &ScheduledPlan) -> ScheduledReport {
    let mut report = ScheduledReport::default();
    let Ok(_permit) = CHECK_GATE.try_acquire() else {
        return report;
    };
    if !is_due(plan.last_check, Utc::now()) {
        return report;
    }
    report.ran = true;
    if let Ok(mut at) = LAST_ATTEMPT.lock() {
        *at = Some(Instant::now());
    }
    let cancel = CancellationToken::new();
    for component in super::catalog() {
        if !component.tracks_updates()
            || !plan
                .wanted
                .iter()
                .any(|id| id.eq_ignore_ascii_case(component.id))
            || !is_installed(profile, component)
        {
            continue;
        }
        if !plan.auto_update || !is_managed(profile, component) {
            // Still ask, so the settings page can say a newer build exists.
            let fetch = check(component).await;
            report.checked_any |= fetch.status == FetchStatus::Ok;
            continue;
        }
        let result = update(profile, component, None, true, &cancel).await;
        report.checked_any |= result.outcome != UpdateOutcome::CheckFailed;
        log::info!(
            "Components: scheduled check for {} says {:?}{}.",
            component.id,
            result.outcome,
            if result.version.is_empty() {
                String::new()
            } else {
                format!(" ({})", result.version)
            }
        );
        report
            .results
            .push((component.id.to_string(), result.outcome, result.version));
    }
    // A completed check is stamped into the settings by the caller.
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_day_between_checks_and_a_future_stamp_is_due() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        assert!(is_due_by_stamp(None, now));
        assert!(!is_due_by_stamp(
            Some(now - chrono::Duration::hours(23)),
            now
        ));
        assert!(is_due_by_stamp(
            Some(now - chrono::Duration::hours(24)),
            now
        ));
        assert!(is_due_by_stamp(
            Some(now + chrono::Duration::days(300)),
            now
        ));
    }

    #[tokio::test]
    async fn the_version_is_read_from_the_binary_with_an_argument_vector() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("fake-yt-dlp");
        let log = dir.path().join("args");
        std::fs::write(
            &tool,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\" >> '{}'; done\necho 2026.08.19\n",
                log.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let arguments = vec!["--ignore-config".to_string(), "--version".to_string()];
        let version = ask_version(&tool, &arguments, Duration::from_secs(10)).await;
        assert_eq!(version, "2026.08.19");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "--ignore-config\n--version\n"
        );
        // A banner is not a version.
        std::fs::write(&tool, "#!/bin/sh\necho yt-dlp is great\n").unwrap();
        assert_eq!(
            ask_version(&tool, &arguments, Duration::from_secs(10)).await,
            ""
        );
        // A binary that hangs is given up on.
        std::fs::write(&tool, "#!/bin/sh\nsleep 30\necho 2026.08.19\n").unwrap();
        let started = Instant::now();
        assert_eq!(
            ask_version(&tool, &arguments, Duration::from_millis(300)).await,
            ""
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
