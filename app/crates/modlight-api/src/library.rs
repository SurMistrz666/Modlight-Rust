//! The account's own files and folders (`ModlightLibrary.cs`).

use std::path::Path;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::client::{Api, Credential, REQUEST_TIMEOUT, parse};
use crate::error::{ApiError, ApiResult};
use crate::host::{escape_data_string, rewrite_to_host};
use crate::models::{
    ConvertVerdict, DeleteResult, Folder, FolderDeleteResult, FolderList, FolderResult,
    MotionPreview, UploadEdit, UploadPage, UploadPatchResult,
};

/// What the server means by "not in any folder" when it has not said otherwise.
pub const ROOT_FOLDER: &str = "root";

/// A picture is allowed to take longer than a listing: the first request for a size
/// makes the server render it.
pub const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(45);

/// The most bytes read for one still tile. A fence around a bug, not a working limit.
pub const MAX_THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;

/// The most bytes read for one MOVING tile (an animated WebP of a 30-frame clip).
pub const MAX_MOTION_BYTES: usize = 4 * 1024 * 1024;

/// Ids the delete endpoint accepts in one request: the server's own MAX_IDS.
pub const MAX_DELETE_IDS: usize = 500;

/// Which uploads to list (`GetUploadsAsync`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadQuery {
    pub page: i64,
    pub per_page: i64,
    /// A folder id, the root sentinel for "no folder", or `None` for "everywhere".
    pub folder_id: Option<String>,
    pub search: Option<String>,
    /// image, video, audio, document, archive, paste, or `None` for every kind.
    pub kind: Option<String>,
    pub trashed: bool,
}

impl UploadQuery {
    /// The query string, built in the order and with the escaping the Windows client uses.
    pub fn to_path(&self) -> String {
        let mut url = format!(
            "/api/uploads?page={}&perPage={}",
            self.page.max(1),
            self.per_page.max(1)
        );
        if let Some(folder) = self.folder_id.as_deref().filter(|f| !f.is_empty()) {
            url.push_str("&folderId=");
            url.push_str(&escape_data_string(folder));
        }
        if let Some(search) = self
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            url.push_str("&search=");
            url.push_str(&escape_data_string(search));
        }
        if let Some(kind) = self.kind.as_deref().filter(|k| !k.is_empty()) {
            url.push_str("&kind=");
            url.push_str(&escape_data_string(kind));
        }
        if self.trashed {
            url.push_str("&trashed=1");
        }
        url
    }
}

impl Api {
    /// Every folder the account has.
    pub async fn get_folders(&self, credential: &Credential) -> ApiResult<FolderList> {
        let text = self
            .get_string("/api/folders", credential, REQUEST_TIMEOUT)
            .await?;
        parse(&text)
    }

    /// One page of the account's files.
    pub async fn get_uploads(
        &self,
        credential: &Credential,
        query: &UploadQuery,
    ) -> ApiResult<UploadPage> {
        let text = self
            .get_string(&query.to_path(), credential, REQUEST_TIMEOUT)
            .await?;
        parse(&text)
    }

    /// Fetch one preview image, or `None` when the server has none to give.
    ///
    /// Anything the server answers with comes back as `None`; a connection that never
    /// completed is still an error, because that is a fact about the whole grid.
    pub async fn get_thumbnail(
        &self,
        credential: &Credential,
        thumbnail_url: &str,
    ) -> ApiResult<Option<Vec<u8>>> {
        let Some(url) = rewrite_to_host(thumbnail_url, &credential.host) else {
            return Ok(None);
        };
        let response = self
            .request(Method::GET, &url, credential)
            .timeout(THUMBNAIL_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        if !response.status().is_success() {
            return Ok(None);
        }
        Api::read_limited(response, MAX_THUMBNAIL_BYTES).await
    }

    /// Fetch one MOVING preview and say which kind of no it got.
    pub async fn get_motion_thumbnail(
        &self,
        credential: &Credential,
        motion_url: &str,
    ) -> ApiResult<MotionPreview> {
        let Some(url) = rewrite_to_host(motion_url, &credential.host) else {
            return Ok(MotionPreview::Refused);
        };
        let response = self
            .request(Method::GET, &url, credential)
            .timeout(THUMBNAIL_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if !status.is_success() {
            return Ok(
                if status == StatusCode::UNSUPPORTED_MEDIA_TYPE
                    || status == StatusCode::PAYLOAD_TOO_LARGE
                {
                    MotionPreview::Refused
                } else {
                    MotionPreview::Unavailable
                },
            );
        }
        Ok(match Api::read_limited(response, MAX_MOTION_BYTES).await? {
            Some(bytes) if !bytes.is_empty() => MotionPreview::Ok(bytes),
            _ => MotionPreview::Unavailable,
        })
    }

    /// Move one file to the account's trash. True when the server moved it.
    pub async fn trash(&self, credential: &Credential, id: &str) -> ApiResult<bool> {
        if id.is_empty() {
            return Ok(false);
        }
        let result = self
            .delete_uploads(credential, &[id.to_string()], "trash")
            .await?;
        // The count, not the status: an id the account does not own is answered 200.
        Ok(result.affected > 0)
    }

    /// Change files: move, publish, rename the URL, or set the link's gates.
    pub async fn edit_uploads(
        &self,
        credential: &Credential,
        ids: &[String],
        edit: &UploadEdit,
    ) -> ApiResult<UploadPatchResult> {
        let mut body = serde_json::Map::new();
        body.insert("ids".into(), json!(ids));
        for (key, value) in edit.fields() {
            body.insert(key.clone(), value.clone());
        }
        let text = self
            .send_json(
                Method::PATCH,
                "/api/uploads",
                credential,
                &serde_json::Value::Object(body),
                REQUEST_TIMEOUT,
            )
            .await?;
        parse(&text)
    }

    /// Trash, restore, or delete for good (`mode` is `trash`, `restore` or `permanent`).
    pub async fn delete_uploads(
        &self,
        credential: &Credential,
        ids: &[String],
        mode: &str,
    ) -> ApiResult<DeleteResult> {
        if ids.is_empty() {
            return Ok(DeleteResult::default());
        }
        let text = self
            .send_json(
                Method::DELETE,
                "/api/uploads",
                credential,
                &json!({ "ids": ids, "mode": mode }),
                REQUEST_TIMEOUT,
            )
            .await?;
        parse(&text)
    }

    /// Make a folder, at the top of the library or inside another one.
    pub async fn create_folder(
        &self,
        credential: &Credential,
        name: &str,
        parent_id: Option<&str>,
    ) -> ApiResult<Option<Folder>> {
        let mut body = serde_json::Map::new();
        body.insert("name".into(), json!(name));
        if let Some(parent) = parent_id.filter(|p| !p.is_empty()) {
            body.insert("parentId".into(), json!(parent));
        }
        let text = self
            .send_json(
                Method::POST,
                "/api/folders",
                credential,
                &serde_json::Value::Object(body),
                REQUEST_TIMEOUT,
            )
            .await?;
        Ok(parse::<FolderResult>(&text)?.folder)
    }

    /// Give a folder a different name. Its public gallery address follows.
    pub async fn rename_folder(
        &self,
        credential: &Credential,
        id: &str,
        name: &str,
    ) -> ApiResult<Option<Folder>> {
        self.patch_folder(credential, json!({ "id": id, "name": name }))
            .await
    }

    /// Turn the folder's public gallery on or off.
    pub async fn set_folder_visibility(
        &self,
        credential: &Credential,
        id: &str,
        is_public: bool,
    ) -> ApiResult<Option<Folder>> {
        self.patch_folder(credential, json!({ "id": id, "isPublic": is_public }))
            .await
    }

    /// Remove a folder, keeping (`false`) or trashing (`true`) what is inside it.
    pub async fn delete_folder(
        &self,
        credential: &Credential,
        id: &str,
        with_contents: bool,
    ) -> ApiResult<FolderDeleteResult> {
        let text = self
            .send_json(
                Method::DELETE,
                "/api/folders",
                credential,
                &json!({ "id": id, "withContents": with_contents }),
                REQUEST_TIMEOUT,
            )
            .await?;
        parse(&text)
    }

    async fn patch_folder(
        &self,
        credential: &Credential,
        body: serde_json::Value,
    ) -> ApiResult<Option<Folder>> {
        let text = self
            .send_json(
                Method::PATCH,
                "/api/folders",
                credential,
                &body,
                REQUEST_TIMEOUT,
            )
            .await?;
        Ok(parse::<FolderResult>(&text)?.folder)
    }

    /// Ask whether a conversion would work, without making it happen (`?check=1`).
    pub async fn check_conversion(
        &self,
        credential: &Credential,
        convert_url: &str,
    ) -> ConvertVerdict {
        let Some(mut url) = rewrite_to_host(convert_url, &credential.host) else {
            return ConvertVerdict::no("missing");
        };
        url.push_str(if url.contains('?') {
            "&check=1"
        } else {
            "?check=1"
        });

        let response = match self
            .request(Method::GET, &url, credential)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return ConvertVerdict::no("timeout"),
            Err(_) => return ConvertVerdict::no("offline"),
        };

        if response.status().is_success() {
            ConvertVerdict::yes()
        } else {
            ConvertVerdict::no(refusal_of(&response).as_deref().unwrap_or(""))
        }
    }

    /// Pull one file down to disk, streaming rather than buffering it.
    ///
    /// The bytes go to a hidden file beside `path` that takes its name only once the
    /// whole body has arrived, and is removed when it does not: a truncated download
    /// left lying about with a plausible name is worse than no download, and a file the
    /// person agreed to replace stays as it was until there is something to replace it
    /// with. (Upstream wrote straight into `path` and deleted it on any failure, which
    /// took the old file with it even when nothing had been received.)
    pub async fn download_to_file(
        &self,
        credential: &Credential,
        link: &str,
        path: &Path,
    ) -> ApiResult<()> {
        use futures_util::StreamExt;

        let Some(url) = rewrite_to_host(link, &credential.host) else {
            return Err(ApiError::Refused {
                status: 400,
                detail: String::new(),
                reason: Some("missing".into()),
            });
        };
        let response = self
            .request(Method::GET, &url, credential)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Refused {
                status: status.as_u16(),
                detail: String::new(),
                reason: refusal_of(&response),
            });
        }

        let part = part_path(path);
        let written = async {
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part)
                .await
                .map_err(|e| ApiError::Transport(e.to_string()))?;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(ApiError::from_reqwest)?;
                file.write_all(&chunk)
                    .await
                    .map_err(|e| ApiError::Transport(e.to_string()))?;
            }
            file.flush()
                .await
                .map_err(|e| ApiError::Transport(e.to_string()))?;
            file.sync_all()
                .await
                .map_err(|e| ApiError::Transport(e.to_string()))?;
            drop(file);
            tokio::fs::rename(&part, path)
                .await
                .map_err(|e| ApiError::Transport(e.to_string()))
        }
        .await;
        if written.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        written
    }
}

/// Where a download is written until it is whole: a hidden name beside `path`, unique to
/// this download, so a rename moves it into place on the same volume.
fn part_path(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tag: u64 = rand::random();
    path.with_file_name(format!(".{name}.{tag:016x}.part"))
}

/// The reason a conversion was refused (`X-Convert-Refused`), or `None` when unnamed.
fn refusal_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get("X-Convert-Refused")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_urls_match_the_windows_client() {
        let query = UploadQuery {
            page: 0,
            per_page: 120,
            folder_id: Some("root".into()),
            search: Some("  a b ".into()),
            kind: Some("image".into()),
            trashed: true,
        };
        assert_eq!(
            query.to_path(),
            "/api/uploads?page=1&perPage=120&folderId=root&search=a%20b&kind=image&trashed=1"
        );
        assert_eq!(
            UploadQuery {
                page: 3,
                per_page: 30,
                ..Default::default()
            }
            .to_path(),
            "/api/uploads?page=3&perPage=30"
        );
    }

    /// A one-shot server on 127.0.0.1 that answers the first request with `reply`
    /// (written as is, so it can promise more body than it sends) and hangs up.
    async fn answer_once(reply: Vec<u8>) -> String {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut head = Vec::new();
            let mut buffer = [0u8; 4096];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buffer[..n]),
                }
            }
            let _ = stream.write_all(&reply).await;
            let _ = stream.shutdown().await;
        });
        host
    }

    fn leftovers(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    /// The person agreed to replace a file of the same name; a download that does not
    /// finish must leave that file as it was, not truncated and not deleted.
    #[tokio::test]
    async fn a_download_that_does_not_finish_leaves_the_old_file_alone() {
        let api = Api::new("test").unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("report.pdf");
        std::fs::write(&path, b"the old report").unwrap();

        // Promised 100 bytes, sent 10, hung up.
        let host = answer_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n0123456789"
                .to_vec(),
        )
        .await;
        let credential = Credential::new(host.as_str(), "token");
        let cut = api
            .download_to_file(&credential, &format!("{host}/a/report/download"), &path)
            .await;
        assert!(cut.is_err(), "a short body is a failure: {cut:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"the old report");
        assert_eq!(leftovers(folder.path()), vec!["report.pdf"]);

        // Refused before a byte arrived.
        let host = answer_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        )
        .await;
        let credential = Credential::new(host.as_str(), "token");
        let refused = api
            .download_to_file(&credential, &format!("{host}/a/report/download"), &path)
            .await;
        assert_eq!(refused.unwrap_err().status(), Some(404));
        assert_eq!(std::fs::read(&path).unwrap(), b"the old report");
        assert_eq!(leftovers(folder.path()), vec!["report.pdf"]);

        // A whole download replaces it.
        let host = answer_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nnew report"
                .to_vec(),
        )
        .await;
        let credential = Credential::new(host.as_str(), "token");
        api.download_to_file(&credential, &format!("{host}/a/report/download"), &path)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new report");
        assert_eq!(leftovers(folder.path()), vec!["report.pdf"]);
    }
}
