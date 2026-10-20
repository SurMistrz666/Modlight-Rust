//! Reading a GitHub release and judging what it publishes (`ModlightToolRelease`,
//! `ModlightToolClient`, `ModlightToolBuild`).
//!
//! The digest is the whole point. An automatic update cannot have a pin a human looked
//! at - the version does not exist yet when the human looks - so it has two publications
//! instead: GitHub's own asset digest, computed server-side and served from
//! api.github.com, and the project's SHA2-256SUMS file, produced by its release workflow.
//!
//! One rule is stricter than upstream's, as the audit asked (PORT.md section 5): BOTH must
//! be present and identical. Upstream accepted either one alone "with the weakening
//! written onto the build"; here a release missing either publication is refused like a
//! disagreement is, and so is a checksum file that could not be fetched.

use std::time::Duration;

use serde_json::Value;

use super::net;
use super::verify::{digests_match, is_digest};

/// Where GitHub says what the newest release is.
const API_HOST: &str = "https://api.github.com";

/// A release document with two dozen assets and its notes. Anything past this is not one.
const MAX_RELEASE_BYTES: usize = 2 * 1024 * 1024;

/// The real checksum manifest is under two kilobytes.
const MAX_CHECKSUM_BYTES: usize = 256 * 1024;

/// Long enough for a cold answer, short enough that nothing ever waits on it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// One published build of an external tool, ready to be fetched. Only ever produced by
/// [`with_published_digest`], so `sha256` is never empty and `url` is never a host or a
/// scheme we would not fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolBuild {
    /// The publisher's version string, verbatim.
    pub version_text: String,
    /// https, on github.com.
    pub url: String,
    /// Bytes, from the release document.
    pub size: i64,
    /// Lowercase hex.
    pub sha256: String,
    /// Which publications the digest came from, for the log.
    pub digest_source: String,
}

/// Why a release was rejected. Only ever written to the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolProblem {
    None,
    Empty,
    NotJson,
    Unpublished,
    MissingVersion,
    BadVersion,
    MissingAsset,
    BadUrl,
    MissingSize,
    MissingDigest,
    DigestConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchStatus {
    /// A build we are willing to install.
    Ok,
    /// Nobody answered, or the request never completed.
    Unreachable,
    /// The publisher answered, but not with something this build can act on.
    Malformed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchResult {
    pub status: FetchStatus,
    /// `Some` exactly when `status` is `Ok`.
    pub build: Option<ToolBuild>,
    /// One line for the log. Never shown to a person.
    pub detail: String,
}

impl FetchResult {
    pub fn ok(build: ToolBuild) -> FetchResult {
        FetchResult {
            status: FetchStatus::Ok,
            build: Some(build),
            detail: String::new(),
        }
    }

    pub fn unreachable(detail: impl Into<String>) -> FetchResult {
        FetchResult {
            status: FetchStatus::Unreachable,
            build: None,
            detail: detail.into(),
        }
    }

    pub fn malformed(detail: impl Into<String>) -> FetchResult {
        FetchResult {
            status: FetchStatus::Malformed,
            build: None,
            detail: detail.into(),
        }
    }
}

/// Where the newest release of a repository is described. `api_origin` is for tests: it
/// does NOT loosen where an artefact may come from, which is checked separately.
pub fn latest_release_url(repository: &str, api_origin: Option<&str>) -> String {
    format!(
        "{}/repos/{}/releases/latest",
        api_origin.unwrap_or(API_HOST).trim_end_matches('/'),
        repository.trim_matches('/')
    )
}

/// A date-shaped tool version, as numbers: three or four numeric segments behind a
/// four-digit year (yt-dlp publishes YYYY.MM.DD, and YYYY.MM.DD.HHMMSS nightly). Anything
/// else is not a version this code knows how to order.
pub fn parse_version(text: &str) -> Option<Vec<u32>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let pieces: Vec<&str> = text.split('.').collect();
    if !(3..=4).contains(&pieces.len()) {
        return None;
    }
    let mut numbers = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        if piece.is_empty() || piece.len() > 9 || !piece.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        numbers.push(piece.parse().ok()?);
    }
    if pieces[0].len() != 4 || !(2000..=2999).contains(&numbers[0]) {
        return None;
    }
    Some(numbers)
}

/// Compare two parsed versions, a missing segment counting as zero.
pub fn compare_versions(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    let length = a.len().max(b.len());
    for i in 0..length {
        let left = a.get(i).copied().unwrap_or(0);
        let right = b.get(i).copied().unwrap_or(0);
        if left != right {
            return left.cmp(&right);
        }
    }
    std::cmp::Ordering::Equal
}

/// Whether `candidate` is worth moving to from `installed`: false when either side does
/// not parse; strictly newer, never merely different.
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    match (parse_version(candidate), parse_version(installed)) {
        (Some(left), Some(right)) => compare_versions(&left, &right).is_gt(),
        _ => false,
    }
}

fn read_str<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

/// The asset digest field, which GitHub publishes as `sha256:<hex>`.
fn read_digest(asset: &Value) -> String {
    let value = read_str(asset, "digest");
    let Some(digest) = value
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("sha256:"))
        .map(|_| value[7..].trim())
    else {
        return String::new();
    };
    if is_digest(digest) {
        digest.to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// A release document parsed for one asset: the build carrying the API's digest (which
/// may still be empty here) and the address of the checksum manifest (or empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedRelease {
    pub version_text: String,
    pub url: String,
    pub size: i64,
    pub api_digest: String,
    pub checksums_url: String,
}

/// `TryParseRelease`.
pub fn parse_release(
    json: &str,
    asset_name: &str,
    checksum_asset_name: &str,
) -> Result<ParsedRelease, ToolProblem> {
    if json.trim().is_empty() {
        return Err(ToolProblem::Empty);
    }
    let root: Value = serde_json::from_str(json).map_err(|_| ToolProblem::NotJson)?;
    if !root.is_object() {
        return Err(ToolProblem::NotJson);
    }
    // A draft is a release its own publisher has not stood behind yet.
    if root.get("draft").and_then(Value::as_bool) == Some(true)
        || root.get("prerelease").and_then(Value::as_bool) == Some(true)
    {
        return Err(ToolProblem::Unpublished);
    }
    let version_text = read_str(&root, "tag_name").trim().to_string();
    if version_text.is_empty() {
        return Err(ToolProblem::MissingVersion);
    }
    if parse_version(&version_text).is_none() {
        return Err(ToolProblem::BadVersion);
    }
    let Some(assets) = root.get("assets").and_then(Value::as_array) else {
        return Err(ToolProblem::MissingAsset);
    };
    let mut found: Option<&Value> = None;
    let mut checksums_url = String::new();
    for asset in assets {
        let name = read_str(asset, "name");
        if name.is_empty() {
            continue;
        }
        if found.is_none() && name == asset_name {
            found = Some(asset);
            continue;
        }
        if checksums_url.is_empty()
            && !checksum_asset_name.is_empty()
            && name == checksum_asset_name
        {
            let candidate = read_str(asset, "browser_download_url");
            if net::is_artefact_url(candidate) {
                checksums_url = candidate.to_string();
            }
        }
    }
    let Some(asset) = found else {
        return Err(ToolProblem::MissingAsset);
    };
    let url = read_str(asset, "browser_download_url").to_string();
    if !net::is_artefact_url(&url) {
        return Err(ToolProblem::BadUrl);
    }
    let size = asset
        .get("size")
        .and_then(Value::as_i64)
        .filter(|s| *s > 0)
        .unwrap_or(0);
    if size <= 0 {
        // A download without a known size is a failure (the audit's rule).
        return Err(ToolProblem::MissingSize);
    }
    Ok(ParsedRelease {
        version_text,
        url,
        size,
        api_digest: read_digest(asset),
        checksums_url,
    })
}

/// The digest for one file out of a sha256sum-style manifest: the name matched exactly,
/// the binary-mode `*` stripped.
pub fn digest_from_checksums(text: &str, file_name: &str) -> String {
    if text.is_empty() || file_name.is_empty() {
        return String::new();
    }
    for raw in text.split('\n') {
        let line = raw.trim();
        let Some(space) = line.find(' ') else {
            continue;
        };
        if space == 0 {
            continue;
        }
        let digest = &line[..space];
        if !is_digest(digest) {
            continue;
        }
        let name = line[space + 1..].trim_start_matches([' ', '*']);
        if name == file_name {
            return digest.to_ascii_lowercase();
        }
    }
    String::new()
}

/// `WithPublishedDigest`, strict: the API's digest and the project's checksum file must
/// both be there and agree, or nothing is installed.
pub fn with_published_digest(
    release: &ParsedRelease,
    checksums: &str,
    asset_name: &str,
) -> Result<ToolBuild, ToolProblem> {
    let published = digest_from_checksums(checksums, asset_name);
    let api = &release.api_digest;
    if api.is_empty() || published.is_empty() {
        return Err(ToolProblem::MissingDigest);
    }
    if !digests_match(api, &published) {
        return Err(ToolProblem::DigestConflict);
    }
    Ok(ToolBuild {
        version_text: release.version_text.clone(),
        url: release.url.clone(),
        size: release.size,
        sha256: published,
        digest_source: "release API and published checksums".into(),
    })
}

/// Fetch a body, refusing to hold more of it than the thing could ever be. Empty on any
/// failure.
async fn get_text(client: &reqwest::Client, url: &str, limit: usize) -> Result<String, String> {
    if !net::is_fetchable(url) {
        return Err(format!("refused {url}"));
    }
    let mut response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > limit {
            return Err("too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

/// What the newest published build is, or why we do not know (`ModlightToolClient.FetchAsync`).
/// Never fails loudly: everything comes back as a status and a line for the log.
pub async fn fetch(
    repository: &str,
    asset_name: &str,
    checksum_asset_name: &str,
    api_origin: Option<&str>,
) -> FetchResult {
    let client = match net::api_client(REQUEST_TIMEOUT) {
        Ok(client) => client,
        Err(error) => return FetchResult::unreachable(error),
    };
    let url = latest_release_url(repository, api_origin);
    let body = match get_text(&client, &url, MAX_RELEASE_BYTES).await {
        Ok(body) if !body.is_empty() => body,
        Ok(_) => return FetchResult::unreachable(format!("no release document from {url}")),
        Err(error) => return FetchResult::unreachable(format!("{url}: {error}")),
    };
    let release = match parse_release(&body, asset_name, checksum_asset_name) {
        Ok(release) => release,
        Err(problem) => return FetchResult::malformed(format!("release rejected: {problem:?}")),
    };
    // Strict: a checksum file that cannot be fetched costs the update, not just the
    // second publication.
    let checksums = if release.checksums_url.is_empty() {
        String::new()
    } else {
        get_text(&client, &release.checksums_url, MAX_CHECKSUM_BYTES)
            .await
            .unwrap_or_default()
    };
    match with_published_digest(&release, &checksums, asset_name) {
        Ok(build) => FetchResult::ok(build),
        Err(problem) => FetchResult::malformed(format!("digest rejected: {problem:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = include_str!("../../tests/fixtures/yt-dlp-2026.08.19-release.json");
    const SUMS: &str = include_str!("../../tests/fixtures/yt-dlp-2026.08.19-SHA2-256SUMS");
    const MACOS: &str = "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202";
    const EXE: &str = "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a";

    fn parsed(digest: &str) -> ParsedRelease {
        ParsedRelease {
            version_text: "2026.08.19".into(),
            url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos"
                .into(),
            size: 37_146_048,
            api_digest: digest.into(),
            checksums_url: String::new(),
        }
    }

    #[test]
    fn the_real_release_parses_for_the_mac_asset() {
        let release = parse_release(RELEASE, "yt-dlp_macos", "SHA2-256SUMS").unwrap();
        assert_eq!(release.version_text, "2026.08.19");
        assert_eq!(
            release.url,
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos"
        );
        assert_eq!(release.size, 37_146_048);
        assert_eq!(release.api_digest, MACOS);
        assert_eq!(
            release.checksums_url,
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/SHA2-256SUMS"
        );
        let build = with_published_digest(&release, SUMS, "yt-dlp_macos").unwrap();
        assert_eq!(build.sha256, MACOS);
        assert_eq!(build.digest_source, "release API and published checksums");
        // The neighbouring line is a different file.
        let zip = parse_release(RELEASE, "yt-dlp_macos.zip", "SHA2-256SUMS").unwrap();
        let zip = with_published_digest(&zip, SUMS, "yt-dlp_macos.zip").unwrap();
        assert_ne!(zip.sha256, MACOS);
    }

    #[test]
    fn dated_versions_compare_as_numbers() {
        assert!(is_newer("2026.08.19", "2026.07.04"));
        assert!(is_newer("2026.08.19", "2026.08.9"));
        assert!(is_newer("2026.08.19.232658", "2026.08.19"));
        assert!(!is_newer("2026.08.19", "2026.08.19.232658"));
        assert!(!is_newer("2026.08.19", "2026.08.19"));
        assert!(!is_newer("2026.8.19", "2026.08.19"));
        assert!(!is_newer("2026.07.04", "2026.08.19"));
    }

    #[test]
    fn what_is_not_a_version_is_refused() {
        for bad in [
            "",
            "   ",
            "nightly",
            "v2026.08.19",
            "2026.08",
            "2026.08.19-rc1",
            "1.2.3",
            "202.08.19",
            "3026.08.19",
            "2026.08.19.232658.1.2",
            "2026.08.nineteen",
            "2026.08.19 ; rm -rf",
        ] {
            assert!(parse_version(bad).is_none(), "accepted {bad:?}");
        }
        assert!(parse_version("2026.08.19").is_some());
        assert!(parse_version("2026.08.19.232658").is_some());
    }

    #[test]
    fn an_unreadable_version_is_never_overruled() {
        assert!(!is_newer("2026.08.19", ""));
        assert!(!is_newer("2026.08.19", "not a version"));
        assert!(!is_newer("", "2026.08.19"));
    }

    #[test]
    fn checksum_lines_are_matched_by_exact_name() {
        assert_eq!(digest_from_checksums(SUMS, "yt-dlp_macos"), MACOS);
        assert_eq!(digest_from_checksums(SUMS, "yt-dlp.exe"), EXE);
        assert_eq!(digest_from_checksums(SUMS, "yt-dlp_win.zipx"), "");
        assert_eq!(digest_from_checksums(SUMS, "yt-dlp_maco"), "");
        assert_eq!(
            digest_from_checksums(&format!("{MACOS} *yt-dlp_macos\n"), "yt-dlp_macos"),
            MACOS
        );
        assert_eq!(
            digest_from_checksums("not-a-digest  yt-dlp_macos\n", "yt-dlp_macos"),
            ""
        );
        assert_eq!(
            digest_from_checksums(&format!("{MACOS}  yt-dlp_macos\r\n"), "yt-dlp_macos"),
            MACOS
        );
    }

    #[test]
    fn both_publications_must_exist_and_agree() {
        let both = with_published_digest(&parsed(MACOS), SUMS, "yt-dlp_macos");
        assert!(both.is_ok());
        // The API alone: refused (upstream would have accepted it).
        assert_eq!(
            with_published_digest(&parsed(MACOS), "", "yt-dlp_macos"),
            Err(ToolProblem::MissingDigest)
        );
        // The checksum file alone: refused too.
        assert_eq!(
            with_published_digest(&parsed(""), SUMS, "yt-dlp_macos"),
            Err(ToolProblem::MissingDigest)
        );
        // Two publishers, two answers.
        let other = format!("{}  yt-dlp_macos\n", "0".repeat(64));
        assert_eq!(
            with_published_digest(&parsed(MACOS), &other, "yt-dlp_macos"),
            Err(ToolProblem::DigestConflict)
        );
        assert_eq!(
            with_published_digest(&parsed(""), "", "yt-dlp_macos"),
            Err(ToolProblem::MissingDigest)
        );
    }

    #[test]
    fn bad_releases_are_refused_one_reason_at_a_time() {
        let refused = |json: &str| parse_release(json, "yt-dlp_macos", "SHA2-256SUMS").unwrap_err();
        assert_eq!(refused(""), ToolProblem::Empty);
        assert_eq!(refused("<html>rate limited</html>"), ToolProblem::NotJson);
        assert_eq!(refused("[]"), ToolProblem::NotJson);
        assert_eq!(
            refused(&RELEASE.replace("\"draft\":false", "\"draft\":true")),
            ToolProblem::Unpublished
        );
        assert_eq!(
            refused(&RELEASE.replace("\"prerelease\":false", "\"prerelease\":true")),
            ToolProblem::Unpublished
        );
        assert_eq!(
            refused(&RELEASE.replace("\"tag_name\":\"2026.08.19\"", "\"tag_name\":\"\"")),
            ToolProblem::MissingVersion
        );
        assert_eq!(
            refused(&RELEASE.replace("\"tag_name\":\"2026.08.19\"", "\"tag_name\":\"nightly\"")),
            ToolProblem::BadVersion
        );
        assert_eq!(
            refused("{\"tag_name\":\"2026.08.19\"}"),
            ToolProblem::MissingAsset
        );
        assert_eq!(
            refused(&RELEASE.replace("\"name\":\"yt-dlp_macos\"", "\"name\":\"yt-dlp_linux2\"")),
            ToolProblem::MissingAsset
        );
        // Not GitHub, or not https: refused before anything is read further.
        for elsewhere in [
            "https://evil.invalid/yt-dlp_macos",
            "http://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos",
            "https://github.com.evil.invalid/yt-dlp_macos",
            "https://objects.githubusercontent.com/yt-dlp_macos",
        ] {
            assert_eq!(
                refused(&RELEASE.replace(
                    "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos\"",
                    &format!("{elsewhere}\"")
                )),
                ToolProblem::BadUrl,
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn the_api_address_is_github_unless_a_test_says_otherwise() {
        assert_eq!(
            latest_release_url("yt-dlp/yt-dlp", None),
            "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest"
        );
        assert_eq!(
            latest_release_url("/yt-dlp/yt-dlp/", Some("http://127.0.0.1:5/")),
            "http://127.0.0.1:5/repos/yt-dlp/yt-dlp/releases/latest"
        );
    }
}
