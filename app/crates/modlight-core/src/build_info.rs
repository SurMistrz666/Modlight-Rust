//! What this build is: the version (Cargo), the build (the commit's short hash, empty when
//! it was built outside git) and when it was built (UTC).
//!
//! `build.rs` sets them at compile time. The build scripts (`app/scripts/build-app.sh`,
//! `build-app.ps1`) pass `MODLIGHT_BUILD` and `MODLIGHT_BUILT` to cargo and publish the zip
//! with the same three values, so a published build knows it is that build; any other
//! build takes the commit from git and the time from the clock. The updater compares
//! them with the server's builds (`update::is_newer`); About and the update question show
//! them.

use chrono::{DateTime, Utc};

/// The version of the workspace, e.g. `0.7.7`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit's short hash (`git rev-parse --short=8 HEAD`), or empty.
pub const BUILD: &str = env!("MODLIGHT_BUILD_ID");
/// When it was built, `2026-10-27T14:03:05Z`.
pub const BUILT: &str = env!("MODLIGHT_BUILT_AT");

/// The system a build is for, as the server's Downloads name it.
pub const PLATFORM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

/// The processor a build is for, as the server's Downloads name it.
pub const ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else if cfg!(target_arch = "x86_64") {
    "x64"
} else {
    std::env::consts::ARCH
};

/// One build's identity: this one ([`BuildIdentity::current`]) or one on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildIdentity {
    pub version: String,
    pub build: String,
    pub built: Option<DateTime<Utc>>,
}

impl BuildIdentity {
    /// The running build.
    pub fn current() -> BuildIdentity {
        BuildIdentity {
            version: VERSION.to_string(),
            build: BUILD.to_string(),
            built: built_at(),
        }
    }

    /// `0.7.7 (1a2b3c4d)`, or `0.7.7` without a commit.
    pub fn label(&self) -> String {
        version_label(&self.version, &self.build)
    }
}

/// When this build was made.
pub fn built_at() -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(BUILT)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// `0.7.7 (1a2b3c4d)`, or `0.7.7` without a commit: the version a person sees beside
/// another build of the same version.
pub fn version_label(version: &str, build: &str) -> String {
    if build.is_empty() {
        version.to_string()
    } else {
        format!("{version} ({build})")
    }
}

/// `1a2b3c4d, 2026-10-27 14:03 UTC`, or only the time without a commit (About's line).
pub fn build_text(build: &str, built: Option<DateTime<Utc>>) -> String {
    let time = built
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();
    match (build.is_empty(), time.is_empty()) {
        (false, false) => format!("{build}, {time}"),
        (false, true) => build.to_string(),
        (true, _) => time,
    }
}

/// The zip the build scripts publish: `Modlight-<version>-<build>-<platform>-<arch>.zip`
/// (the build is empty outside git, which leaves two dashes, as the scripts write it).
pub fn zip_name(version: &str, build: &str, platform: &str, arch: &str) -> String {
    format!("Modlight-{version}-{build}-{platform}-{arch}.zip")
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn this_build_knows_what_it_is() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert!(
            built_at().is_some(),
            "build.rs wrote a time that does not parse: {BUILT}"
        );
        assert!(BUILD.len() <= 40);
        let current = BuildIdentity::current();
        assert_eq!(current.version, VERSION);
        assert_eq!(current.built, built_at());
        assert!(matches!(PLATFORM, "macos" | "windows" | "linux"));
        if cfg!(target_arch = "aarch64") {
            assert_eq!(ARCH, "arm64");
        }
    }

    #[test]
    fn labels_and_names() {
        assert_eq!(version_label("0.7.7", "1a2b3c4d"), "0.7.7 (1a2b3c4d)");
        assert_eq!(version_label("0.7.7", ""), "0.7.7");
        let at = Utc.with_ymd_and_hms(2026, 10, 27, 14, 3, 5).unwrap();
        assert_eq!(
            build_text("1a2b3c4d", Some(at)),
            "1a2b3c4d, 2026-10-27 14:03 UTC"
        );
        assert_eq!(build_text("", Some(at)), "2026-10-27 14:03 UTC");
        assert_eq!(build_text("1a2b3c4d", None), "1a2b3c4d");
        assert_eq!(
            zip_name("0.7.7", "1a2b3c4d", "macos", "arm64"),
            "Modlight-0.7.7-1a2b3c4d-macos-arm64.zip"
        );
        assert_eq!(
            zip_name("0.7.7", "1a2b3c4d", "windows", "x64"),
            "Modlight-0.7.7-1a2b3c4d-windows-x64.zip"
        );
    }
}
