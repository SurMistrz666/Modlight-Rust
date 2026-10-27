//! The app's own updates, from the Downloads of the server it is signed in to.
//!
//! **Check.** `GET {server}/api/downloads` with the account's token, which travels as every
//! other call carries it (the `x-api-key` header, never in a URL, redirects never
//! followed), on start, every 30 minutes and on "Check now" (`modlight-app`'s
//! `update_window`). Signed out, nothing is asked. The listing holds the builds the server
//! keeps per system, newest first ([`modlight_api::DownloadsListing`]); the newest valid
//! build for this system and processor is offered when it is another build made later than
//! this one ([`is_newer`]): another version or another commit, and a later build time. A
//! rebuild of the same commit is not offered again, and a build deleted on the server
//! never sends anyone back to an older one.
//!
//! **Download.** Only once the person said yes: `GET {server}{url}` with the same token,
//! into a folder of its own in the temporary folder, refused unless its size and SHA-256
//! are the listing's ([`download`]).
//!
//! **Install** ([`install`]): on a Mac only a `Modlight.app` signed by the same developer
//! as the running one (team identifier and designated requirement); on Windows the files
//! of `build-app.ps1`'s zip. A helper started beside the app waits for it to quit, puts the
//! new version in place and starts it. A copy that cannot replace itself (not an app
//! bundle, a read-only place, an unsigned or differently signed Mac build) sends the
//! person to the server's download page instead.
//!
//! Updates come only from the server the person signed in to, over https (plain http
//! only on the loopback interface, for the reference server and the tests). On a Mac the
//! signature check means the server can only hand out builds of the same developer; on
//! Windows the builds carry no publisher signature yet, so the server and TLS are what
//! vouch for them (`docs/PORT.md`, section 5).

pub mod codesign;
pub mod helper;
pub mod install;

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use modlight_api::{Api, ApiError, Credential, DownloadsListing, PublishedBuild};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio_util::sync::CancellationToken;

use crate::build_info::{self, BuildIdentity};
use crate::components::download::{self, DownloadOutcome, DownloadProgress};
use crate::components::{net, verify};

/// Whether this build can update itself. Every build can now, from the server it is signed
/// in to; kept for the screens that hid the update settings without a release host.
pub fn has_release_channel() -> bool {
    true
}

/// How often the timer asks: often enough that a fix reaches everybody the same
/// afternoon, rarely enough to be nothing for the server.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// The server's page where a person downloads builds themselves.
pub const DOWNLOAD_PAGE_PATH: &str = "/dashboard/downloads";
/// The largest build the updater fetches; a Modlight zip is about 50 MB.
pub const MAX_BUILD_SIZE: i64 = 2 * 1024 * 1024 * 1024;
const MAX_NOTES_LENGTH: usize = 1200;
const MAX_FILE_NAME_LENGTH: usize = 160;
const MAX_VERSION_LENGTH: usize = 64;

/// The server's download page, for a person to fetch a build by hand.
pub fn download_page_url(server: &str) -> String {
    format!("{}{DOWNLOAD_PAGE_PATH}", server.trim_end_matches('/'))
}

/// One build of the server's Downloads, checked and ready to fetch: an update for this
/// system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseManifest {
    pub name: String,
    /// The version as published, e.g. `0.7.8`.
    pub version_text: String,
    /// The version's numbers when it has the usual shape (`System.Version`), else empty.
    pub version: Vec<u64>,
    /// Absolute address of the zip, on `server`.
    pub url: String,
    /// The zip's name: letters, digits, `.`, `-`, `_`, ending in `.zip`.
    pub file_name: String,
    /// Bytes. Required: a download without a known size is a failure.
    pub size: i64,
    /// Lowercase hex. Never empty.
    pub sha256: String,
    /// The publisher's note, at most 1200 characters.
    pub notes: String,
    /// The server's id of the build.
    pub id: String,
    /// `macos` or `windows`, and `arm64` or `x64`.
    pub platform: String,
    pub arch: String,
    /// The commit (short hash), empty when built outside git.
    pub build: String,
    /// When it was built (always known: a build without a time is never offered).
    pub built: Option<DateTime<Utc>>,
    pub published: Option<DateTime<Utc>>,
    pub uploader: String,
    /// The origin of the server it comes from; it is fetched with that server's token only.
    pub server: String,
}

impl ReleaseManifest {
    /// `0.7.8 (9f8e7d6c)`, or `0.7.8` without a commit.
    pub fn label(&self) -> String {
        build_info::version_label(&self.version_text, &self.build)
    }
}

/// Why a build of the listing is not offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestProblem {
    Empty,
    NotJson,
    MissingVersion,
    BadVersion,
    BadBuild,
    MissingBuilt,
    MissingUrl,
    BadUrl,
    MissingDigest,
    BadDigest,
    MissingSize,
    TooLarge,
    BadFileName,
    NotForThisPlatform,
}

/// Whether we are willing to fetch from this address: https anywhere, plain http only on
/// the loopback interface (`ModlightRelease.IsAcceptableUrl`).
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
/// dropped, the rest digits and dots only, two to four numbers.
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

/// Whether a build's zip name can be joined onto a folder as it is: letters, digits,
/// `.`, `-` and `_`, no leading dot, at most 160 characters, ending in `.zip`. Nothing is
/// cleaned up: a name the build scripts would never write means the build is not offered.
pub fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_FILE_NAME_LENGTH
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && name.to_ascii_lowercase().ends_with(".zip")
}

/// Whether a root-relative address the server gave stays on the server once joined to it:
/// one leading `/` (never `//`, which would name another host), printable ASCII without
/// `\`, and the joined address's origin the server's.
fn same_server_url(server: &str, path: &str) -> Option<String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || !path.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
    {
        return None;
    }
    let joined = format!("{}{path}", server.trim_end_matches('/'));
    let parsed = url::Url::parse(&joined).ok()?;
    let origin = url::Url::parse(server).ok()?;
    (parsed.origin() == origin.origin()).then_some(joined)
}

fn plain_text(value: &str, max: usize, extra: &[u8]) -> bool {
    value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

/// One build of the listing as an update, or why it cannot be one. `server` is the origin
/// the listing came from.
pub fn release_from_build(
    build: &PublishedBuild,
    server: &str,
) -> Result<ReleaseManifest, ManifestProblem> {
    if !is_acceptable_url(server) {
        return Err(ManifestProblem::BadUrl);
    }
    if build.version.is_empty() {
        return Err(ManifestProblem::MissingVersion);
    }
    if !plain_text(&build.version, MAX_VERSION_LENGTH, b".-+_") {
        return Err(ManifestProblem::BadVersion);
    }
    if !plain_text(&build.build, MAX_VERSION_LENGTH, b".-_") {
        return Err(ManifestProblem::BadBuild);
    }
    if build.built.is_none() {
        return Err(ManifestProblem::MissingBuilt);
    }
    if build.url.is_empty() {
        return Err(ManifestProblem::MissingUrl);
    }
    let url = same_server_url(server, &build.url).ok_or(ManifestProblem::BadUrl)?;
    if build.sha256.is_empty() {
        return Err(ManifestProblem::MissingDigest);
    }
    if !verify::is_digest(&build.sha256) {
        return Err(ManifestProblem::BadDigest);
    }
    if build.size <= 0 {
        return Err(ManifestProblem::MissingSize);
    }
    if build.size > MAX_BUILD_SIZE {
        return Err(ManifestProblem::TooLarge);
    }
    if !is_safe_file_name(&build.filename) {
        return Err(ManifestProblem::BadFileName);
    }
    let mut notes = build.note.clone();
    if notes.chars().count() > MAX_NOTES_LENGTH {
        notes = notes.chars().take(MAX_NOTES_LENGTH).collect::<String>();
        notes = format!("{}...", notes.trim_end());
    }
    Ok(ReleaseManifest {
        name: crate::APP_NAME.to_string(),
        version_text: build.version.clone(),
        version: parse_version(&build.version).unwrap_or_default(),
        url,
        file_name: build.filename.clone(),
        size: build.size,
        sha256: build.sha256.to_ascii_lowercase(),
        notes,
        id: build.id.clone(),
        platform: build.platform.to_ascii_lowercase(),
        arch: build.arch.to_ascii_lowercase(),
        build: build.build.clone(),
        built: build.built,
        published: build.published,
        uploader: build.uploader.clone(),
        server: server.trim_end_matches('/').to_string(),
    })
}

/// Another build made later than `mine`: another version or another commit, and a later
/// build time. The time keeps an older build (the newest deleted on the server) from being
/// offered; the version and commit keep a rebuild of this very build from being offered.
pub fn is_newer(release: &ReleaseManifest, mine: &BuildIdentity) -> bool {
    let other = release.version_text.trim() != mine.version.trim()
        || !release.build.trim().eq_ignore_ascii_case(mine.build.trim());
    let later = match (release.built, mine.built) {
        (Some(theirs), Some(ours)) => theirs > ours,
        (Some(_), None) => true,
        (None, _) => false,
    };
    other && later
}

/// What happened when we asked.
// One a check, every half hour at most: the size of the found build is no cost, and the
// variant keeps the type the screens match on (`UpdateAvailable(ReleaseManifest)`).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// A newer build exists: offer it, never install it unasked.
    UpdateAvailable(ReleaseManifest),
    UpToDate,
    CheckFailed(String),
}

/// What the listing offers `mine`: the newest valid build of `platform` and `arch` (by its
/// build time), when it is newer ([`is_newer`]). Builds that fail [`release_from_build`]
/// are passed over (and logged), so one broken entry never hides the others.
pub fn evaluate_listing(
    listing: &DownloadsListing,
    server: &str,
    mine: &BuildIdentity,
    platform: &str,
    arch: &str,
) -> UpdateOutcome {
    let mut best: Option<ReleaseManifest> = None;
    for build in listing.builds_for(platform) {
        if (!build.platform.is_empty() && !build.platform.eq_ignore_ascii_case(platform))
            || !build.arch.eq_ignore_ascii_case(arch)
        {
            continue;
        }
        match release_from_build(build, server) {
            Ok(mut release) => {
                release.platform = platform.to_ascii_lowercase();
                if best.as_ref().is_none_or(|b| release.built > b.built) {
                    best = Some(release);
                }
            }
            Err(problem) => log::warn!("Update: build {:?} passed over: {problem:?}", build.id),
        }
    }
    match best {
        Some(release) if is_newer(&release, mine) => UpdateOutcome::UpdateAvailable(release),
        _ => UpdateOutcome::UpToDate,
    }
}

/// Ask the server this app is signed in to (`credential`) whether it has a newer build of
/// this app. Never fails loudly.
pub async fn check(api: &Api, credential: &Credential) -> UpdateOutcome {
    check_as(
        api,
        credential,
        &BuildIdentity::current(),
        build_info::PLATFORM,
        build_info::ARCH,
    )
    .await
}

/// [`check`] for a given build, system and processor (the tests' other builds).
pub async fn check_as(
    api: &Api,
    credential: &Credential,
    mine: &BuildIdentity,
    platform: &str,
    arch: &str,
) -> UpdateOutcome {
    if !is_acceptable_url(&credential.host) {
        return UpdateOutcome::CheckFailed(
            "updates are taken only from a server reached over https".into(),
        );
    }
    let outcome = match api.get_downloads(credential).await {
        Ok(listing) => evaluate_listing(&listing, &credential.host, mine, platform, arch),
        // A server from before Downloads: nothing to offer, nothing wrong.
        Err(ApiError::Status(404)) => UpdateOutcome::UpToDate,
        Err(error) => UpdateOutcome::CheckFailed(error.to_string()),
    };
    log::info!("Modlight update check: {outcome:?}");
    outcome
}

/// This build's version (`Application.ProductVersion`).
pub fn installed_version() -> Option<Vec<u64>> {
    parse_version(env!("CARGO_PKG_VERSION"))
}

/// Where updates are downloaded and unpacked: a folder of their own in the temporary
/// folder (`Path.GetTempPath()/Modlight`, whose `Downloads` belongs to the Download tab).
pub fn download_folder() -> PathBuf {
    std::env::temp_dir().join(crate::APP_NAME).join("Update")
}

/// The downloader's states (`DownloaderWindowStatus`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadResult {
    /// Downloaded, sized and hashed as the listing promised.
    Verified(PathBuf),
    Cancelled,
    /// Short, long or refused body; the partial file is gone.
    Incomplete(String),
    /// The bytes do not match the digest; the file is gone.
    VerificationFailed,
    /// The app is no longer signed in to the server the build is on; nothing was asked.
    OtherServer,
}

/// A client that carries the account's token in `x-api-key` to whatever it asks, follows no
/// redirect and has no overall deadline (the stall watch tells slow from dead). Used only
/// for the build's address, which is on the token's own server.
fn token_client(credential: &Credential) -> Result<reqwest::Client, String> {
    let mut value = HeaderValue::from_str(&credential.token)
        .map_err(|_| "the token cannot be sent in a header".to_string())?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(HeaderName::from_static(modlight_api::API_KEY_HEADER), value);
    reqwest::Client::builder()
        .user_agent(net::user_agent())
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// Fetch the build into `folder` and verify it (`StartDownloadAsync` +
/// `VerifyDownloadAsync`). A file already there that hashes right is reused.
pub async fn download(
    manifest: &ReleaseManifest,
    credential: &Credential,
    folder: &Path,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> DownloadResult {
    // The token goes to its own server only.
    if !modlight_api::same_origin(&manifest.server, &credential.host)
        || !modlight_api::same_origin(&manifest.url, &credential.host)
    {
        return DownloadResult::OtherServer;
    }
    if !is_safe_file_name(&manifest.file_name) || !is_acceptable_url(&manifest.url) {
        return DownloadResult::Incomplete(format!("not a build address: {}", manifest.url));
    }
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
    let client = match token_client(credential) {
        Ok(client) => client,
        Err(error) => return DownloadResult::Incomplete(error),
    };
    let part = folder.join(format!("{}.part", manifest.file_name));
    // A previous attempt is never resumed: only a whole, hashed file may be trusted.
    verify::try_delete(&part);
    if let Err(error) = std::fs::create_dir_all(folder) {
        return DownloadResult::Incomplete(error.to_string());
    }
    // `to_file` insists on https; the loopback http `is_acceptable_url` lets through (the
    // reference server, the tests) is judged above.
    let result = download::to_file_unchecked(
        &client,
        &manifest.url,
        &part,
        manifest.size,
        download::STALL_TIMEOUT,
        progress,
        cancel,
    )
    .await;
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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    const DIGEST: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const SERVER: &str = "https://files.example.com";

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }

    fn build(
        id: &str,
        arch: &str,
        version: &str,
        commit: &str,
        built: DateTime<Utc>,
    ) -> PublishedBuild {
        let filename = format!("Modlight-{version}-{commit}-macos-{arch}.zip");
        PublishedBuild {
            id: id.into(),
            platform: "macos".into(),
            arch: arch.into(),
            version: version.into(),
            build: commit.into(),
            built: Some(built),
            published: Some(built),
            size: 41_200_000,
            sha256: DIGEST.to_uppercase(),
            url: format!("/downloads/{id}/{filename}"),
            filename,
            uploader: "Alex".into(),
            note: "Faster uploads".into(),
        }
    }

    fn mine(version: &str, commit: &str, built: DateTime<Utc>) -> BuildIdentity {
        BuildIdentity {
            version: version.into(),
            build: commit.into(),
            built: Some(built),
        }
    }

    fn listing(builds: Vec<PublishedBuild>) -> DownloadsListing {
        DownloadsListing {
            platforms: vec![
                modlight_api::DownloadsPlatform {
                    platform: "macos".into(),
                    builds,
                },
                modlight_api::DownloadsPlatform {
                    platform: "windows".into(),
                    builds: vec![PublishedBuild {
                        platform: "windows".into(),
                        arch: "x64".into(),
                        ..build("w1", "x64", "9.0.0", "ffffffff", at(30, 0))
                    }],
                },
            ],
            ..DownloadsListing::default()
        }
    }

    #[test]
    fn every_build_can_update_from_its_server() {
        assert!(has_release_channel());
        assert_eq!(
            download_page_url("https://files.example.com/"),
            "https://files.example.com/dashboard/downloads"
        );
        assert_eq!(CHECK_INTERVAL, Duration::from_secs(1800));
    }

    #[test]
    fn a_build_becomes_an_update_with_its_address_on_the_server() {
        let release = release_from_build(
            &build("m2", "arm64", "0.7.9", "9f8e7d6c", at(27, 14)),
            SERVER,
        )
        .unwrap();
        assert_eq!(
            release.url,
            "https://files.example.com/downloads/m2/Modlight-0.7.9-9f8e7d6c-macos-arm64.zip"
        );
        assert_eq!(release.sha256, DIGEST, "lowercase");
        assert_eq!(release.version, [0, 7, 9, 0]);
        assert_eq!(release.label(), "0.7.9 (9f8e7d6c)");
        assert_eq!(release.server, SERVER);
        assert_eq!(release.notes, "Faster uploads");
    }

    #[test]
    fn a_build_that_could_lead_anywhere_else_is_not_offered() {
        let good = build("m2", "arm64", "0.7.9", "9f8e7d6c", at(27, 14));
        let with = |change: &dyn Fn(&mut PublishedBuild)| {
            let mut b = good.clone();
            change(&mut b);
            release_from_build(&b, SERVER)
        };
        // Another host, however it is spelled.
        for url in [
            "//evil.example/x.zip",
            "@evil.example/x.zip",
            "https://evil.example/x.zip",
            "downloads/x.zip",
            "/downloads/a b.zip",
            "/downloads\\..\\x.zip",
            "/downloads/\u{0}x.zip",
        ] {
            assert_eq!(
                with(&|b| b.url = url.into()),
                Err(ManifestProblem::BadUrl),
                "{url}"
            );
        }
        assert_eq!(with(&|b| b.url.clear()), Err(ManifestProblem::MissingUrl));
        // A file name that is not a plain zip name.
        for name in [
            "../Modlight.zip",
            ".Modlight.zip",
            "Modlight.dmg",
            "Modlight setup.zip",
            "Modlight.zip.exe",
            "",
        ] {
            assert_eq!(
                with(&|b| b.filename = name.into()),
                Err(ManifestProblem::BadFileName),
                "{name}"
            );
        }
        assert_eq!(
            with(&|b| b.sha256.clear()),
            Err(ManifestProblem::MissingDigest)
        );
        assert_eq!(
            with(&|b| b.sha256 = "abc".into()),
            Err(ManifestProblem::BadDigest)
        );
        assert_eq!(with(&|b| b.size = 0), Err(ManifestProblem::MissingSize));
        assert_eq!(
            with(&|b| b.size = MAX_BUILD_SIZE + 1),
            Err(ManifestProblem::TooLarge)
        );
        assert_eq!(
            with(&|b| b.built = None),
            Err(ManifestProblem::MissingBuilt)
        );
        assert_eq!(
            with(&|b| b.version.clear()),
            Err(ManifestProblem::MissingVersion)
        );
        assert_eq!(
            with(&|b| b.version = "0.7.9\nX".into()),
            Err(ManifestProblem::BadVersion)
        );
        assert_eq!(
            with(&|b| b.build = "abc/def".into()),
            Err(ManifestProblem::BadBuild)
        );
        // Only https, or http on the loopback interface.
        assert_eq!(
            release_from_build(&good, "http://files.example.com"),
            Err(ManifestProblem::BadUrl)
        );
        assert!(release_from_build(&good, "http://127.0.0.1:8420").is_ok());
    }

    #[test]
    fn newer_means_another_build_made_later() {
        let release = |version: &str, commit: &str, built| {
            release_from_build(&build("x", "arm64", version, commit, built), SERVER).unwrap()
        };
        let me = mine("0.7.8", "1a2b3c4d", at(20, 9));
        // Another version, built later.
        assert!(is_newer(&release("0.7.9", "9f8e7d6c", at(27, 14)), &me));
        // The same version from another commit, built later: a fix without a version bump.
        assert!(is_newer(&release("0.7.8", "9f8e7d6c", at(27, 14)), &me));
        // This very build, published (or rebuilt from the same commit later).
        assert!(!is_newer(&release("0.7.8", "1a2b3c4d", at(20, 9)), &me));
        assert!(!is_newer(&release("0.7.8", "1A2B3C4D", at(27, 14)), &me));
        // Built before this one: the newest was deleted on the server; no way back.
        assert!(!is_newer(&release("0.7.7", "0c0c0c0c", at(10, 9)), &me));
        assert!(!is_newer(&release("0.7.9", "9f8e7d6c", at(20, 9)), &me));
        // Built outside git on both sides: the version and the time decide.
        let plain = mine("0.7.8", "", at(20, 9));
        assert!(is_newer(&release("0.7.9", "", at(21, 9)), &plain));
        assert!(!is_newer(&release("0.7.8", "", at(21, 9)), &plain));
    }

    #[test]
    fn the_newest_valid_build_for_this_system_and_processor_is_offered() {
        let me = mine("0.7.8", "1a2b3c4d", at(20, 9));
        let builds = vec![
            // Newest first, as the server lists them; the newest is an Intel build.
            build("m4", "x64", "0.8.1", "44444444", at(29, 9)),
            build("m3", "arm64", "0.8.0", "33333333", at(28, 9)),
            build("m2", "arm64", "0.7.9", "22222222", at(27, 9)),
            build("m1", "arm64", "0.7.8", "1a2b3c4d", at(20, 9)),
        ];
        let UpdateOutcome::UpdateAvailable(release) =
            evaluate_listing(&listing(builds.clone()), SERVER, &me, "macos", "arm64")
        else {
            panic!("nothing offered");
        };
        assert_eq!(release.id, "m3");
        let UpdateOutcome::UpdateAvailable(release) =
            evaluate_listing(&listing(builds.clone()), SERVER, &me, "macos", "x64")
        else {
            panic!("nothing offered");
        };
        assert_eq!(release.id, "m4");
        // A broken newest entry does not hide the one before it.
        let mut broken = builds.clone();
        broken[1].sha256 = "nope".into();
        let UpdateOutcome::UpdateAvailable(release) =
            evaluate_listing(&listing(broken), SERVER, &me, "macos", "arm64")
        else {
            panic!("nothing offered");
        };
        assert_eq!(release.id, "m2");
        // An entry that says it is for another system, inside this system's list.
        let mut stray = builds.clone();
        stray[1].platform = "windows".into();
        stray[2].platform = String::new();
        let UpdateOutcome::UpdateAvailable(release) =
            evaluate_listing(&listing(stray), SERVER, &me, "macos", "arm64")
        else {
            panic!("nothing offered");
        };
        assert_eq!(release.id, "m2", "an empty platform is the list's");
        // Only this build there, or nothing at all, or only other systems' builds.
        assert_eq!(
            evaluate_listing(
                &listing(vec![builds[3].clone()]),
                SERVER,
                &me,
                "macos",
                "arm64"
            ),
            UpdateOutcome::UpToDate
        );
        assert_eq!(
            evaluate_listing(&DownloadsListing::default(), SERVER, &me, "macos", "arm64"),
            UpdateOutcome::UpToDate
        );
        let UpdateOutcome::UpdateAvailable(release) =
            evaluate_listing(&listing(Vec::new()), SERVER, &me, "windows", "x64")
        else {
            panic!("the Windows build is not offered to Windows");
        };
        assert_eq!(release.id, "w1");
        assert_eq!(release.platform, "windows");
        assert_eq!(
            evaluate_listing(&listing(Vec::new()), SERVER, &me, "windows", "arm64"),
            UpdateOutcome::UpToDate
        );
    }

    #[test]
    fn versions_parse_as_numbers() {
        assert_eq!(parse_version("0.7"), Some(vec![0, 7, 0, 0]));
        assert_eq!(parse_version("v0.7.10-rc1"), Some(vec![0, 7, 10, 0]));
        assert_eq!(parse_version("1.2.3.4.5"), None);
        assert_eq!(parse_version("eight"), None);
    }

    #[test]
    fn only_https_or_loopback_http_is_fetched() {
        assert!(is_acceptable_url("https://files.example.com/x.zip"));
        assert!(is_acceptable_url("http://127.0.0.1:8420/x.zip"));
        assert!(is_acceptable_url("http://localhost:8420/x.zip"));
        assert!(is_acceptable_url("http://[::1]:8420/x.zip"));
        assert!(!is_acceptable_url("http://files.example.com/x.zip"));
        assert!(!is_acceptable_url("file:///x.zip"));
    }

    #[test]
    fn updates_have_a_folder_of_their_own() {
        let folder = download_folder();
        assert!(folder.ends_with(std::path::Path::new("Modlight").join("Update")));
        assert!(!crate::download::scratch_folder().starts_with(&folder));
        assert!(!folder.starts_with(crate::download::scratch_folder()));
    }

    #[tokio::test]
    async fn the_build_is_fetched_only_from_the_signed_in_server() {
        let release = release_from_build(
            &build("m2", "arm64", "0.7.9", "9f8e7d6c", at(27, 14)),
            SERVER,
        )
        .unwrap();
        let elsewhere = Credential::new("https://other.example.com", "token-0123456789");
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(
            download(
                &release,
                &elsewhere,
                folder.path(),
                &|_| {},
                &CancellationToken::new()
            )
            .await,
            DownloadResult::OtherServer
        );
        assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());
    }
}
