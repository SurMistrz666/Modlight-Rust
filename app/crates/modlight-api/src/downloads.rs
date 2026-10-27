//! `GET /api/downloads`: the builds of the desktop app the server keeps ("Downloads"), per
//! system, newest first. The app reads it to update itself; the build scripts publish into
//! it (`app/scripts/build-app.sh`, `build-app.ps1`).
//!
//! ```text
//! {"platforms":[{"platform":"macos","builds":[Build...]},{"platform":"windows","builds":[...]}],
//!  "keep":5,"admin":false,"publishToken":null}
//! Build = {"id","platform","arch","version","build","built","published","size","sha256",
//!          "filename","uploader","note","url":"/downloads/{id}/{filename}"}
//! ```
//!
//! `GET {server}{url}` with the same credential returns the build's bytes. Read leniently,
//! like every other answer here: a field of an unexpected type reads as absent, so one odd
//! build never hides the others. What a build must have before it is offered is decided by
//! the updater (`modlight_core::update`), not here.

use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Method, StatusCode};
use serde_json::Value;

use crate::client::{Api, Credential, REQUEST_TIMEOUT};
use crate::error::{ApiError, ApiResult};
use crate::models::parse_time;

/// Where the server lists its builds.
pub const DOWNLOADS_PATH: &str = "/api/downloads";
/// A listing keeps a few builds per system; this is far more than any real one.
pub const MAX_DOWNLOADS_BYTES: usize = 1024 * 1024;

/// The answer to `GET /api/downloads`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadsListing {
    pub platforms: Vec<DownloadsPlatform>,
    /// How many builds per system the server keeps.
    pub keep: Option<i64>,
    /// Whether the account may publish and delete builds (advisory).
    pub admin: bool,
    /// Whether a publish token exists; `None` when the server does not say (not an admin).
    pub publish_token: Option<bool>,
}

impl DownloadsListing {
    /// The builds of one system (`macos`, `windows`), newest first as the server sent them.
    pub fn builds_for(&self, platform: &str) -> impl Iterator<Item = &PublishedBuild> {
        self.platforms
            .iter()
            .filter(move |p| p.platform.eq_ignore_ascii_case(platform))
            .flat_map(|p| p.builds.iter())
    }
}

/// One system's builds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadsPlatform {
    pub platform: String,
    pub builds: Vec<PublishedBuild>,
}

/// One published build.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PublishedBuild {
    pub id: String,
    /// `macos` or `windows`.
    pub platform: String,
    /// `arm64` or `x64`.
    pub arch: String,
    /// The app's version (Cargo), e.g. `0.7.8`.
    pub version: String,
    /// The commit it was built from (short hash), empty when it was built outside git.
    pub build: String,
    /// When it was compiled (UTC), as the build script wrote it into the app.
    pub built: Option<DateTime<Utc>>,
    /// When it was published.
    pub published: Option<DateTime<Utc>>,
    /// Bytes of the zip.
    pub size: i64,
    /// Hex SHA-256 of the zip.
    pub sha256: String,
    pub filename: String,
    /// Who published it, as the server names them.
    pub uploader: String,
    /// What changed, in the publisher's words.
    pub note: String,
    /// Root-relative address of the zip on the server: `/downloads/{id}/{filename}`.
    pub url: String,
}

impl Api {
    /// `GET /api/downloads` with the credential. 401 and 403 are
    /// [`ApiError::Unauthorized`]; 404 (a server without Downloads) and every other
    /// failure are [`ApiError::Status`]. Redirects are not followed (no call follows them).
    pub async fn get_downloads(&self, credential: &Credential) -> ApiResult<DownloadsListing> {
        let response = self
            .request(Method::GET, &credential.url(DOWNLOADS_PATH), credential)
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16()));
        }
        let Some(bytes) = Api::read_limited(response, MAX_DOWNLOADS_BYTES).await? else {
            return Err(ApiError::Decode(
                "the downloads listing is too large".into(),
            ));
        };
        parse_downloads(&String::from_utf8_lossy(&bytes))
    }
}

/// Read a `GET /api/downloads` answer. Anything that is not a JSON object with a
/// `platforms` array is not a listing.
pub fn parse_downloads(text: &str) -> ApiResult<DownloadsListing> {
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| ApiError::Decode(e.to_string()))?;
    let Some(platforms) = root.get("platforms").and_then(Value::as_array) else {
        return Err(ApiError::Decode("not a downloads listing".into()));
    };
    Ok(DownloadsListing {
        platforms: platforms
            .iter()
            .filter(|p| p.is_object())
            .map(|p| DownloadsPlatform {
                platform: text_of(p.get("platform")),
                builds: p
                    .get("builds")
                    .and_then(Value::as_array)
                    .map(|builds| {
                        builds
                            .iter()
                            .filter(|b| b.is_object())
                            .map(parse_build_value)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
        keep: root.get("keep").and_then(Value::as_i64),
        admin: root.get("admin").and_then(Value::as_bool).unwrap_or(false),
        publish_token: root.get("publishToken").and_then(Value::as_bool),
    })
}

/// One build object of the listing (also what `POST .../finish` answers).
pub fn parse_build(text: &str) -> ApiResult<PublishedBuild> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| ApiError::Decode(e.to_string()))?;
    if !value.is_object() {
        return Err(ApiError::Decode("not a build".into()));
    }
    Ok(parse_build_value(&value))
}

fn parse_build_value(value: &Value) -> PublishedBuild {
    PublishedBuild {
        id: text_of(value.get("id")),
        platform: text_of(value.get("platform")),
        arch: text_of(value.get("arch")),
        version: text_of(value.get("version")),
        build: text_of(value.get("build")),
        built: time_of(value.get("built")),
        published: time_of(value.get("published")),
        size: value
            .get("size")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
            .unwrap_or(0),
        sha256: text_of(value.get("sha256")),
        filename: text_of(value.get("filename")),
        uploader: match value.get("uploader") {
            // An account the server describes as an object: its name, else its email.
            Some(Value::Object(account)) => {
                let name = text_of(account.get("name"));
                if name.is_empty() {
                    text_of(account.get("email"))
                } else {
                    name
                }
            }
            other => text_of(other),
        },
        note: text_of(value.get("note")),
        url: text_of(value.get("url")),
    }
}

/// A string field, trimmed; a number is written out; anything else is empty.
fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

/// A time the server wrote as ISO 8601 (with or without an offset), or as a number of
/// milliseconds (seconds when it is too small to be milliseconds) since 1970.
pub fn time_of(value: Option<&Value>) -> Option<DateTime<Utc>> {
    match value? {
        Value::String(text) => parse_time(text.trim()),
        Value::Number(number) => {
            let value = number.as_i64()?;
            if value.abs() >= 100_000_000_000 {
                Utc.timestamp_millis_opt(value).single()
            } else {
                Utc.timestamp_opt(value, 0).single()
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    const LISTING: &str = r#"{
        "platforms": [
            {"platform": "macos", "builds": [
                {"id": "m2", "platform": "macos", "arch": "arm64", "version": "0.7.9",
                 "build": "9f8e7d6c", "built": "2026-10-27T14:03:05Z",
                 "published": "2026-10-27T14:10:00.000Z", "size": 41200000,
                 "sha256": "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824",
                 "filename": "Modlight-0.7.9-9f8e7d6c-macos-arm64.zip",
                 "uploader": {"name": "Alex", "email": "alex@example.com"},
                 "note": "Faster uploads", "url": "/downloads/m2/Modlight-0.7.9-9f8e7d6c-macos-arm64.zip"},
                {"id": "m1", "arch": "arm64", "version": "0.7.8", "build": "", "built": 1761000000000,
                 "size": "1000", "sha256": null, "filename": "x.zip", "uploader": "ci", "url": "/downloads/m1/x.zip"},
                "not a build"
            ]},
            {"platform": "windows", "builds": []},
            {"platform": "linux"}
        ],
        "keep": 5, "admin": true, "publishToken": null
    }"#;

    #[test]
    fn a_listing_reads_leniently() {
        let listing = parse_downloads(LISTING).unwrap();
        assert_eq!(listing.keep, Some(5));
        assert!(listing.admin);
        assert_eq!(listing.publish_token, None);
        assert_eq!(listing.platforms.len(), 3);
        let mac: Vec<_> = listing.builds_for("MacOS").collect();
        assert_eq!(mac.len(), 2, "the stray string is not a build");
        let newest = mac[0];
        assert_eq!(newest.id, "m2");
        assert_eq!(newest.build, "9f8e7d6c");
        assert_eq!(
            newest.built,
            Some(Utc.with_ymd_and_hms(2026, 10, 27, 14, 3, 5).unwrap())
        );
        assert!(newest.published.is_some());
        assert_eq!(newest.size, 41_200_000);
        assert_eq!(newest.uploader, "Alex");
        assert_eq!(newest.note, "Faster uploads");
        let older = mac[1];
        assert_eq!(older.platform, "", "a missing field is empty, not an error");
        assert_eq!(older.size, 1000);
        assert_eq!(older.sha256, "");
        assert_eq!(older.uploader, "ci");
        assert_eq!(
            older.built,
            Utc.timestamp_millis_opt(1_761_000_000_000).single()
        );
        assert_eq!(listing.builds_for("windows").count(), 0);
        assert_eq!(listing.builds_for("linux").count(), 0);
    }

    #[test]
    fn what_is_not_a_listing_is_an_error() {
        assert!(parse_downloads("").is_err());
        assert!(parse_downloads("<html>").is_err());
        assert!(parse_downloads("[]").is_err());
        assert!(parse_downloads(r#"{"platforms": 3}"#).is_err());
        assert_eq!(
            parse_downloads(r#"{"platforms": []}"#).unwrap(),
            DownloadsListing::default()
        );
        assert_eq!(parse_build(r#"{"id":"x","size":3}"#).unwrap().size, 3);
        assert!(parse_build("[1]").is_err());
    }

    #[test]
    fn times_are_iso_or_milliseconds_or_seconds() {
        let at = Utc.with_ymd_and_hms(2026, 10, 27, 14, 3, 5).unwrap();
        assert_eq!(
            time_of(Some(&Value::from("2026-10-27T14:03:05Z"))),
            Some(at)
        );
        assert_eq!(
            time_of(Some(&Value::from("2026-10-27T16:03:05+02:00"))),
            Some(at)
        );
        assert_eq!(time_of(Some(&Value::from("2026-10-27T14:03:05"))), Some(at));
        assert_eq!(time_of(Some(&Value::from(at.timestamp_millis()))), Some(at));
        assert_eq!(time_of(Some(&Value::from(at.timestamp()))), Some(at));
        assert_eq!(time_of(Some(&Value::from("yesterday"))), None);
        assert_eq!(time_of(Some(&Value::Null)), None);
        assert_eq!(time_of(None), None);
    }

    /// A stand-in that answers once with `status` and `body`, and hands back the request.
    async fn answering_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        (origin, seen)
    }

    #[tokio::test]
    async fn the_listing_is_asked_with_the_token_in_its_header_only() {
        let api = Api::new("0.7.7").unwrap();
        let (origin, seen) = answering_once("200 OK", r#"{"platforms":[]}"#).await;
        let credential = Credential::new(origin.as_str(), "token-0123456789");
        assert_eq!(
            api.get_downloads(&credential).await,
            Ok(DownloadsListing::default())
        );
        let request = seen.await.unwrap();
        let first = request.lines().next().unwrap();
        assert_eq!(first, "GET /api/downloads HTTP/1.1");
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("x-api-key: token-0123456789"))
        );
        assert!(!first.contains("token-0123456789"));
    }

    #[tokio::test]
    async fn refusals_keep_their_meaning() {
        let api = Api::new("0.7.7").unwrap();
        for (status, expected) in [
            ("401 Unauthorized", ApiError::Unauthorized),
            ("403 Forbidden", ApiError::Unauthorized),
            ("404 Not Found", ApiError::Status(404)),
            ("302 Found", ApiError::Status(302)),
            ("500 Internal Server Error", ApiError::Status(500)),
        ] {
            let (origin, _) = answering_once(status, "{}").await;
            let credential = Credential::new(origin.as_str(), "token-0123456789");
            assert_eq!(
                api.get_downloads(&credential).await,
                Err(expected),
                "{status}"
            );
        }
        let (origin, _) = answering_once("200 OK", "<html>").await;
        let credential = Credential::new(origin.as_str(), "token-0123456789");
        assert!(matches!(
            api.get_downloads(&credential).await,
            Err(ApiError::Decode(_))
        ));
    }
}
