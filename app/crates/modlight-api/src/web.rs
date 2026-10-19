//! Requests to addresses the person typed or copied, not to the account's server
//! (`WebHelpers`): fetching a file for "Upload from URL", asking a server what a file is
//! called, fetching a picture to preview. They carry no credential at all, and they
//! follow redirects like any download would (the account's token never travels here).

use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE, USER_AGENT};
use tokio::io::AsyncWriteExt;

use crate::client::Api;
use crate::error::{ApiError, ApiResult};

/// The longest a preview picture may be.
const MAX_PREVIEW_BYTES: usize = 50 * 1024 * 1024;

fn web_client() -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(ApiError::from_reqwest)
}

fn user_agent() -> String {
    format!("Modlight/{}", env!("CARGO_PKG_VERSION"))
}

/// Only web addresses are fetched; anything else is refused before a socket opens.
fn check(url: &str) -> ApiResult<()> {
    match url::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.has_host() => Ok(()),
        _ => Err(ApiError::Transport(format!("not a web address: {url}"))),
    }
}

/// The file name in a `Content-Disposition: ...; filename="..."` value.
pub fn file_name_from_disposition(value: &str) -> Option<String> {
    let marker = "filename=\"";
    let lower = value.to_ascii_lowercase();
    let begin = lower.find(marker)? + marker.len();
    let rest = &value[begin..];
    let end = rest.find('"')?;
    let name = &rest[..end];
    (!name.is_empty()).then(|| name.to_string())
}

impl Api {
    /// `GetFileNameFromWebServerAsync`: a HEAD request, and the name the server gives
    /// the file in `Content-Disposition`, when it gives one.
    pub async fn file_name_from_server(&self, url: &str) -> Option<String> {
        check(url).ok()?;
        let client = web_client().ok()?;
        let response = client
            .head(url)
            .header(USER_AGENT, user_agent())
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .ok()?;
        let value = response.headers().get(CONTENT_DISPOSITION)?.to_str().ok()?;
        file_name_from_disposition(value)
    }

    /// `DownloadFileAsync`: the body of `url` into `path`, streamed. A status that is
    /// not a success is an error here (upstream wrote nothing and went on).
    pub async fn download_url_to_file(&self, url: &str, path: &Path) -> Result<(), String> {
        check(url).map_err(|e| e.to_string())?;
        let client = web_client().map_err(|e| e.to_string())?;
        let response = client
            .get(url)
            .header(USER_AGENT, user_agent())
            .send()
            .await
            .map_err(|e| ApiError::from_reqwest(e).to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "Response status code does not indicate success: {} ({}).",
                response.status().as_u16(),
                response.status().canonical_reason().unwrap_or("")
            ));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError::from_reqwest(e).to_string())?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        file.flush().await.map_err(|e| e.to_string())
    }

    /// `DownloadImageAsync`: the picture at `url` when the server says it is one.
    pub async fn download_image(&self, url: &str) -> Option<image_bytes::RgbaPicture> {
        check(url).ok()?;
        let client = web_client().ok()?;
        let response = client
            .get(url)
            .header(USER_AGENT, user_agent())
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let media = response
            .headers()
            .get(CONTENT_TYPE)?
            .to_str()
            .ok()?
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase();
        if !media.starts_with("image/") {
            return None;
        }
        let bytes = Api::read_limited(response, MAX_PREVIEW_BYTES).await.ok()??;
        image_bytes::decode(&bytes)
    }
}

/// Decoding stays out of this crate's dependencies: the bytes are handed back and the
/// caller (which has an image library) decodes them.
pub mod image_bytes {
    /// An encoded picture as downloaded.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct RgbaPicture {
        pub bytes: Vec<u8>,
    }

    pub fn decode(bytes: &[u8]) -> Option<RgbaPicture> {
        (!bytes.is_empty()).then(|| RgbaPicture {
            bytes: bytes.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disposition_names_the_file() {
        assert_eq!(
            file_name_from_disposition(r#"attachment; filename="report final.pdf""#).as_deref(),
            Some("report final.pdf")
        );
        assert_eq!(
            file_name_from_disposition(r#"inline; FILENAME="a.png"; size=3"#).as_deref(),
            Some("a.png")
        );
        assert_eq!(file_name_from_disposition("attachment"), None);
        assert_eq!(file_name_from_disposition(r#"attachment; filename="""#), None);
    }

    #[test]
    fn only_web_addresses_are_fetched() {
        assert!(check("https://example.com/a").is_ok());
        assert!(check("file:///etc/passwd").is_err());
        assert!(check("ftp://example.com/a").is_err());
    }
}
