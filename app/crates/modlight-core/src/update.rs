//! The app's own update check (`ModlightUpdateChecker`, `ModlightRelease`,
//! `ModlightReleaseClient`, the downloader's verify step).
//!
//! Off unless the build was given a release host: `MODLIGHT_DEFAULT_HOST` at BUILD time
//! (`ModlightDefaultHost`), which must start with `https://` and must not end with a
//! slash - a bad value fails the build, as upstream's MSBuild check does. The host is a
//! constant and never the server somebody signed in to: the sign-in host answers "where
//! do my files live", this one "who may hand me an executable".
//!
//! Upstream downloads its Inno Setup `.exe` and runs it elevated. The port downloads the
//! release, refuses it unless its size and SHA-256 match the manifest, and reveals the
//! verified file (in Explorer on Windows, in Finder on a Mac); nothing is opened,
//! mounted, installed or run, the person starts it. On Windows a release is the
//! installer `.exe` (upstream's) or a `.zip`; on a Mac a `.dmg` or a `.zip` of
//! `Modlight.app`. A manifest whose file is not one of those is not a release for this
//! system and is answered "up to date". The server publishes one release
//! (`/api/releases/app`), so a `.zip` there reaches both systems: a per-platform
//! manifest is a server change.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::components::download::{self, DownloadOutcome, DownloadProgress};
use crate::components::{net, verify};

/// The release host this build was given, or empty (updates off).
pub const RELEASE_HOST: &str = match option_env!("MODLIGHT_DEFAULT_HOST") {
    Some(host) => host,
    None => "",
};

const fn valid_host(host: &str) -> bool {
    let bytes = host.as_bytes();
    if bytes.is_empty() {
        return true;
    }
    let prefix = b"https://";
    if bytes.len() <= prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if bytes[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    bytes[bytes.len() - 1] != b'/'
}

const _: () = assert!(
    valid_host(RELEASE_HOST),
    "MODLIGHT_DEFAULT_HOST must start with https:// and must not end with a slash"
);

/// Whether this build has somewhere to check (`HasReleaseChannel`). False hides every
/// trace of updating: no timer, no settings switch, no About line.
pub fn has_release_channel() -> bool {
    !RELEASE_HOST.is_empty()
}

/// The manifest endpoint.
pub const MANIFEST_PATH: &str = "/api/releases/app";
/// How often the timer asks (`GitHubUpdateManager.UpdateCheckInterval`).
pub const CHECK_INTERVAL: Duration = Duration::from_secs(3600);
/// Long enough for a cold server, short enough that a launch never waits on it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The real manifest is a few hundred bytes.
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_NOTES_LENGTH: usize = 1200;
const MAX_FILE_NAME_LENGTH: usize = 100;
/// The extensions a release for this system may have.
const RELEASE_EXTENSIONS: [&str; 2] = if cfg!(windows) {
    [".exe", ".zip"]
} else {
    [".dmg", ".zip"]
};
/// The name used when the manifest's is unusable.
const FALLBACK_FILE_NAME: &str = if cfg!(windows) {
    "Modlight-setup.exe"
} else {
    "Modlight.dmg"
};

/// The page a person is sent to when they would rather fetch it themselves.
pub fn download_page_url() -> String {
    format!("{RELEASE_HOST}/download")
}

/// One published release, as GET /api/releases/app describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseManifest {
    pub name: String,
    pub version_text: String,
    pub version: Vec<u64>,
    pub url: String,
    /// Safe to join onto a folder: built from the manifest, never taken verbatim.
    pub file_name: String,
    /// Bytes. Required: a download without a known size is a failure.
    pub size: i64,
    /// Lowercase hex. Never empty.
    pub sha256: String,
    pub notes: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestProblem {
    Empty,
    NotJson,
    MissingVersion,
    BadVersion,
    MissingUrl,
    BadUrl,
    MissingDigest,
    BadDigest,
    MissingSize,
    NotForThisPlatform,
}

/// Whether we are willing to fetch this URL: https anywhere, plain http only on the
/// loopback interface (`ModlightRelease.IsAcceptableUrl`).
pub fn is_acceptable_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    match url.scheme() {
        "https" => url.host_str().is_some_and(|h| !h.is_empty()),
        "http" => {
            matches!(
                url.host(),
                Some(url::Host::Ipv4(ip)) if ip.is_loopback()
            ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
                || url.host_str() == Some("localhost")
        }
        _ => false,
    }
}

/// A published version: a leading `v` dropped, anything from `-`, `+` or a space
/// dropped, the rest digits and dots only.
pub fn parse_version(text: &str) -> Option<Vec<u64>> {
    let mut value = text.trim();
    if value.starts_with(['v', 'V']) {
        value = &value[1..];
    }
    if let Some(cut) = value.find(['-', '+', ' ']) {
        value = &value[..cut];
    }
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let parts: Vec<&str> = value.split('.').collect();
    // `System.Version`: two to four numeric parts.
    if !(2..=4).contains(&parts.len()) || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut numbers: Vec<u64> = parts
        .iter()
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;
    numbers.resize(4, 0);
    Some(numbers)
}

/// A file name for the downloaded artefact: letters, digits, dot, dash, underscore; no
/// leading dot; at most 100 characters; and one of this system's release extensions
/// (`.exe` or `.zip` on Windows, `.dmg` or `.zip` on a Mac) - anything else is not a
/// release for this system (see the module notes).
pub fn safe_file_name(from_manifest: &str, url: &str) -> Option<String> {
    let mut candidate = from_manifest.trim().to_string();
    if candidate.is_empty()
        && let Ok(parsed) = url::Url::parse(url)
    {
        candidate = parsed
            .path_segments()
            .and_then(|mut s| s.next_back().map(str::to_string))
            .map(|s| percent_decode(&s))
            .unwrap_or_default();
    }
    let cleaned: String = candidate
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let mut name = cleaned.trim_start_matches('.').to_string();
    if name.is_empty() || name.len() > MAX_FILE_NAME_LENGTH {
        name = FALLBACK_FILE_NAME.to_string();
    }
    let lower = name.to_ascii_lowercase();
    RELEASE_EXTENSIONS
        .iter()
        .any(|extension| lower.ends_with(extension))
        .then_some(name)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_str(root: &Value, name: &str) -> String {
    root.get(name)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// `ModlightRelease.TryParse`, with the size required and the platform checked.
pub fn parse_manifest(json: &str) -> Result<ReleaseManifest, ManifestProblem> {
    if json.trim().is_empty() {
        return Err(ManifestProblem::Empty);
    }
    let root: Value = serde_json::from_str(json).map_err(|_| ManifestProblem::NotJson)?;
    if !root.is_object() {
        return Err(ManifestProblem::NotJson);
    }
    let version_text = read_str(&root, "version");
    if version_text.is_empty() {
        return Err(ManifestProblem::MissingVersion);
    }
    let version = parse_version(&version_text).ok_or(ManifestProblem::BadVersion)?;
    let url = read_str(&root, "url");
    if url.is_empty() {
        return Err(ManifestProblem::MissingUrl);
    }
    if !is_acceptable_url(&url) {
        return Err(ManifestProblem::BadUrl);
    }
    let digest = read_str(&root, "sha256");
    if digest.is_empty() {
        return Err(ManifestProblem::MissingDigest);
    }
    if !verify::is_digest(&digest) {
        return Err(ManifestProblem::BadDigest);
    }
    let size = root
        .get("size")
        .and_then(Value::as_i64)
        .filter(|s| *s > 0)
        .ok_or(ManifestProblem::MissingSize)?;
    let file_name = safe_file_name(&read_str(&root, "filename"), &url)
        .ok_or(ManifestProblem::NotForThisPlatform)?;
    let mut notes = read_str(&root, "notes");
    if notes.chars().count() > MAX_NOTES_LENGTH {
        notes = notes.chars().take(MAX_NOTES_LENGTH).collect::<String>();
        notes = format!("{}...", notes.trim_end());
    }
    Ok(ReleaseManifest {
        name: read_str(&root, "name"),
        version_text,
        version,
        url,
        file_name,
        size,
        sha256: digest.to_ascii_lowercase(),
        notes,
    })
}

/// What came back when we asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchStatus {
    Ok(ReleaseManifest),
    /// 404: nothing published. Not an error.
    NotPublished,
    Unreachable(String),
    Malformed(String),
}

/// What the app does about a completed check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// A newer release exists: offer it, never install it unasked.
    UpdateAvailable(ReleaseManifest),
    UpToDate,
    CheckFailed(String),
}

/// The URL of the manifest on `host`.
pub fn manifest_url(host: &str) -> String {
    format!("{}{MANIFEST_PATH}", host.trim_end_matches('/'))
}

/// Ask `host` for its current release (`ModlightReleaseClient.FetchAsync`). Never
/// fails loudly.
pub async fn fetch(host: &str) -> FetchStatus {
    let url = manifest_url(host);
    if !is_acceptable_url(&url) {
        return FetchStatus::Unreachable(format!("release host is not usable: {host}"));
    }
    let client = match reqwest::Client::builder()
        .user_agent(net::user_agent())
        .timeout(REQUEST_TIMEOUT)
        // The manifest's own host answers or nobody does.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => return FetchStatus::Unreachable(error.to_string()),
    };
    let response = match client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return FetchStatus::Unreachable(error.to_string()),
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return FetchStatus::NotPublished;
    }
    if !response.status().is_success() {
        return FetchStatus::Unreachable(format!("HTTP {}", response.status().as_u16()));
    }
    let mut response = response;
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_MANIFEST_BYTES {
                    return FetchStatus::Malformed("empty or oversized body".into());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => return FetchStatus::Unreachable(error.to_string()),
        }
    }
    let text = String::from_utf8_lossy(&body);
    match parse_manifest(text.trim_start_matches('\u{feff}')) {
        Ok(manifest) => FetchStatus::Ok(manifest),
        Err(problem) => FetchStatus::Malformed(format!("manifest rejected: {problem:?}")),
    }
}

/// Turn a completed check into the one of three things the app can do
/// (`ModlightReleaseClient.Evaluate`): numeric comparison; equal or older is not an
/// update; "nothing published" is up to date, not a failure.
pub fn evaluate(installed: Option<&[u64]>, fetch: &FetchStatus) -> UpdateOutcome {
    match fetch {
        FetchStatus::NotPublished => UpdateOutcome::UpToDate,
        FetchStatus::Unreachable(detail) | FetchStatus::Malformed(detail) => {
            if detail.contains("NotForThisPlatform") {
                UpdateOutcome::UpToDate
            } else {
                UpdateOutcome::CheckFailed(detail.clone())
            }
        }
        FetchStatus::Ok(manifest) => {
            let Some(installed) = installed else {
                return UpdateOutcome::CheckFailed("installed version unknown".into());
            };
            let mut mine = installed.to_vec();
            mine.resize(4, 0);
            match mine.cmp(&manifest.version) {
                Ordering::Less => UpdateOutcome::UpdateAvailable(manifest.clone()),
                _ => UpdateOutcome::UpToDate,
            }
        }
    }
}

/// This build's version (`Application.ProductVersion`).
pub fn installed_version() -> Option<Vec<u64>> {
    parse_version(env!("CARGO_PKG_VERSION"))
}

/// `ModlightUpdateChecker.CheckUpdateAsync` against the fixed host.
pub async fn check() -> UpdateOutcome {
    if !has_release_channel() {
        return UpdateOutcome::UpToDate;
    }
    check_host(RELEASE_HOST).await
}

/// The same against any host: the tests' stub on 127.0.0.1. The app only ever calls
/// [`check`].
pub async fn check_host(host: &str) -> UpdateOutcome {
    let fetch = fetch(host).await;
    let outcome = evaluate(installed_version().as_deref(), &fetch);
    log::info!("Modlight update check: {outcome:?}");
    outcome
}

/// Where the update is downloaded to (`Path.GetTempPath()/Modlight`).
pub fn download_folder() -> PathBuf {
    std::env::temp_dir().join(crate::APP_NAME)
}

/// The downloader's states (`DownloaderWindowStatus`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadResult {
    /// Downloaded, sized and hashed as the manifest promised.
    Verified(PathBuf),
    Cancelled,
    /// Short, long or refused body; the partial file is gone.
    Incomplete(String),
    /// The bytes do not match the digest; the file is gone.
    VerificationFailed,
}

/// Fetch the release file and verify it (`StartDownloadAsync` + `VerifyDownloadAsync`).
/// A file already there that hashes right is reused.
pub async fn download(
    manifest: &ReleaseManifest,
    folder: &Path,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> DownloadResult {
    let path = folder.join(&manifest.file_name);
    if path.is_file() {
        let check = path.clone();
        let digest = manifest.sha256.clone();
        if let Ok(verification) =
            tokio::task::spawn_blocking(move || verify::verify_or_delete(&check, &digest)).await
            && verification.matches
        {
            return DownloadResult::Verified(path);
        }
    }
    if !is_acceptable_url(&manifest.url) {
        return DownloadResult::Incomplete(manifest.url.clone());
    }
    let client = match reqwest::Client::builder()
        .user_agent(net::user_agent())
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if is_acceptable_url(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.error("a redirect left https")
            }
        }))
        .build()
    {
        Ok(client) => client,
        Err(error) => return DownloadResult::Incomplete(error.to_string()),
    };
    let part = folder.join(format!("{}.part", manifest.file_name));
    let result = download_with(&client, manifest, &part, progress, cancel).await;
    match result.outcome {
        DownloadOutcome::Completed => {}
        DownloadOutcome::Cancelled => {
            verify::try_delete(&part);
            return DownloadResult::Cancelled;
        }
        _ => {
            verify::try_delete(&part);
            return DownloadResult::Incomplete(result.message);
        }
    }
    let check = part.clone();
    let digest = manifest.sha256.clone();
    let verification = match tokio::task::spawn_blocking(move || {
        verify::verify_or_delete(&check, &digest)
    })
    .await
    {
        Ok(verification) => verification,
        Err(_) => return DownloadResult::VerificationFailed,
    };
    if !verification.matches {
        log::warn!(
            "Update: REJECTED download. Expected {}, got {}.",
            manifest.sha256,
            if verification.actual.is_empty() {
                "nothing"
            } else {
                &verification.actual
            }
        );
        return DownloadResult::VerificationFailed;
    }
    if std::fs::rename(&part, &path).is_err() {
        verify::try_delete(&part);
        return DownloadResult::Incomplete("the verified file could not be moved".into());
    }
    log::info!("Update: checksum verified ({}).", verification.actual);
    DownloadResult::Verified(path)
}

async fn download_with(
    client: &reqwest::Client,
    manifest: &ReleaseManifest,
    part: &Path,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> download::DownloadResult {
    // A previous attempt is never resumed: the folder is shared and only a whole,
    // hashed file may be trusted out of it.
    verify::try_delete(part);
    if let Some(parent) = part.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // `to_file` insists on https; the loopback http the tests use is let through here
    // only when it is the manifest's own scheme on 127.0.0.1.
    if manifest.url.starts_with("http://") {
        return download::to_file_unchecked(
            client,
            &manifest.url,
            part,
            manifest.size,
            download::STALL_TIMEOUT,
            progress,
            cancel,
        )
        .await;
    }
    download::to_file(
        client,
        &manifest.url,
        part,
        manifest.size,
        download::STALL_TIMEOUT,
        progress,
        cancel,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn manifest_json(file: &str, size: i64) -> String {
        format!(
            r#"{{"name":"Modlight App","version":"0.8.0","url":"https://files.example.com/r/{file}","filename":"{file}","size":{size},"sha256":"{DIGEST}","notes":"Faster uploads","publishedAt":"2026-10-01T10:00:00Z"}}"#
        )
    }

    #[test]
    fn updates_are_off_without_a_build_time_host() {
        // The test build is made without MODLIGHT_DEFAULT_HOST.
        if option_env!("MODLIGHT_DEFAULT_HOST").is_none() {
            assert!(!has_release_channel());
            assert_eq!(RELEASE_HOST, "");
            let outcome = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(check());
            assert_eq!(outcome, UpdateOutcome::UpToDate);
        }
        assert!(valid_host(""));
        assert!(valid_host("https://files.example.com"));
        assert!(!valid_host("https://files.example.com/"));
        assert!(!valid_host("http://files.example.com"));
        assert!(!valid_host("https://"));
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_manifest_parses_and_a_mac_one_is_not_for_us() {
        let manifest =
            parse_manifest(&manifest_json("Modlight-0.8.0-setup.exe", 80_000_000)).unwrap();
        assert_eq!(manifest.version, [0, 8, 0, 0]);
        assert_eq!(manifest.file_name, "Modlight-0.8.0-setup.exe");
        assert!(parse_manifest(&manifest_json("Modlight-0.8.0.zip", 10)).is_ok());
        assert_eq!(
            parse_manifest(&manifest_json("Modlight-0.8.0.dmg", 41_000_000)),
            Err(ManifestProblem::NotForThisPlatform)
        );
        assert_eq!(
            parse_manifest(&manifest_json("Modlight-setup.exe", 0)),
            Err(ManifestProblem::MissingSize)
        );
        assert_eq!(
            safe_file_name("", "https://h/r/"),
            Some("Modlight-setup.exe".into())
        );
        assert_eq!(safe_file_name("Modlight.msi", ""), None);
    }

    /// A release file this system takes.
    const OURS: &str = if cfg!(windows) {
        "Modlight-setup.exe"
    } else {
        "Modlight.dmg"
    };

    #[cfg(not(windows))]
    #[test]
    fn a_mac_manifest_parses_and_a_windows_one_is_not_for_us() {
        let manifest = parse_manifest(&manifest_json("Modlight-0.8.0.dmg", 41_000_000)).unwrap();
        assert_eq!(manifest.version, [0, 8, 0, 0]);
        assert_eq!(manifest.file_name, "Modlight-0.8.0.dmg");
        assert_eq!(manifest.size, 41_000_000);
        assert_eq!(
            parse_manifest(&manifest_json("Modlight-0.8.0-setup.exe", 80_000_000)),
            Err(ManifestProblem::NotForThisPlatform)
        );
        assert_eq!(
            parse_manifest(&manifest_json("Modlight.dmg", 0)),
            Err(ManifestProblem::MissingSize)
        );
        assert_eq!(
            evaluate(
                Some(&[0, 7, 7]),
                &FetchStatus::Malformed("manifest rejected: NotForThisPlatform".into())
            ),
            UpdateOutcome::UpToDate
        );
    }

    #[test]
    fn bad_manifests_are_refused_one_reason_at_a_time() {
        assert_eq!(parse_manifest(""), Err(ManifestProblem::Empty));
        assert_eq!(parse_manifest("<html>"), Err(ManifestProblem::NotJson));
        assert_eq!(parse_manifest("[]"), Err(ManifestProblem::NotJson));
        let good = manifest_json(OURS, 10);
        assert_eq!(
            parse_manifest(&good.replace("\"version\":\"0.8.0\"", "\"version\":\"\"")),
            Err(ManifestProblem::MissingVersion)
        );
        assert_eq!(
            parse_manifest(&good.replace("\"version\":\"0.8.0\"", "\"version\":\"eight\"")),
            Err(ManifestProblem::BadVersion)
        );
        assert_eq!(
            parse_manifest(&good.replace(
                "https://files.example.com/r/",
                "http://files.example.com/r/"
            )),
            Err(ManifestProblem::BadUrl)
        );
        assert_eq!(
            parse_manifest(&good.replace(DIGEST, "")),
            Err(ManifestProblem::MissingDigest)
        );
        assert_eq!(
            parse_manifest(&good.replace(DIGEST, "abc")),
            Err(ManifestProblem::BadDigest)
        );
    }

    #[test]
    fn versions_compare_as_numbers_and_never_downgrade() {
        let fetch = |v: &str| {
            FetchStatus::Ok(parse_manifest(&manifest_json(OURS, 10).replace("0.8.0", v)).unwrap())
        };
        assert!(matches!(
            evaluate(Some(&[0, 7, 9]), &fetch("0.7.10")),
            UpdateOutcome::UpdateAvailable(_)
        ));
        assert_eq!(
            evaluate(Some(&[0, 7, 10]), &fetch("0.7.10")),
            UpdateOutcome::UpToDate
        );
        assert_eq!(
            evaluate(Some(&[0, 8, 0]), &fetch("v0.7.10-rc1")),
            UpdateOutcome::UpToDate
        );
        assert_eq!(
            evaluate(Some(&[0, 7]), &FetchStatus::NotPublished),
            UpdateOutcome::UpToDate
        );
        assert!(matches!(
            evaluate(None, &fetch("9.9")),
            UpdateOutcome::CheckFailed(_)
        ));
        assert_eq!(parse_version("0.7"), Some(vec![0, 7, 0, 0]));
        assert_eq!(parse_version("1.2.3.4.5"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_file_name_cannot_walk_out_of_the_folder() {
        assert_eq!(
            safe_file_name("../../Library/LaunchAgents/x.dmg", ""),
            Some(
                "....LibraryLaunchAgentsx.dmg"
                    .trim_start_matches('.')
                    .to_string()
            )
        );
        assert_eq!(
            safe_file_name("", "https://h/r/Modlight%200.8.zip"),
            Some("Modlight0.8.zip".into())
        );
        assert_eq!(
            safe_file_name("", "https://h/r/"),
            Some("Modlight.dmg".into())
        );
        assert_eq!(safe_file_name("setup.exe", ""), None);
    }

    #[test]
    fn only_https_or_loopback_http_is_fetched() {
        assert!(is_acceptable_url("https://files.example.com/x.dmg"));
        assert!(is_acceptable_url("http://127.0.0.1:8420/x.dmg"));
        assert!(is_acceptable_url("http://localhost:8420/x.dmg"));
        assert!(!is_acceptable_url("http://files.example.com/x.dmg"));
        assert!(!is_acceptable_url("file:///x.dmg"));
    }

    #[cfg(windows)]
    #[test]
    fn the_file_name_cannot_walk_out_of_the_folder_on_windows() {
        assert_eq!(
            safe_file_name("..\\..\\AppData\\Roaming\\x.exe", ""),
            Some(
                "....AppDataRoamingx.exe"
                    .trim_start_matches('.')
                    .to_string()
            )
        );
        assert_eq!(
            safe_file_name("C:\\Windows\\setup.exe", ""),
            Some("CWindowssetup.exe".into())
        );
        assert_eq!(
            safe_file_name("", "https://h/r/Modlight%200.8.zip"),
            Some("Modlight0.8.zip".into())
        );
        assert_eq!(safe_file_name("x.dmg", ""), None);
        assert_eq!(safe_file_name("x.exe.lnk", ""), None);
    }
}
