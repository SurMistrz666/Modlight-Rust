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

/// How long a download may go without receiving anything: HttpClient's default
/// timeout, which bounds upstream's `DownloadFileAsync`. Measured from the last bytes
/// rather than over the whole file, so a large file on a slow line still arrives.
const STALL_TIMEOUT: Duration = Duration::from_secs(100);

fn web_client() -> ApiResult<reqwest::Client> {
    web_client_stalling_after(STALL_TIMEOUT)
}

fn web_client_stalling_after(stall: Duration) -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(stall)
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
    /// not a success is an error here (upstream wrote nothing and went on), and so is a
    /// server that stops sending for [`STALL_TIMEOUT`].
    ///
    /// `path` is written only once the whole body is there (a part file beside it is
    /// renamed over it), as upstream's `GetAsync` read the whole answer before it opened
    /// the file: a download that fails leaves no half file under the name, and a file
    /// the person agreed to overwrite stays as it was.
    pub async fn download_url_to_file(&self, url: &str, path: &Path) -> Result<(), String> {
        download_to_file(url, path, STALL_TIMEOUT).await
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
        let bytes = Api::read_limited(response, MAX_PREVIEW_BYTES)
            .await
            .ok()??;
        image_bytes::decode(&bytes)
    }
}

/// [`Api::download_url_to_file`], ending when nothing arrives for `stall`.
async fn download_to_file(url: &str, path: &Path, stall: Duration) -> Result<(), String> {
    check(url).map_err(|e| e.to_string())?;
    let client = web_client_stalling_after(stall).map_err(|e| e.to_string())?;
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
    let part = crate::library::part_path(path);
    let written = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .await
            .map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError::from_reqwest(e).to_string())?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        tokio::fs::rename(&part, path)
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    if written.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    written
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
        assert_eq!(
            file_name_from_disposition(r#"attachment; filename="""#),
            None
        );
    }

    #[test]
    fn only_web_addresses_are_fetched() {
        assert!(check("https://example.com/a").is_ok());
        assert!(check("file:///etc/passwd").is_err());
        assert!(check("ftp://example.com/a").is_err());
    }

    /// A server that stops sending in the middle of the body: the download must end
    /// (upstream's `DownloadFileAsync` runs inside HttpClient's 100 s timeout), not leave
    /// the Upload from URL task downloading for ever.
    #[tokio::test]
    async fn a_download_that_stalls_ends() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n0123456789")
                        .await;
                    // Then nothing more, with the connection held open.
                    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                });
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let ended = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            download_to_file(
                &url,
                &dir.path().join("file.bin"),
                std::time::Duration::from_millis(500),
            ),
        )
        .await;
        let result = ended.expect("the download ends when the server stops sending");
        assert!(result.is_err());
    }

    /// Upload from URL onto a name that is taken, after the person said "overwrite": a
    /// body that ends short is a failure that leaves the old file as it was and no half
    /// file under the name (upstream's `GetAsync` read the whole answer before it opened
    /// the file). A whole body replaces it.
    #[tokio::test]
    async fn a_failed_download_leaves_the_file_it_would_replace() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn answer_once(reply: &'static [u8]) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/shot.png", listener.local_addr().unwrap());
            tokio::spawn(async move {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(reply).await;
                let _ = stream.shutdown().await;
            });
            url
        }
        let names = |dir: &Path| {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, b"the old picture").unwrap();

        // Promised 100 bytes, sent 10, hung up.
        let url = answer_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n0123456789",
        )
        .await;
        let cut = download_to_file(&url, &path, Duration::from_secs(5)).await;
        assert!(cut.is_err(), "a short body is a failure: {cut:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"the old picture");
        assert_eq!(names(dir.path()), ["shot.png"]);

        let url = answer_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nnew picture",
        )
        .await;
        download_to_file(&url, &path, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new picture");
        assert_eq!(names(dir.path()), ["shot.png"]);
    }
}
