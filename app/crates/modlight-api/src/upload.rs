//! The live upload: one streamed `POST /upload`, as the Windows client sends it
//! (`ShareX.UploadersLib/FileUploaders/Modlight.cs` on top of `Uploader.SendRequestFile`,
//! `TransferData` and `ProcessWebResponse`, and `Helpers/RequestHelpers.cs`).
//!
//! The app sends only files up to one tus chunk (50 MiB) this way: a larger one goes over
//! tus in chunks (`modlight_core::tasks::uploader`), because a proxy such as Cloudflare's
//! refuses a single request body over 100 MB before the server sees it.
//!
//! On the wire, what ShareX writes:
//!
//! * the boundary is twenty dashes and `DateTime.Now.Ticks` in lower-case hex;
//! * the form fields go FIRST (`folderId`, and only when a folder was named), then the
//!   file part `name="upload"; filename="<name>"` with `Content-Type: <type of the name>`,
//!   then the closing boundary;
//! * `Content-Length` is always stated and the body is never chunked: the server's proxy
//!   is set not to buffer request bodies, and chunked is the one shape that can still
//!   make it decide to;
//! * the token travels in `x-api-key` only, which is what lets the server lift its size
//!   ceiling for this request before it has read a byte of the body;
//! * no deadline: a capture of several gigabytes legitimately takes hours.
//!
//! The source is read and written in chunks of the caller's buffer size
//! (`2^BufferSizePower` KiB in the settings), and progress is told after each chunk as
//! (bytes of the file sent so far, file length): the numbers `TransferData` gives its
//! `ProgressManager`. Smoothing them into a speed is the caller's business.
//!
//! What comes back keeps three endings apart, because each has a different remedy: the
//! server answered (whatever it said), nothing answered, or the caller stopped it. The
//! sentences `ReportRefusal` puts in front of the user live with the other translated
//! texts; [`ServerAnswer::refusal`] and [`ServerAnswer::server_detail`] are their inputs.
//!
//! Where this differs from upstream, and why:
//!
//! * Redirects are never followed (`docs/PORT.md`, section 5): a 3xx is an answer that is
//!   not a success, so the final URL is always the one asked.
//! * The reason phrase is the standard one for the status: the HTTP stack underneath does
//!   not hand on the phrase the server wrote. The reference server writes standard ones.
//! * An answer that arrives before the body is finished (a refusal decided from the
//!   headers, such as the room check against `Content-Length`) is returned at once and
//!   the rest of the body is not sent. `HttpWebRequest` wrote the whole body first.
//! * reqwest always adds `accept: */*` and writes header names in lower case; the server
//!   reads neither.
//! * A `"`, CR or LF in the file name goes out as `%22`, `%0D`, `%0A`, as browsers write
//!   them. No such name can exist on Windows, so upstream never sent one, and written raw
//!   it would end the header early.

use std::fmt;
use std::io;
use std::path::PathBuf;

use chrono::{NaiveDate, NaiveDateTime};
use futures_util::Stream;
use reqwest::Body;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::client::{API_KEY_HEADER, Api, Credential};
use crate::error::ApiError;
use crate::extras::server_text;
use crate::mime::mime_type_from_file_name;
use crate::models::UploadResponse;

/// Where the live upload is sent.
pub const UPLOAD_PATH: &str = "/upload";

/// The multipart field the server reads the file from (`ModlightAPI.FileFormName`).
pub const FILE_FORM_NAME: &str = "upload";

/// The multipart field it reads the target folder from (`ModlightAPI.FolderFieldName`).
pub const FOLDER_FIELD_NAME: &str = "folderId";

/// `BufferSizePower` 5, the setting's default: 32 KiB per write.
pub const DEFAULT_BUFFER_SIZE: usize = 32 * 1024;

/// The largest power the settings offer (8 MiB per write).
const MAX_BUFFER_SIZE_POWER: u32 = 13;

/// Bytes per write for the `BufferSizePower` setting: `2^power` KiB, as `WorkerTask`
/// computes it. Clamped to the range the settings page offers, so a hand-edited value
/// cannot ask for gigabytes per write.
pub fn buffer_size_for_power(power: u32) -> usize {
    (1usize << power.min(MAX_BUFFER_SIZE_POWER)) * 1024
}

/// Where the bytes of a live upload come from.
#[derive(Clone, PartialEq, Eq)]
pub enum UploadSource {
    /// A file, read as it is sent. Its length is taken when the upload starts and
    /// exactly that many bytes are sent.
    File(PathBuf),
    /// Bytes already in memory: a capture that is never saved, a paste.
    Bytes(Vec<u8>),
}

impl fmt::Debug for UploadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadSource::File(path) => f.debug_tuple("File").field(path).finish(),
            // The bytes themselves are somebody's screenshot, and useless in a log.
            UploadSource::Bytes(bytes) => write!(f, "Bytes({} bytes)", bytes.len()),
        }
    }
}

/// One live upload: what to send, under which name, into which folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveUpload {
    /// The name the server stores, after the caller's own clean-up of it (bidi controls,
    /// reserved characters); this sends it as given.
    pub file_name: String,
    pub source: UploadSource,
    /// `None` says nothing, and the server files the upload in the account's default
    /// folder; the root sentinel asks for no folder; an id asks for that folder. An id
    /// the account does not own is not a failure: the file is stored in the default
    /// folder and the answer carries `folder_warning`.
    pub folder_id: Option<String>,
    /// Bytes per write; see [`buffer_size_for_power`].
    pub buffer_size: usize,
}

impl LiveUpload {
    /// An upload with no folder preference and the default buffer.
    pub fn new(file_name: impl Into<String>, source: UploadSource) -> LiveUpload {
        LiveUpload {
            file_name: file_name.into(),
            source,
            folder_id: None,
            buffer_size: DEFAULT_BUFFER_SIZE,
        }
    }

    /// Aim the upload at a folder; `None` or an empty id leaves the choice to the account.
    pub fn with_folder(mut self, folder_id: Option<&str>) -> LiveUpload {
        self.folder_id = folder_id.filter(|id| !id.is_empty()).map(str::to_string);
        self
    }

    pub fn with_buffer_size(mut self, buffer_size: usize) -> LiveUpload {
        self.buffer_size = buffer_size;
        self
    }
}

/// How a live upload ended.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveOutcome {
    /// The server answered, whatever the status (`LastResponseInfo` was set).
    Answered(Box<ServerAnswer>),
    /// Nothing came back: DNS, a refused or reset connection, TLS, or a source that could
    /// not be read. The text is the innermost cause, which is the part a person can act
    /// on; there is no answer to explain, so the caller reports this as it stands.
    NoAnswer(String),
    /// The caller cancelled. Nothing to report: `TransferData` returning null.
    Cancelled,
}

/// Everything upstream's `ResponseInfo` held about an answer, plus the parsed body.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerAnswer {
    pub status: u16,
    /// The standard phrase for `status` (see the module notes).
    pub reason_phrase: String,
    /// Where the answer came from: always the URL asked, since redirects are not followed.
    pub url: String,
    /// In the order received, names in lower case.
    pub headers: Vec<(String, String)>,
    /// Read as UTF-8 whatever the headers say, like `StreamReader(stream, Encoding.UTF8)`.
    pub body: String,
    /// The body as `ModlightUploadResponse`, when the status is 2xx and the body parses.
    pub response: Option<UploadResponse>,
}

/// Which refusal a non-success answer is, for the sentence `ReportRefusal` shows first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadRefusal {
    /// 400: `Modlight_Request_rejected`.
    RequestRejected,
    /// 401: `Modlight_Account_disabled`.
    AccountDisabled,
    /// 404: `Modlight_Token_not_recognized`.
    TokenNotRecognized,
    /// 409: `Modlight_Address_taken`.
    AddressTaken,
    /// 413: `Modlight_File_too_large`.
    FileTooLarge,
    /// 507: `Modlight_Out_of_space`, the one refusal that trying again cannot fix.
    OutOfSpace,
    /// Anything else: `Modlight_Server_refused` with this number in it.
    Refused(u16),
}

impl ServerAnswer {
    /// 2xx: what `HttpWebRequest` returned without throwing.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The stored upload, when this is a success that names a link. A success without one
    /// is what upstream reports as `Modlight_Invalid_response`. Put
    /// [`UploadResponse::link`] on the clipboard (`copy_url`, else `url`), `raw_url` is
    /// the thumbnail, `folder` is where it landed and `folder_warning` is a note for the
    /// log, never an error: the file is stored either way.
    pub fn uploaded(&self) -> Option<&UploadResponse> {
        if !self.is_success() {
            return None;
        }
        self.response
            .as_ref()
            .filter(|response| response.url.as_deref().is_some_and(|url| !url.is_empty()))
    }

    /// Which refusal this is, or `None` for a success.
    pub fn refusal(&self) -> Option<UploadRefusal> {
        if self.is_success() {
            return None;
        }
        Some(match self.status {
            400 => UploadRefusal::RequestRejected,
            401 => UploadRefusal::AccountDisabled,
            404 => UploadRefusal::TokenNotRecognized,
            409 => UploadRefusal::AddressTaken,
            413 => UploadRefusal::FileTooLarge,
            507 => UploadRefusal::OutOfSpace,
            other => UploadRefusal::Refused(other),
        })
    }

    /// The server's own words, for the line under the sentence (`ModlightAPI.ServerText`
    /// as `WithServerDetail` uses it): `message`, else `reason`, never "OK", nothing from
    /// a body that is not JSON.
    pub fn server_detail(&self) -> Option<String> {
        server_text(&self.body)
    }

    /// The first value of a header, by name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl Api {
    /// Send one file as a live upload and wait for the server's answer.
    ///
    /// `progress` is called on this task after each chunk has been handed to the
    /// connection, with (file bytes sent so far, file length). Cancelling `cancel` drops
    /// the connection at once, whatever it was doing, and ends with
    /// [`LiveOutcome::Cancelled`].
    pub async fn upload(
        &self,
        credential: &Credential,
        upload: LiveUpload,
        mut progress: impl FnMut(u64, u64) + Send,
        cancel: &CancellationToken,
    ) -> LiveOutcome {
        if cancel.is_cancelled() {
            return LiveOutcome::Cancelled;
        }

        let (reader, length) = match Reader::open(upload.source).await {
            Ok(opened) => opened,
            Err(error) => return LiveOutcome::NoAnswer(error.to_string()),
        };

        let boundary = create_boundary();
        let head = multipart_head(&boundary, upload.folder_id.as_deref(), &upload.file_name);
        let tail = multipart_tail(&boundary);
        let content_length = head.len() as u64 + length + tail.len() as u64;

        let (told, mut sent) = mpsc::unbounded_channel();
        let body = LiveBody {
            head: Some(head),
            reader,
            remaining: length,
            sent: 0,
            untold: false,
            tail: Some(tail),
            buffer_size: upload.buffer_size.max(1),
            told,
            cancel: cancel.clone(),
        };

        let request = self
            .transfer()
            .post(credential.url(UPLOAD_PATH))
            .header(API_KEY_HEADER, credential.token.as_str())
            .header(
                CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(CONTENT_LENGTH, content_length)
            .body(Body::wrap_stream(body.into_stream()));

        let exchange = async move {
            let response = request.send().await?;
            read_answer(response).await
        };
        tokio::pin!(exchange);

        let mut telling = true;
        // The answer is looked at before the cancellation: an upload the server has
        // already stored cannot be un-stored by a click that arrived with its answer.
        let result = loop {
            tokio::select! {
                biased;
                result = &mut exchange => break result,
                _ = cancel.cancelled() => return LiveOutcome::Cancelled,
                moved = sent.recv(), if telling => match moved {
                    Some(bytes) => progress(bytes, length),
                    None => telling = false,
                },
            }
        };
        // Chunks that went out while the answer was arriving are still owed to the caller.
        while let Ok(bytes) = sent.try_recv() {
            progress(bytes, length);
        }

        match result {
            Ok(answer) => {
                if let Some(warning) = answer
                    .uploaded()
                    .and_then(|response| response.folder_warning.as_deref())
                {
                    // Not an error: the file is stored, in the default folder instead.
                    log::info!(
                        "Modlight: {warning}. Stored in the account's default folder instead."
                    );
                }
                LiveOutcome::Answered(Box::new(answer))
            }
            Err(error) => LiveOutcome::NoAnswer(ApiError::from_reqwest(error).to_string()),
        }
    }
}

/// `ProcessWebResponse`: status, where it came from, the headers and the whole body.
async fn read_answer(response: reqwest::Response) -> Result<ServerAnswer, reqwest::Error> {
    let status = response.status();
    let url = response.url().to_string();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let bytes = response.bytes().await?;
    let body = decode_text(&bytes);
    let response = if status.is_success() {
        serde_json::from_str::<UploadResponse>(&body).ok()
    } else {
        None
    };
    Ok(ServerAnswer {
        status: status.as_u16(),
        reason_phrase: status.canonical_reason().unwrap_or_default().to_string(),
        url,
        headers,
        body,
        response,
    })
}

/// A body as text the way .NET reads one with UTF-8: a byte order mark dropped, anything
/// that is not UTF-8 replaced rather than failing the whole answer.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

/// `RequestHelpers.CreateBoundary`: twenty dashes and the local clock in hex ticks.
pub(crate) fn create_boundary() -> String {
    format!(
        "{}{:x}",
        "-".repeat(20),
        dotnet_ticks(chrono::Local::now().naive_local())
    )
}

/// `DateTime.Ticks`: 100-nanosecond intervals since 0001-01-01 00:00 on the same clock.
fn dotnet_ticks(time: NaiveDateTime) -> i64 {
    let epoch = NaiveDate::from_ymd_opt(1, 1, 1)
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .unwrap_or_default();
    let since = time - epoch;
    since.num_seconds() * 10_000_000 + i64::from(since.subsec_nanos() / 100)
}

/// Everything in front of the file's bytes: the form fields (`MakeInputContent` with
/// `isFinal: false`), then the file part's header (`MakeFileInputContentOpen`).
fn multipart_head(boundary: &str, folder_id: Option<&str>, file_name: &str) -> Vec<u8> {
    let mut head = String::new();
    if let Some(folder) = folder_id.filter(|id| !id.is_empty()) {
        head.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{FOLDER_FIELD_NAME}\"\r\n\r\n{folder}\r\n"
        ));
    }
    head.push_str(&format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{FILE_FORM_NAME}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
        quoted_file_name(file_name),
        mime_type_from_file_name(file_name)
    ));
    head.into_bytes()
}

/// `MakeFileInputContentClose`.
fn multipart_tail(boundary: &str) -> Vec<u8> {
    format!("\r\n--{boundary}--\r\n").into_bytes()
}

/// A file name as it can stand between the quotes of `filename="..."` (see the module
/// notes); any name Windows allows comes out unchanged.
fn quoted_file_name(file_name: &str) -> String {
    file_name
        .replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// The source of the file part.
enum Reader {
    File(tokio::fs::File),
    Memory { bytes: Vec<u8>, position: usize },
}

impl Reader {
    /// Open the source and take its length, which is what `Content-Length` promises.
    async fn open(source: UploadSource) -> io::Result<(Reader, u64)> {
        match source {
            UploadSource::File(path) => {
                let file = tokio::fs::File::open(&path).await?;
                let length = file.metadata().await?.len();
                Ok((Reader::File(file), length))
            }
            UploadSource::Bytes(bytes) => {
                let length = bytes.len() as u64;
                Ok((Reader::Memory { bytes, position: 0 }, length))
            }
        }
    }

    /// Up to `want` bytes; empty at the end.
    async fn read(&mut self, want: usize) -> io::Result<Vec<u8>> {
        match self {
            Reader::File(file) => {
                let mut chunk = vec![0; want];
                let read = file.read(&mut chunk).await?;
                chunk.truncate(read);
                Ok(chunk)
            }
            Reader::Memory { bytes, position } => {
                let end = position.saturating_add(want).min(bytes.len());
                let chunk = bytes[*position..end].to_vec();
                *position = end;
                Ok(chunk)
            }
        }
    }
}

/// The request body, produced as the connection asks for it.
struct LiveBody {
    head: Option<Vec<u8>>,
    reader: Reader,
    remaining: u64,
    sent: u64,
    /// A file chunk was handed out and the caller has not been told yet.
    untold: bool,
    tail: Option<Vec<u8>>,
    buffer_size: usize,
    told: mpsc::UnboundedSender<u64>,
    cancel: CancellationToken,
}

impl LiveBody {
    fn into_stream(self) -> impl Stream<Item = io::Result<Vec<u8>>> + Send + 'static {
        futures_util::stream::unfold(Some(self), |body| async move {
            let mut body = body?;
            match body.next_part().await {
                Ok(Some(part)) => Some((Ok(part), Some(body))),
                Ok(None) => None,
                // An error ends the body, and with it the request.
                Err(error) => Some((Err(error), None)),
            }
        })
    }

    async fn next_part(&mut self) -> io::Result<Option<Vec<u8>>> {
        // Being asked for more means the last chunk has been taken by the connection,
        // which is the moment TransferData counts a write as done.
        if self.untold {
            self.untold = false;
            if self.told.send(self.sent).is_err() {
                // Whoever wanted this upload has its answer or gave up on it.
                return Err(io::Error::other("the upload is no longer wanted"));
            }
        }
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("the upload was cancelled"));
        }
        if let Some(head) = self.head.take() {
            return Ok(Some(head));
        }
        if self.remaining > 0 {
            let want = usize::try_from(self.remaining)
                .unwrap_or(usize::MAX)
                .min(self.buffer_size);
            let chunk = self.reader.read(want).await?;
            if chunk.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the file got shorter while it was being sent",
                ));
            }
            self.remaining -= chunk.len() as u64;
            self.sent += chunk.len() as u64;
            self.untold = true;
            return Ok(Some(chunk));
        }
        Ok(self.tail.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boundary_is_twenty_dashes_and_hex_ticks() {
        let boundary = create_boundary();
        let (dashes, hex) = boundary.split_at(20);
        assert_eq!(dashes, "-".repeat(20));
        let ticks = i64::from_str_radix(hex, 16).unwrap();
        let now = dotnet_ticks(chrono::Local::now().naive_local());
        assert!(
            (now - ticks).abs() < 60 * 10_000_000,
            "the local clock, in ticks"
        );
        assert_eq!(hex, hex.to_ascii_lowercase());
    }

    #[test]
    fn ticks_count_from_year_one() {
        let epoch = NaiveDate::from_ymd_opt(1, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(dotnet_ticks(epoch), 0);
        // new DateTime(2026, 10, 19, 12, 0, 0, 250).Ticks
        let noon = NaiveDate::from_ymd_opt(2026, 10, 19)
            .unwrap()
            .and_hms_milli_opt(12, 0, 0, 250)
            .unwrap();
        assert_eq!(dotnet_ticks(noon), 639_280_080_002_500_000);
    }

    #[test]
    fn fields_come_first_then_the_file_then_the_close() {
        let head = multipart_head("--b", Some("f-1"), "shot.png");
        assert_eq!(
            String::from_utf8(head).unwrap(),
            "----b\r\nContent-Disposition: form-data; name=\"folderId\"\r\n\r\nf-1\r\n\
             ----b\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"shot.png\"\r\nContent-Type: image/png\r\n\r\n"
        );
        let bare = multipart_head("--b", None, "notes");
        assert_eq!(
            String::from_utf8(bare).unwrap(),
            "----b\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"notes\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        );
        assert_eq!(
            multipart_head("--b", Some(""), "a.txt"),
            multipart_head("--b", None, "a.txt"),
            "an empty folder is no folder"
        );
        assert_eq!(multipart_tail("--b"), b"\r\n----b--\r\n");
    }

    #[test]
    fn only_names_windows_forbids_are_escaped() {
        assert_eq!(
            quoted_file_name("Zażółć gęślą (1).png"),
            "Zażółć gęślą (1).png"
        );
        assert_eq!(quoted_file_name("a\"b\r\nc.png"), "a%22b%0D%0Ac.png");
    }

    #[test]
    fn refusals_map_to_the_sentences_upstream_picks() {
        let answer = |status| ServerAnswer {
            status,
            reason_phrase: String::new(),
            url: String::new(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: r#"{"message":"No room."}"#.into(),
            response: None,
        };
        assert_eq!(answer(201).refusal(), None);
        assert_eq!(answer(400).refusal(), Some(UploadRefusal::RequestRejected));
        assert_eq!(answer(401).refusal(), Some(UploadRefusal::AccountDisabled));
        assert_eq!(
            answer(404).refusal(),
            Some(UploadRefusal::TokenNotRecognized)
        );
        assert_eq!(answer(409).refusal(), Some(UploadRefusal::AddressTaken));
        assert_eq!(answer(413).refusal(), Some(UploadRefusal::FileTooLarge));
        assert_eq!(answer(507).refusal(), Some(UploadRefusal::OutOfSpace));
        assert_eq!(answer(302).refusal(), Some(UploadRefusal::Refused(302)));
        assert_eq!(answer(500).refusal(), Some(UploadRefusal::Refused(500)));
        assert_eq!(answer(507).server_detail().as_deref(), Some("No room."));
        assert_eq!(answer(507).header("content-type"), Some("application/json"));
        assert_eq!(answer(201).uploaded(), None, "a success without a link");
    }

    #[test]
    fn buffer_sizes_follow_the_setting() {
        assert_eq!(buffer_size_for_power(5), DEFAULT_BUFFER_SIZE);
        assert_eq!(buffer_size_for_power(0), 1024);
        assert_eq!(buffer_size_for_power(13), 8 * 1024 * 1024);
        assert_eq!(buffer_size_for_power(40), 8 * 1024 * 1024);
    }

    #[test]
    fn bodies_read_as_utf8_without_a_bom() {
        assert_eq!(decode_text(b"\xef\xbb\xbf{\"a\":1}"), "{\"a\":1}");
        assert_eq!(decode_text(b"ok \xff"), "ok \u{fffd}");
    }
}
