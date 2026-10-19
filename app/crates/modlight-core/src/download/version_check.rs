//! Asking how old this yt-dlp is, before anybody says it is too old (`YtDlpVersionCheck`).
//!
//! Runs only on a download that failed with yt-dlp blaming itself. Unknown is a
//! first-class answer: a laptop on a train cannot reach GitHub. A definite answer stands
//! for an hour, "could not find out" for ten minutes; one check at a time.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::error::{Standing, VersionFacts};
use crate::components::{self, tool_release, updates};

const GOOD_FOR: Duration = Duration::from_secs(3600);
const RETRY_AFTER: Duration = Duration::from_secs(600);
const DEADLINE: Duration = Duration::from_secs(20);

static CACHED: Mutex<Option<(VersionFacts, Instant)>> = Mutex::new(None);
static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Drop the remembered answer: the file it was about has been replaced.
pub fn forget() {
    if let Ok(mut cached) = CACHED.lock() {
        *cached = None;
    }
}

fn fresh() -> Option<VersionFacts> {
    let cached = CACHED.lock().ok()?;
    let (facts, at) = cached.as_ref()?;
    let limit = if facts.standing == Standing::Unknown {
        RETRY_AFTER
    } else {
        GOOD_FOR
    };
    (at.elapsed() < limit).then(|| facts.clone())
}

/// What is installed, what is published, and which is newer. Never fails: every path
/// that is not a real comparison is `VersionFacts::unchecked()`.
pub async fn facts(profile: &Path) -> VersionFacts {
    let _guard = GATE.lock().await;
    if let Some(facts) = fresh() {
        return facts;
    }
    let facts = tokio::time::timeout(DEADLINE, measure(profile))
        .await
        .unwrap_or_default();
    log::info!(
        "Download: yt-dlp is {}, the newest published is {} -> {:?}.",
        if facts.installed.is_empty() {
            "an unreadable build"
        } else {
            &facts.installed
        },
        if facts.latest.is_empty() {
            "unknown"
        } else {
            &facts.latest
        },
        facts.standing
    );
    if let Ok(mut cached) = CACHED.lock() {
        *cached = Some((facts.clone(), Instant::now()));
    }
    facts
}

/// The pure verdict from two version strings.
pub fn standing_of(installed: &str, latest: &str) -> Standing {
    if tool_release::parse_version(installed).is_none()
        || tool_release::parse_version(latest).is_none()
    {
        return Standing::Unknown;
    }
    if tool_release::is_newer(latest, installed) {
        Standing::Behind
    } else {
        Standing::Current
    }
}

async fn measure(profile: &Path) -> VersionFacts {
    let Some(component) = components::find(components::YT_DLP) else {
        return VersionFacts::unchecked();
    };
    if !components::is_installed(profile, component) {
        return VersionFacts::unchecked();
    }
    let installed = updates::installed_version(profile, component).await;
    if installed.is_empty() {
        return VersionFacts::unchecked();
    }
    let mut latest = updates::latest_known(component);
    if latest.is_empty() {
        let fetch = updates::check(component).await;
        match fetch.build {
            Some(build) => latest = build.version_text,
            None => {
                return VersionFacts {
                    standing: Standing::Unknown,
                    installed,
                    latest: String::new(),
                };
            }
        }
    }
    VersionFacts {
        standing: standing_of(&installed, &latest),
        installed,
        latest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdict_needs_two_readable_numbers() {
        assert_eq!(standing_of("2026.06.14", "2026.08.19"), Standing::Behind);
        assert_eq!(standing_of("2026.08.19", "2026.08.19"), Standing::Current);
        assert_eq!(standing_of("2026.08.19", ""), Standing::Unknown);
        assert_eq!(standing_of("", "2026.08.19"), Standing::Unknown);
    }
}
