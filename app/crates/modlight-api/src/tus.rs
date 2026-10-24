//! Resumable uploads over tus 1.0.0 (`ShareX/Modlight/ModlightResumableUpload.cs`): how
//! the offline queue sends a file so that a dropped link costs only the bytes in flight,
//! and how the live upload sends one larger than a chunk (`modlight_core`'s uploader).
//!
//! Every queued item goes this way, small ones included, one session per item: `HEAD`
//! the session for the byte the server got to, or `POST /api/files` for a new one, then
//! the remainder as consecutive `PATCH` requests of at most [`TUS_CHUNK_SIZE`] (50 MiB)
//! each, every one sent from the offset the server confirmed for the one before. The
//! links come back as `X-Media-*` headers on the answer that finished the upload, and
//! are read from that answer only.
//!
//! What has to survive a restart (the session URL, the confirmed offset and the target
//! folder, cleared when the server refused it) lives in [`TusSession`], which the caller
//! owns and persists. [`TusClient::send`] updates it in place as it learns, so however a
//! call ends, saving the session afterwards saves the progress;
//! [`TusClient::send_with_checkpoints`] also hands it to the caller whenever the server
//! has confirmed something worth keeping (a new session, a resumed one, each chunk), so
//! a process that dies mid-file resumes from the last confirmed chunk.
//!
//! Carried over as they are: a client of its own with no overall timeout and no
//! redirects (`Location` and `Upload-Offset` must belong to the request we made); 30
//! seconds for each handshake request, its answer included; a 90-second stall watch on
//! each `PATCH`, looked at every 5 seconds, because a laptop that sleeps mid-PATCH leaves
//! a socket that neither delivers nor fails; `Tus-Resumable: 1.0.0` and `x-api-key` on
//! every request. The three durations can be shortened for tests ([`TusTimings`]).
//!
//! Where this differs from upstream (`ModlightResumableUpload.cs`), and why:
//!
//! * Upstream sends ONE `PATCH` carrying the whole remainder. A server behind
//!   Cloudflare's proxy never sees that request for a file over 100 MB: Cloudflare
//!   refuses any single request body past its plan's limit (100 MB on Free and Pro) with
//!   413, whatever the server would accept, so the upload could never finish however
//!   often it was resumed. Chunks of 50 MiB stay well under every plan's limit; the web
//!   panel's Uppy sends the same size (`TUS_CHUNK_SIZE` of the server), and the server
//!   takes any number of `PATCH`es per upload. The cost is one round trip per 50 MiB.
//! * The session is handed out after each confirmed chunk (see above); upstream saved
//!   the queue only when an attempt ended.
//! * The bytes may come from memory as well as from a file ([`TusSource`]), so the live
//!   upload can send a large capture that was never saved the same way.
//! * A `HEAD` answered by a gateway or a busy server (408, 429, 502, 503, 504: the
//!   offline queue's "never reached the application") keeps the stored session and ends
//!   the attempt with that answer, as a `POST` answered the same way would. Upstream
//!   read every refusal of the `HEAD` as a session the server had forgotten and sent the
//!   whole file again, so a proxy's 502 during a server restart cost a large upload all
//!   its progress.
//! * For the rule that the token goes to the account's host and nowhere else
//!   (`docs/PORT.md`, section 5): upstream sent a STORED session URL as it was stored.
//!   Here only its path and query are kept and put on the account's current origin before
//!   every request, exactly as the `Location` of a new session is treated. A session left
//!   on a host the account no longer uses is then simply not found there (404: a new
//!   session), instead of being sent the token.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, NaiveDateTime, Utc};
use futures_util::Stream;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, LOCATION, RETRY_AFTER};
use reqwest::{Body, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::client::{API_KEY_HEADER, Credential, transfer_client};
use crate::error::{ApiError, ApiResult};
use crate::host::rewrite_to_host;
use crate::mime::mime_type_from_file_name;
use crate::upload::decode_text;

/// The protocol version sent in `Tus-Resumable`.
pub const TUS_VERSION: &str = "1.0.0";

/// Where new sessions are created.
pub const TUS_PATH: &str = "/api/files";

/// Handshake requests are small; one that hangs this long says the link will not carry a
/// file either.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the body may move no bytes before the transfer is treated as dead.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(90);

/// How often the stall watch looks.
pub const STALL_POLL: Duration = Duration::from_secs(5);

/// The most file bytes one `PATCH` carries: 50 MiB.
///
/// Cloudflare's proxy refuses any request whose body is larger than the plan allows
/// (100 MB on Free and Pro, 200 MB on Business, 500 MB on Enterprise) with 413 before the
/// server sees a byte, so one `PATCH` with the whole remainder of a large file could
/// never get through, however often it was resumed. Half the smallest limit leaves room
/// for whatever MB means to a given plan, and it is the size the web panel's Uppy sends
/// (the server's `TUS_CHUNK_SIZE` default). Also the size above which the live upload
/// goes over tus instead of one `POST /upload`.
pub const TUS_CHUNK_SIZE: u64 = 50 * 1024 * 1024;

const TUS_RESUMABLE_HEADER: &str = "Tus-Resumable";
const OFFSET_CONTENT_TYPE: &str = "application/offset+octet-stream";

/// The metadata key the server reads a drop target from.
const FOLDER_METADATA_KEY: &str = "folderId";

/// What .NET's `StreamContent` pulls per read (`Stream.CopyToAsync`'s buffer), which is
/// also how often progress and the stall watch hear about the body.
const READ_SIZE: usize = 81_920;

/// How much of a refusal's text is kept: enough for a row of the queue to hold.
const SERVER_TEXT_LIMIT: usize = 200;

/// How one attempt ended (`ModlightTransferOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferOutcome {
    Success,
    /// Did not work this time. Trying again later is reasonable.
    Retry,
    /// Will not work by repetition. Somebody has to do something.
    Blocked,
    /// The caller asked for it to stop.
    Canceled,
}

/// Why, in the terms of the queue (the `ModlightQueueReason` values this file produces).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TransferReason {
    #[default]
    None,
    /// There is a network, but nothing answered: DNS, TLS, a socket that died, a stall.
    Unreachable,
    /// The server answered, with something that may well not happen next time.
    ServerError,
    /// 401, or a 403 about the token: the server will not take this token.
    TokenRejected,
    /// Refused for a reason that will not change by itself.
    Refused,
    /// 507: no room.
    OutOfSpace,
    /// 413: the server will not take a file this size.
    TooLarge,
}

/// What one attempt came to (`ModlightTransferResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferResult {
    pub outcome: TransferOutcome,
    pub reason: TransferReason,
    /// Whatever the server said, or a number for the sentence. Never a finished sentence.
    pub detail: String,
    /// The link to put in front of the user, chosen by the server from the account's
    /// preference (`X-Media-Copy-Url`, else the viewer page).
    pub copy_url: String,
    pub view_url: String,
    pub raw_url: String,
    /// What the server asked us to wait, when it asked.
    pub retry_after: Option<Duration>,
    /// The status of the answer this came from, or `None` when no answer decided it (a
    /// link that died, a stall, a cancellation). The queue does not need it; the live
    /// upload words its errors with it.
    pub status: Option<u16>,
}

impl TransferResult {
    pub fn failure(
        outcome: TransferOutcome,
        reason: TransferReason,
        detail: impl Into<String>,
    ) -> TransferResult {
        TransferResult {
            outcome,
            reason,
            detail: detail.into(),
            copy_url: String::new(),
            view_url: String::new(),
            raw_url: String::new(),
            retry_after: None,
            status: None,
        }
    }

    /// The same, from an answer with this status.
    fn answered(mut self, status: StatusCode) -> TransferResult {
        self.status = Some(status.as_u16());
        self
    }

    pub fn is_success(&self) -> bool {
        self.outcome == TransferOutcome::Success
    }
}

/// The three durations of a transfer. Upstream fixes them; tests shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TusTimings {
    /// Each `HEAD`, `POST` and `DELETE`, its answer included.
    pub handshake_timeout: Duration,
    /// How long the body may move nothing before the attempt is dropped.
    pub stall_timeout: Duration,
    /// How often the stall watch looks.
    pub stall_poll: Duration,
}

impl Default for TusTimings {
    fn default() -> Self {
        TusTimings {
            handshake_timeout: HANDSHAKE_TIMEOUT,
            stall_timeout: STALL_TIMEOUT,
            stall_poll: STALL_POLL,
        }
    }
}

/// Where the bytes of a transfer are read from.
#[derive(Clone, PartialEq, Eq)]
pub enum TusSource {
    /// A file, read from the offset as each `PATCH` goes.
    File(PathBuf),
    /// Bytes already in memory: a capture that was never saved, a paste.
    Bytes(Arc<Vec<u8>>),
}

impl fmt::Debug for TusSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TusSource::File(path) => f.debug_tuple("File").field(path).finish(),
            // Somebody's screenshot, and useless in a log.
            TusSource::Bytes(bytes) => write!(f, "Bytes({} bytes)", bytes.len()),
        }
    }
}

/// The file one transfer sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TusFile {
    /// Where the bytes are read from.
    pub source: TusSource,
    /// The name it is uploaded under, which need not be the name on disk.
    pub name: String,
    /// The size the caller measured, and checks against the file before every attempt
    /// (a changed file needs a new session: reset the [`TusSession`]). It is what
    /// `Upload-Length` declares and exactly what the body carries.
    pub length: u64,
}

impl TusFile {
    /// A file on disk, `length` bytes long.
    pub fn file(path: impl Into<PathBuf>, name: impl Into<String>, length: u64) -> TusFile {
        TusFile {
            source: TusSource::File(path.into()),
            name: name.into(),
            length,
        }
    }

    /// Bytes in memory, all of them.
    pub fn bytes(name: impl Into<String>, bytes: Arc<Vec<u8>>) -> TusFile {
        TusFile {
            length: bytes.len() as u64,
            source: TusSource::Bytes(bytes),
            name: name.into(),
        }
    }
}

/// What a transfer has to remember between attempts and across restarts.
///
/// Field names on disk are those of `ModlightQueueItem` in `Queue.json`, so the queue
/// can flatten this into its item and keep the Windows file's shape.
///
/// The URL is a capability: after creation tus does not re-authenticate, so whoever
/// holds it can append to (or delete) that one upload for the day the server keeps it.
/// Upstream writes it into the queue file in clear for that reason: it is worth far less
/// than the token next to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TusSession {
    /// The session on the server, once there is one.
    #[serde(rename = "TusUrl")]
    pub url: Option<String>,
    /// The last offset the server confirmed. Advisory: the server is asked again first.
    #[serde(rename = "TusOffset")]
    pub offset: u64,
    /// Where the file should land, or `None` for the account's default folder. Cleared
    /// when the server refused the folder and the upload went to the default instead.
    #[serde(rename = "FolderId")]
    pub folder_id: Option<String>,
}

impl TusSession {
    /// A transfer that has not started, aimed at `folder_id`.
    pub fn new(folder_id: Option<&str>) -> TusSession {
        TusSession {
            folder_id: folder_id.filter(|id| !id.is_empty()).map(str::to_string),
            ..TusSession::default()
        }
    }
}

/// Why an attempt stopped with nothing to classify: the two `catch` blocks of `SendAsync`.
enum Stop {
    /// The caller's token.
    Cancelled,
    /// Anything that kept a request from completing; the text is for the row.
    Failed(String),
}

/// What asking a stored session where it stands came to.
enum Head {
    /// The byte the server got to.
    Offset(u64),
    /// The server does not have it any more (404, 410, any other refusal).
    Gone,
    /// A gateway or a server too busy to answer (408, 429, 502, 503, 504): nothing is
    /// known about the session, so it is kept, and the attempt ends with this answer.
    Unanswered(TransferResult),
}

/// Where the session stands before the body goes.
enum Ensured {
    /// Nothing to send: report this (a refusal, or an empty file already stored).
    Report(TransferResult),
    /// Send the body to this URL, on the account's origin.
    Ready(String),
}

/// One answer to a handshake request, read whole.
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

/// Sends files over tus. Cheap to clone; one is enough for every account.
#[derive(Clone)]
pub struct TusClient {
    http: reqwest::Client,
    timings: TusTimings,
    chunk_size: u64,
}

impl TusClient {
    /// A client identifying itself as `Modlight/<version>`, with upstream's durations
    /// and `PATCH`es of [`TUS_CHUNK_SIZE`].
    pub fn new(version: &str) -> ApiResult<TusClient> {
        TusClient::with_timings(version, TusTimings::default())
    }

    /// The same with other durations, for tests that cannot wait 90 seconds for a stall.
    pub fn with_timings(version: &str, timings: TusTimings) -> ApiResult<TusClient> {
        Ok(TusClient {
            http: transfer_client(version)?,
            timings,
            chunk_size: TUS_CHUNK_SIZE,
        })
    }

    /// The same client sending at most `bytes` per `PATCH` (at least one), for tests
    /// that cannot send several 50 MiB chunks.
    pub fn with_chunk_size(mut self, bytes: u64) -> TusClient {
        self.chunk_size = bytes.max(1);
        self
    }

    pub fn timings(&self) -> TusTimings {
        self.timings
    }

    /// The most file bytes one `PATCH` carries.
    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }

    /// Send one file as far as it will go this time (`SendAsync`).
    ///
    /// `session` is read for where to resume and written as things are learned: the URL
    /// of a new session, the offset the server confirms after each chunk, the folder
    /// dropped after a refusal, and all of it cleared once the file is stored. `progress`
    /// is called on this task with (bytes sent so far, file length), starting from the
    /// resume point and never going back within one call.
    ///
    /// Never fails: everything that stops a request from completing (DNS, TLS, a reset, a
    /// stall, a handshake deadline) is [`TransferOutcome::Retry`] with
    /// [`TransferReason::Unreachable`], because none of it says whether the upload would
    /// work in ten minutes.
    pub async fn send(
        &self,
        credential: &Credential,
        file: &TusFile,
        session: &mut TusSession,
        progress: impl FnMut(u64, u64) + Send,
        cancel: &CancellationToken,
    ) -> TransferResult {
        self.send_with_checkpoints(credential, file, session, progress, |_| {}, cancel)
            .await
    }

    /// [`send`](Self::send), calling `checkpoint` with the session each time the server
    /// has confirmed something the next attempt should start from: the session made or
    /// found (its URL and offset), then the offset after each chunk but the last. Saving
    /// it there is what lets a process that dies mid-file resume at the last confirmed
    /// chunk instead of asking the server about a session it never wrote down.
    pub async fn send_with_checkpoints(
        &self,
        credential: &Credential,
        file: &TusFile,
        session: &mut TusSession,
        mut progress: impl FnMut(u64, u64) + Send,
        mut checkpoint: impl FnMut(&TusSession) + Send,
        cancel: &CancellationToken,
    ) -> TransferResult {
        let attempt = async {
            let before = session.clone();
            match self
                .ensure_session(credential, file, session, cancel)
                .await?
            {
                Ensured::Report(result) => Ok(result),
                Ensured::Ready(url) => {
                    if *session != before {
                        checkpoint(session);
                    }
                    self.send_body(
                        credential,
                        file,
                        session,
                        &url,
                        &mut progress,
                        &mut checkpoint,
                        cancel,
                    )
                    .await
                }
            }
        };
        match attempt.await {
            Ok(result) => result,
            Err(Stop::Cancelled) => {
                TransferResult::failure(TransferOutcome::Canceled, TransferReason::None, "")
            }
            Err(Stop::Failed(detail)) => {
                log::warn!("Modlight queue transfer: {detail}");
                TransferResult::failure(TransferOutcome::Retry, TransferReason::Unreachable, detail)
            }
        }
    }

    /// Tell the server to forget a half-finished upload (`AbandonAsync`).
    ///
    /// Not needed for correctness, since the server reclaims untouched sessions within a
    /// day, but a cancelled four-gigabyte upload should not squat on somebody's scratch
    /// space until tomorrow when one request settles it. Returns the status the server
    /// answered, or `None` when `url` is not a URL and nothing was sent. The request goes
    /// to the account's origin whatever host `url` names (see the module notes).
    pub async fn abandon(&self, credential: &Credential, url: &str) -> ApiResult<Option<u16>> {
        let Some(target) = rewrite_to_host(url, &credential.host) else {
            return Ok(None);
        };
        let request = decorate(self.http.delete(target), credential);
        let exchange = async {
            let response = request.send().await?;
            let status = response.status();
            // Read the (empty) answer so the connection can be used again.
            response.bytes().await?;
            Ok::<_, reqwest::Error>(status)
        };
        let status = match tokio::time::timeout(self.timings.handshake_timeout, exchange).await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => return Err(ApiError::from_reqwest(error)),
            Err(_) => return Err(ApiError::Timeout),
        };
        log::info!(
            "Modlight queue: abandoned a resumable session, {}.",
            status.as_u16()
        );
        Ok(Some(status.as_u16()))
    }

    /// A session with a known offset, created if there is none (`EnsureSessionAsync`).
    async fn ensure_session(
        &self,
        credential: &Credential,
        file: &TusFile,
        session: &mut TusSession,
        cancel: &CancellationToken,
    ) -> Result<Ensured, Stop> {
        if let Some(stored) = session.url.clone().filter(|url| !url.is_empty()) {
            if let Some(url) = rewrite_to_host(&stored, &credential.host) {
                match self.head_offset(credential, &url, cancel).await? {
                    Head::Offset(offset) if offset <= file.length => {
                        session.offset = offset;
                        return Ok(Ensured::Ready(url));
                    }
                    // Not an answer about the session: keep it for the next attempt
                    // rather than send the whole file again (upstream started over).
                    Head::Unanswered(result) => return Ok(Ensured::Report(result)),
                    Head::Offset(_) | Head::Gone => {}
                }
            }
            // 404 or 410: the server forgets sessions after a day. Not a failure and not
            // worth reporting; the file is still here and the only cost is the bytes
            // already sent. A new session, from zero.
            log::info!(
                "Modlight queue: the resumable session is gone; starting the transfer again."
            );
            session.url = None;
            session.offset = 0;
        }
        self.create_session(credential, file, session, cancel).await
    }

    /// Where the server got to (`HeadOffsetAsync`).
    async fn head_offset(
        &self,
        credential: &Credential,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<Head, Stop> {
        let answer = self
            .handshake(decorate(self.http.head(url), credential), cancel)
            .await?;
        let status = answer.status.as_u16();
        // The answers that mean the request never reached the application (the offline
        // queue's own list) say nothing about the session.
        if crate::is_worth_queuing(Some(status)) {
            let retry_after = first_header(&answer.headers, RETRY_AFTER.as_str());
            return Ok(Head::Unanswered(classify(
                status,
                retry_after.as_deref(),
                &answer.body,
            )));
        }
        // 404, 410 and every other refusal alike: no session worth resuming.
        if !answer.status.is_success() {
            return Ok(Head::Gone);
        }
        Ok(first_header(&answer.headers, "upload-offset")
            .as_deref()
            .and_then(parse_offset)
            .map_or(Head::Gone, Head::Offset))
    }

    /// `CreateSessionAsync`: one creation, and one more without the folder when the
    /// folder was what the server refused.
    async fn create_session(
        &self,
        credential: &Credential,
        file: &TusFile,
        session: &mut TusSession,
        cancel: &CancellationToken,
    ) -> Result<Ensured, Stop> {
        let folder = session.folder_id.clone().filter(|id| !id.is_empty());
        let first = self
            .create_once(credential, file, session, folder.as_deref(), cancel)
            .await?;
        match first {
            Ensured::Report(failure)
                if failure.reason == TransferReason::Refused && folder.is_some() =>
            {
                // The folder is the only thing in the request that can have stopped
                // belonging to this account since the item was queued, and the live
                // upload treats exactly this as a warning: stored in the default folder.
                // Refusing here would strand a queued upload for ever over a folder the
                // user deleted last week.
                log::info!(
                    "Modlight queue: the target folder was refused; sending to the account's default folder instead."
                );
                session.folder_id = None;
                self.create_once(credential, file, session, None, cancel)
                    .await
            }
            other => Ok(other),
        }
    }

    /// `CreateOnceAsync`: `POST /api/files` with the length and the metadata.
    async fn create_once(
        &self,
        credential: &Credential,
        file: &TusFile,
        session: &mut TusSession,
        folder_id: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Ensured, Stop> {
        let request = decorate(self.http.post(credential.url(TUS_PATH)), credential)
            .header("Upload-Length", file.length.to_string())
            .header("Upload-Metadata", metadata(&file.name, folder_id))
            // An empty body with its length stated: without it some proxies answer 411
            // before the request reaches the application at all.
            .header(CONTENT_LENGTH, "0")
            .body(Vec::new());
        let answer = self.handshake(request, cancel).await?;

        // An empty file is finished the moment it is created: the server runs its finish
        // hook there and answers 200 with the links and, because the status is no longer
        // 201, deliberately without a Location. Reading that as a refusal would create a
        // new session on every attempt for two weeks. (The reference server stores the
        // file and then answers 404 instead, see tests/uploads.rs; that 404 is read as a
        // refusal here exactly as upstream reads it.)
        if answer.status == StatusCode::OK && file.length == 0 {
            session.url = None;
            session.offset = 0;
            return Ok(Ensured::Report(landed(answer.status, &answer.headers)));
        }

        if answer.status != StatusCode::CREATED {
            let retry_after = first_header(&answer.headers, RETRY_AFTER.as_str());
            return Ok(Ensured::Report(classify(
                answer.status.as_u16(),
                retry_after.as_deref(),
                &answer.body,
            )));
        }

        let Some(location) =
            first_header(&answer.headers, LOCATION.as_str()).filter(|value| !value.is_empty())
        else {
            return Ok(Ensured::Report(
                TransferResult::failure(
                    TransferOutcome::Retry,
                    TransferReason::ServerError,
                    "201 without Location",
                )
                .answered(answer.status),
            ));
        };

        let url = resolve_location(&credential.host, &location);
        session.url = Some(url.clone());
        session.offset = 0;
        Ok(Ensured::Ready(url))
    }

    /// `SendBodyAsync`, in chunks: consecutive `PATCH`es of at most
    /// [`chunk_size`](Self::chunk_size) bytes from the offset to the end, each sent from
    /// the offset the server confirmed for the one before.
    #[allow(clippy::too_many_arguments)]
    async fn send_body(
        &self,
        credential: &Credential,
        file: &TusFile,
        session: &mut TusSession,
        url: &str,
        progress: &mut (impl FnMut(u64, u64) + Send),
        checkpoint: &mut (impl FnMut(&TusSession) + Send),
        cancel: &CancellationToken,
    ) -> Result<TransferResult, Stop> {
        if session.offset > file.length {
            // The server holds more bytes than the file has: the file changed under a
            // live session. Abandon it and let the next attempt build a new one.
            session.url = None;
            session.offset = 0;
            return Ok(TransferResult::failure(
                TransferOutcome::Retry,
                TransferReason::ServerError,
                "offset past the end of the file",
            ));
        }

        progress(session.offset, file.length);

        loop {
            let offset = session.offset;
            // Nothing left still takes one (empty) PATCH: a session that holds every
            // byte but never finished is finished by it.
            let size = (file.length - offset).min(self.chunk_size);
            let Some(response) = self
                .patch_chunk(credential, file, url, offset, size, progress, cancel)
                .await?
            else {
                // Stalled. Whatever the server did take is still on its disk, and the
                // next HEAD will say how much.
                log::info!(
                    "Modlight queue: the transfer stopped moving, so it was dropped and will be resumed."
                );
                return Ok(TransferResult::failure(
                    TransferOutcome::Retry,
                    TransferReason::Unreachable,
                    "",
                ));
            };

            let status = response.status();
            if matches!(
                status,
                StatusCode::CONFLICT | StatusCode::NOT_FOUND | StatusCode::GONE
            ) {
                // 409 is tus for "that is not where I am". Never a reason to give up: the
                // next attempt asks again and starts from wherever the server says.
                session.offset = 0;
                return Ok(TransferResult::failure(
                    TransferOutcome::Retry,
                    TransferReason::ServerError,
                    status.as_u16().to_string(),
                )
                .answered(status));
            }

            if !status.is_success() {
                let retry_after = first_header(response.headers(), RETRY_AFTER.as_str());
                // Upstream read this with no deadline at all; a cancel at least ends it.
                let body = tokio::select! {
                    biased;
                    body = response.bytes() => body.map(|bytes| decode_text(&bytes)).unwrap_or_default(),
                    _ = cancel.cancelled() => return Err(Stop::Cancelled),
                };
                return Ok(classify(status.as_u16(), retry_after.as_deref(), &body));
            }

            let confirmed = first_header(response.headers(), "upload-offset")
                .as_deref()
                .and_then(parse_offset);
            if let Some(confirmed) = confirmed {
                session.offset = confirmed;
            }

            // 200 rather than 204 is how the server says its finish hook ran; the links
            // come back on that answer and nowhere else.
            if status != StatusCode::OK && session.offset < file.length {
                if confirmed == Some(offset + size) {
                    // The whole chunk is on the server's disk: worth surviving a crash.
                    checkpoint(session);
                    continue;
                }
                // Short of what was sent (or not said at all): the link went. Keep what
                // the server confirmed; the next attempt asks it again.
                return Ok(TransferResult::failure(
                    TransferOutcome::Retry,
                    TransferReason::Unreachable,
                    "",
                )
                .answered(status));
            }

            session.url = None;
            session.offset = file.length;
            return Ok(landed(status, response.headers()));
        }
    }

    /// One `PATCH` of `size` bytes from `offset`, under its own stall watch: the answer,
    /// or `None` when the body stopped moving for the stall timeout.
    #[allow(clippy::too_many_arguments)]
    async fn patch_chunk(
        &self,
        credential: &Credential,
        file: &TusFile,
        url: &str,
        offset: u64,
        size: u64,
        progress: &mut (impl FnMut(u64, u64) + Send),
        cancel: &CancellationToken,
    ) -> Result<Option<reqwest::Response>, Stop> {
        if cancel.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        let source = Reader::open(&file.source, offset)
            .await
            .map_err(|error| Stop::Failed(error.to_string()))?;

        let started = Instant::now();
        let activity = Arc::new(AtomicU64::new(0));
        let (told, mut moved) = mpsc::unbounded_channel();
        let body = PatchBody {
            source,
            remaining: size,
            sent: offset,
            started,
            activity: Arc::clone(&activity),
            told,
            cancel: cancel.clone(),
        };

        let request = decorate(self.http.patch(url), credential)
            .header("Upload-Offset", offset.to_string())
            .header(CONTENT_TYPE, OFFSET_CONTENT_TYPE)
            // Stated, so the body is never sent chunked (see the live upload).
            .header(CONTENT_LENGTH, size)
            .body(Body::wrap_stream(body.into_stream()));

        let poll = self.timings.stall_poll.max(Duration::from_millis(1));
        let mut watch = tokio::time::interval_at(started + poll, poll);
        watch.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let stall = self.timings.stall_timeout;

        let exchange = request.send();
        tokio::pin!(exchange);
        let mut telling = true;
        // The watch covers everything up to the answer's headers, the wait after the last
        // byte included, as upstream's linked token did. An answer that is there is taken
        // before a cancellation or a stall: it may be the one that says the file is stored.
        let response = loop {
            tokio::select! {
                biased;
                result = &mut exchange => {
                    break result.map_err(|error| {
                        Stop::Failed(ApiError::from_reqwest(error).to_string())
                    })?;
                }
                _ = cancel.cancelled() => return Err(Stop::Cancelled),
                _ = watch.tick() => {
                    let quiet = started
                        .elapsed()
                        .saturating_sub(Duration::from_millis(activity.load(Ordering::Relaxed)));
                    if quiet > stall {
                        return Ok(None);
                    }
                }
                bytes = moved.recv(), if telling => match bytes {
                    Some(bytes) => progress(bytes, file.length),
                    None => telling = false,
                },
            }
        };
        while let Ok(bytes) = moved.try_recv() {
            progress(bytes, file.length);
        }
        Ok(Some(response))
    }

    /// One small request under the handshake deadline, its answer read within the same
    /// deadline (what `HttpClient.SendAsync` does by default).
    async fn handshake(
        &self,
        request: RequestBuilder,
        cancel: &CancellationToken,
    ) -> Result<Answer, Stop> {
        let exchange = async {
            let response = request.send().await?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.bytes().await?;
            Ok::<_, reqwest::Error>(Answer {
                status,
                headers,
                body: decode_text(&body),
            })
        };
        // A complete answer wins over a cancellation that came with it: a new session the
        // server made must not be forgotten.
        tokio::select! {
            biased;
            result = tokio::time::timeout(self.timings.handshake_timeout, exchange) => match result {
                Ok(Ok(answer)) => Ok(answer),
                Ok(Err(error)) => Err(Stop::Failed(ApiError::from_reqwest(error).to_string())),
                Err(_) => Err(Stop::Failed(ApiError::Timeout.to_string())),
            },
            _ = cancel.cancelled() => Err(Stop::Cancelled),
        }
    }
}

/// `Decorate`: the protocol version and the credential, on every request.
fn decorate(request: RequestBuilder, credential: &Credential) -> RequestBuilder {
    request
        .header(TUS_RESUMABLE_HEADER, TUS_VERSION)
        .header(API_KEY_HEADER, credential.token.as_str())
}

/// The upload is stored: take the links the finish hook put on the answer (`Landed`).
///
/// A stored upload with no link in the answer is stopped rather than retried: trying
/// again would send the whole file a second time and leave the account holding it twice.
fn landed(status: StatusCode, headers: &HeaderMap) -> TransferResult {
    let copy = first_header(headers, "x-media-copy-url").unwrap_or_default();
    let view = first_header(headers, "x-media-url").unwrap_or_default();
    let raw = first_header(headers, "x-media-raw-url").unwrap_or_default();
    if copy.is_empty() && view.is_empty() {
        return TransferResult::failure(
            TransferOutcome::Blocked,
            TransferReason::ServerError,
            "no link in the response",
        )
        .answered(status);
    }
    TransferResult {
        copy_url: if copy.is_empty() { view.clone() } else { copy },
        view_url: view,
        raw_url: raw,
        ..TransferResult::failure(TransferOutcome::Success, TransferReason::None, "")
            .answered(status)
    }
}

/// What a refusal means for the queue (`ClassifyAsync`), from the status, the
/// `Retry-After` header and the body as received.
///
/// The split that matters is whether repeating the request could ever get a different
/// answer. A 500 could; a 507 could not, because the account is full and nothing the
/// client does changes that. Retrying a permanent refusal for a week is the bug this
/// exists to avoid.
pub fn classify(status: u16, retry_after: Option<&str>, body: &str) -> TransferResult {
    let detail = server_text(body);
    let blocked =
        |reason| TransferResult::failure(TransferOutcome::Blocked, reason, detail.clone());
    let result = match status {
        401 => blocked(TransferReason::TokenRejected),
        403 if is_auth_refusal(&detail) => blocked(TransferReason::TokenRejected),
        // Left for the caller to read as "the folder": every other 403 this endpoint
        // produces says so in words, which is what the check above reads.
        403 => blocked(TransferReason::Refused),
        507 => blocked(TransferReason::OutOfSpace),
        // Creation compares the declared length with the smaller of the operator's
        // ceiling and what is left of the quota, so this arrives for both; the queue
        // asks about the quota first, which is what lets this mean "too large".
        413 => blocked(TransferReason::TooLarge),
        400 | 404 | 405 | 409 | 411 | 412 | 415 => blocked(TransferReason::Refused),
        _ => TransferResult {
            retry_after: retry_after.and_then(|value| parse_retry_after(value, Utc::now())),
            ..TransferResult::failure(
                TransferOutcome::Retry,
                TransferReason::ServerError,
                status.to_string(),
            )
        },
    };
    TransferResult {
        status: Some(status),
        ..result
    }
}

/// The server's own words about a refusal, clipped to what a row can hold
/// (`ServerTextAsync`): trimmed, nothing from an HTML page (a proxy's error page holds
/// nothing a person wants), at most 200 UTF-16 units as .NET counts them, without
/// splitting a character.
pub fn server_text(body: &str) -> String {
    let text = body.trim();
    if text.is_empty() || text.starts_with('<') {
        return String::new();
    }
    let mut units = 0;
    for (index, character) in text.char_indices() {
        units += character.len_utf16();
        if units > SERVER_TEXT_LIMIT {
            return text[..index].to_string();
        }
    }
    text.to_string()
}

/// Whether a 403 is about the credential rather than the folder (`IsAuthRefusal`).
///
/// Both come out of creation with the same code; only the words tell them apart. When
/// they say nothing recognisable the folder retry is the safe guess: it runs once and
/// then reports whatever comes back.
fn is_auth_refusal(detail: &str) -> bool {
    let words = detail.to_ascii_lowercase();
    words.contains("scope") || words.contains("token")
}

/// `Retry-After` as delta seconds or an HTTP date; a date in the past means now.
fn parse_retry_after(value: &str, now: DateTime<Utc>) -> Option<Duration> {
    let value = value.trim();
    if value.starts_with(|c: char| c.is_ascii_digit()) {
        // .NET reads the delta as an Int32 and rejects anything else in the value.
        let seconds: u32 = value.parse().ok()?;
        return (seconds <= i32::MAX as u32).then(|| Duration::from_secs(u64::from(seconds)));
    }
    let date = DateTime::parse_from_rfc2822(value)
        .map(|date| date.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            // RFC 850 and asctime, the two older forms an HTTP date may take.
            ["%A, %d-%b-%y %H:%M:%S GMT", "%a %b %e %H:%M:%S %Y"]
                .iter()
                .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
                .map(|naive| naive.and_utc())
        })?;
    Some((date - now).to_std().unwrap_or(Duration::ZERO))
}

/// `long.TryParse` on an offset header: surrounding blanks and a plus sign are allowed.
/// A negative offset is no offset (the server never sends one).
fn parse_offset(value: &str) -> Option<u64> {
    let value = value.trim();
    value.strip_prefix('+').unwrap_or(value).parse().ok()
}

/// `Upload-Metadata`: the name, the type its name maps to, and the folder when one is
/// named, each value base64 of its UTF-8.
fn metadata(file_name: &str, folder_id: Option<&str>) -> String {
    let mut pairs = vec![
        ("filename", file_name),
        ("filetype", mime_type_from_file_name(file_name)),
    ];
    if let Some(folder) = folder_id.filter(|id| !id.is_empty()) {
        pairs.push((FOLDER_METADATA_KEY, folder));
    }
    pairs
        .iter()
        .map(|(key, value)| format!("{key} {}", BASE64.encode(value.as_bytes())))
        .collect::<Vec<_>>()
        .join(",")
}

/// Turn the server's `Location` into a URL this client will send the token to
/// (`ResolveLocation`): its path and query on the account's origin, whatever scheme and
/// host it named. The server builds that header from the `Host` and `X-Forwarded-Proto`
/// it was given, so behind a proxy that forwards neither it honestly answers `http://`,
/// and following that would put the token on the wire in clear.
fn resolve_location(origin: &str, location: &str) -> String {
    let absolute = Url::parse(location).ok().or_else(|| {
        // "//host/path" is absolute to .NET on Windows (a UNC name); its path is used too.
        location
            .starts_with("//")
            .then(|| Url::parse(&format!("http:{location}")).ok())
            .flatten()
    });
    match absolute {
        Some(url) => {
            let mut path = url.path().to_string();
            if !path.starts_with('/') {
                path.insert(0, '/');
            }
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            format!("{origin}{path}")
        }
        None if location.starts_with('/') => format!("{origin}{location}"),
        None => format!("{origin}/{location}"),
    }
}

/// The first value of a header, as text.
fn first_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
}

/// The bytes of one `PATCH`, read from the file or from memory.
enum Reader {
    File(tokio::fs::File),
    Bytes { bytes: Arc<Vec<u8>>, at: usize },
}

impl Reader {
    /// The source, positioned at `offset`.
    async fn open(source: &TusSource, offset: u64) -> io::Result<Reader> {
        match source {
            TusSource::File(path) => {
                let mut file = tokio::fs::File::open(path).await?;
                file.seek(io::SeekFrom::Start(offset)).await?;
                Ok(Reader::File(file))
            }
            TusSource::Bytes(bytes) => Ok(Reader::Bytes {
                bytes: Arc::clone(bytes),
                at: usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len()),
            }),
        }
    }

    async fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Reader::File(file) => file.read(buffer).await,
            Reader::Bytes { bytes, at } => {
                let count = buffer.len().min(bytes.len() - *at);
                buffer[..count].copy_from_slice(&bytes[*at..*at + count]);
                *at += count;
                Ok(count)
            }
        }
    }
}

/// One `PATCH` body: `remaining` bytes of the source from the offset, saying how far it
/// has got (`ProgressReportingStream`).
struct PatchBody {
    source: Reader,
    remaining: u64,
    /// Offset plus what has been read since.
    sent: u64,
    started: Instant,
    /// Milliseconds after `started` of the last read: what the stall watch looks at.
    activity: Arc<AtomicU64>,
    told: mpsc::UnboundedSender<u64>,
    cancel: CancellationToken,
}

impl PatchBody {
    fn into_stream(self) -> impl Stream<Item = io::Result<Vec<u8>>> + Send + 'static {
        futures_util::stream::unfold(Some(self), |body| async move {
            let mut body = body?;
            match body.next_chunk().await {
                Ok(Some(chunk)) => Some((Ok(chunk), Some(body))),
                Ok(None) => None,
                Err(error) => Some((Err(error), None)),
            }
        })
    }

    async fn next_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("the transfer was cancelled"));
        }
        let want = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(READ_SIZE);
        let mut chunk = vec![0; want];
        let read = self.source.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the file got shorter while it was being sent",
            ));
        }
        chunk.truncate(read);
        self.remaining -= read as u64;
        self.sent += read as u64;
        // Being read from is what counts as moving, as in upstream's stream.
        let since = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.activity.store(since, Ordering::Relaxed);
        if self.told.send(self.sent).is_err() {
            // The attempt is over (an answer, a stall, a cancel); stop reading the file.
            return Err(io::Error::other("the transfer is no longer wanted"));
        }
        Ok(Some(chunk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(result: &TransferResult) -> (TransferOutcome, TransferReason, &str) {
        (result.outcome, result.reason, result.detail.as_str())
    }

    #[test]
    fn refusals_are_split_by_whether_repeating_could_help() {
        use TransferOutcome::{Blocked, Retry};
        use TransferReason::{OutOfSpace, Refused, ServerError, TokenRejected, TooLarge};

        assert_eq!(
            outcome(&classify(401, None, "Sign in or send a valid API token.\n")),
            (Blocked, TokenRejected, "Sign in or send a valid API token.")
        );
        assert_eq!(
            outcome(&classify(403, None, "This API token is scoped to \"read\"")),
            (
                Blocked,
                TokenRejected,
                "This API token is scoped to \"read\""
            )
        );
        assert_eq!(
            outcome(&classify(403, None, "Insufficient SCOPE")).1,
            TokenRejected
        );
        assert_eq!(
            outcome(&classify(403, None, "Folder not found.")),
            (Blocked, Refused, "Folder not found.")
        );
        assert_eq!(outcome(&classify(403, None, "")).1, Refused);
        assert_eq!(
            outcome(&classify(507, None, "Quota exceeded.")),
            (Blocked, OutOfSpace, "Quota exceeded.")
        );
        assert_eq!(
            outcome(&classify(413, None, "Maximum size exceeded\n")),
            (Blocked, TooLarge, "Maximum size exceeded")
        );
        for status in [400, 404, 405, 409, 411, 412, 415] {
            assert_eq!(
                outcome(&classify(status, None, "x")),
                (Blocked, Refused, "x"),
                "{status}"
            );
        }
        for status in [429, 500, 502, 503, 504, 302, 418] {
            let result = classify(status, Some("12"), "<html>busy</html>");
            assert_eq!(
                outcome(&result),
                (Retry, ServerError, status.to_string().as_str())
            );
            assert_eq!(result.retry_after, Some(Duration::from_secs(12)));
        }
        assert_eq!(classify(507, Some("12"), "").retry_after, None);
        assert_eq!(classify(500, None, "").retry_after, None);
    }

    #[test]
    fn server_text_is_trimmed_html_free_and_clipped() {
        assert_eq!(
            server_text("  Upload-Offset conflict\n"),
            "Upload-Offset conflict"
        );
        assert_eq!(server_text("<html><body>502</body></html>"), "");
        assert_eq!(server_text("   "), "");
        let long = "a".repeat(300);
        assert_eq!(server_text(&long).len(), 200);
        // 199 units, then a character worth two: it does not fit and is not split.
        let astral = format!("{}{}", "b".repeat(199), "\u{1F600}tail");
        assert_eq!(server_text(&astral), "b".repeat(199));
        assert_eq!(server_text(&"ż".repeat(250)).chars().count(), 200);
    }

    #[test]
    fn retry_after_takes_seconds_or_a_date() {
        let now = DateTime::parse_from_rfc3339("1994-11-06T08:49:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after(" 0 ", now), Some(Duration::ZERO));
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after("Sunday, 06-Nov-94 08:49:37 GMT", now),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after("Sun Nov  6 08:49:37 1994", now),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:00:00 GMT", now),
            Some(Duration::ZERO),
            "a date in the past is now"
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(parse_retry_after("12s", now), None);
        assert_eq!(parse_retry_after("-5", now), None);
        assert_eq!(parse_retry_after("99999999999", now), None);
    }

    #[test]
    fn offsets_parse_like_long_try_parse() {
        assert_eq!(parse_offset("1048576"), Some(1_048_576));
        assert_eq!(parse_offset(" +7 "), Some(7));
        assert_eq!(parse_offset("-1"), None);
        assert_eq!(parse_offset("12abc"), None);
        assert_eq!(parse_offset(""), None);
    }

    #[test]
    fn metadata_carries_name_type_and_folder() {
        assert_eq!(
            metadata("Zrzut ekranu.png", Some("f-1")),
            format!(
                "filename {},filetype {},folderId {}",
                BASE64.encode("Zrzut ekranu.png"),
                BASE64.encode("image/png"),
                BASE64.encode("f-1")
            )
        );
        assert_eq!(
            metadata("clip", None),
            format!(
                "filename {},filetype {}",
                BASE64.encode("clip"),
                BASE64.encode("application/octet-stream")
            )
        );
        assert_eq!(metadata("a.txt", Some("")), metadata("a.txt", None));
    }

    #[test]
    fn locations_are_put_on_the_accounts_origin() {
        let origin = "https://files.example.com";
        assert_eq!(
            resolve_location(origin, "http://internal:3000/api/files/abc?x=1#frag"),
            "https://files.example.com/api/files/abc?x=1"
        );
        assert_eq!(
            resolve_location(origin, "/api/files/abc"),
            "https://files.example.com/api/files/abc"
        );
        assert_eq!(
            resolve_location(origin, "api/files/abc"),
            "https://files.example.com/api/files/abc"
        );
        assert_eq!(
            resolve_location(origin, "//evil.example/api/files/abc"),
            "https://files.example.com/api/files/abc"
        );
        assert_eq!(
            resolve_location(origin, "urn:files:abc"),
            "https://files.example.com/files:abc"
        );
    }

    #[test]
    fn the_session_keeps_the_queue_files_field_names() {
        let session = TusSession {
            url: Some("http://127.0.0.1:8420/api/files/abc".into()),
            offset: 42,
            folder_id: None,
        };
        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "TusUrl": "http://127.0.0.1:8420/api/files/abc",
                "TusOffset": 42,
                "FolderId": null
            })
        );
        assert_eq!(serde_json::from_value::<TusSession>(json).unwrap(), session);
        assert_eq!(
            serde_json::from_str::<TusSession>("{}").unwrap(),
            TusSession::default()
        );
        assert_eq!(TusSession::new(Some("")).folder_id, None);
    }

    #[test]
    fn links_come_from_the_finish_hooks_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-media-url", "https://h/u/a".parse().unwrap());
        headers.insert("x-media-raw-url", "https://h/u/a.png".parse().unwrap());
        let result = landed(StatusCode::OK, &headers);
        assert!(result.is_success());
        assert_eq!(result.status, Some(200));
        assert_eq!(result.copy_url, "https://h/u/a", "no copy link: the page");
        assert_eq!(result.raw_url, "https://h/u/a.png");

        headers.insert("x-media-copy-url", "https://h/u/a.png".parse().unwrap());
        assert_eq!(
            landed(StatusCode::OK, &headers).copy_url,
            "https://h/u/a.png"
        );

        let nothing = landed(StatusCode::NO_CONTENT, &HeaderMap::new());
        assert_eq!(nothing.status, Some(204));
        assert_eq!(
            outcome(&nothing),
            (
                TransferOutcome::Blocked,
                TransferReason::ServerError,
                "no link in the response"
            )
        );
    }
}
