//! The uploaders a task calls (`UploadData` with the Modlight file, image and text
//! services, `ShortenURL` with the Modlight shortener, `ShareURL` with the sharing
//! services): the request, and the errors upstream records for each way it can end.
//!
//! The error texts keep upstream's shape: one sentence a person can act on first
//! (`ReportRefusal`), then the dump `ProcessError` writes (message, request URL, status,
//! headers, response text). The stack trace section has no counterpart and is left out.
//!
//! One difference from upstream, which sends every live upload as one `POST /upload`: a
//! file larger than one tus chunk ([`modlight_api::TUS_CHUNK_SIZE`], 50 MiB) goes over
//! tus instead, in `PATCH`es of that size, as the offline queue sends. Behind
//! Cloudflare's proxy a single request body over 100 MB is refused with 413 before the
//! server sees it, so the one `POST` could never carry such a file. The session such an
//! upload leaves behind is kept by the task for its next attempt, and handed to the queue
//! with the file.

use std::path::Path;

use modlight_api::tus::{
    TUS_PATH, TransferOutcome as TusOutcome, TransferReason, TransferResult as TusResult, TusFile,
    TusSession,
};
use modlight_api::upload::{LiveOutcome, LiveUpload, ServerAnswer, UploadRefusal, UploadSource};
use modlight_api::{ApiError, Credential};
use tokio_util::sync::CancellationToken;

use crate::l10n::{Arg, format, s, t, uploaders};
use crate::tasks::context::TaskContext;
use crate::tasks::info::{DataType, ResponseInfo, TaskInfo, UploadResult, UploaderErrors};
use crate::tasks::naming;
use crate::tasks::settings::{TaskSettings, UrlSharingServices};
use crate::tasks::worker::TaskData;
use crate::upload::progress::ProgressManager;
use crate::upload::queue::LiveSession;

/// How one upload attempt ended, for `DoUpload`.
#[derive(Debug, Default)]
pub struct UploadOutcome {
    /// What the uploader returned; `None` when it was stopped (`TransferData` gave up).
    pub result: Option<UploadResult>,
    /// What the uploader recorded (`uploader.Errors`).
    pub errors: UploaderErrors,
    /// The server's status, or `None` when nothing answered (`LastAnswer`).
    pub answer: Option<u16>,
    pub stopped: bool,
    /// The folder the server says the file landed in, when this upload reports it:
    /// `Some(None)` is "no folder".
    pub landed_folder: Option<Option<String>>,
}

/// Which service carries this upload, and what it was built with
/// (`ModlightImageUploaderService` / `ModlightTextUploaderService` / the file one).
struct Service {
    /// The folder the uploader is aimed at; the text service names none.
    folder: Option<String>,
    /// Whether the landed folder travels back (only the file service keeps the task).
    reports_landing: bool,
}

fn service_for(info: &TaskInfo) -> Service {
    match info.upload_destination() {
        DataType::Image => Service {
            folder: info.modlight_folder_id.clone(),
            reports_landing: false,
        },
        DataType::Text => Service {
            folder: None,
            reports_landing: false,
        },
        _ => Service {
            folder: info.modlight_folder_id.clone(),
            reports_landing: true,
        },
    }
}

/// `GetInvalidConfigResult`: no account, so nothing was sent; the sign-in window opens.
fn invalid_config(ctx: &TaskContext) -> UploadResult {
    let mut result = UploadResult::default();
    let message = format(
        t(s::WorkerTask_GetInvalidConfigResult__0__configuration_is_invalid_or_missing__Please_check__Destination_settings__window_to_configure_it_),
        &[Arg::from("Modlight")],
    );
    log::info!("{message}");
    result.errors.add(&message);
    ctx.channel
        .effect(crate::tasks::host::TaskEffect::ShowSignIn);
    result
}

/// `Uploader.ProcessError`'s text for an attempt that got no answer.
fn transport_error(error: &str, url: &str) -> String {
    format!(
        "{}\n{error}\n\n{}\n{url}",
        t(uploaders::Uploader_Error_message),
        t(uploaders::Uploader_Request_URL)
    )
}

/// `Uploader.ProcessError`'s text for a refusal (a `WebException` with a response).
fn refusal_error(answer: &ServerAnswer, url: &str) -> String {
    let mut text = format!(
        "{}\nThe remote server returned an error: ({}) {}.\n\n{}\n{url}\n\n{}:\n({}) {}",
        t(uploaders::Uploader_Error_message),
        answer.status,
        answer.reason_phrase,
        t(uploaders::Uploader_Request_URL),
        t(uploaders::ResponseWindow_Status_code),
        answer.status,
        answer.reason_phrase,
    );
    if !answer.url.is_empty() && answer.url != url {
        text.push_str(&format!(
            "\n\n{}:\n{}",
            t(uploaders::ResponseWindow_Response_URL),
            answer.url
        ));
    }
    if !answer.headers.is_empty() {
        let headers: Vec<String> = answer
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        text.push_str(&format!(
            "\n\n{}:\n{}",
            t(uploaders::ResponseWindow_Headers),
            headers.join("\n")
        ));
    }
    text.push_str(&format!(
        "\n\n{}:\n{}",
        t(uploaders::ResponseWindow_Response_text),
        answer.body
    ));
    text
}

/// `Modlight.ReportRefusal`: the sentence for the status, with the server's own words.
fn refusal_sentence(answer: &ServerAnswer) -> String {
    let message = match answer.refusal() {
        Some(UploadRefusal::RequestRejected) => t(uploaders::Modlight_Request_rejected).to_string(),
        Some(UploadRefusal::AccountDisabled) => t(uploaders::Modlight_Account_disabled).to_string(),
        Some(UploadRefusal::TokenNotRecognized) => {
            t(uploaders::Modlight_Token_not_recognized).to_string()
        }
        Some(UploadRefusal::AddressTaken) => t(uploaders::Modlight_Address_taken).to_string(),
        Some(UploadRefusal::FileTooLarge) => t(uploaders::Modlight_File_too_large).to_string(),
        Some(UploadRefusal::OutOfSpace) => t(uploaders::Modlight_Out_of_space).to_string(),
        Some(UploadRefusal::Refused(_)) | None => format(
            t(uploaders::Modlight_Server_refused),
            &[Arg::from(status_of(answer))],
        ),
    };
    with_server_detail(&message, answer.server_detail())
}

fn status_of(answer: &ServerAnswer) -> i64 {
    answer.status as i64
}

/// `ModlightAPI.WithServerDetail`.
fn with_server_detail(message: &str, detail: Option<String>) -> String {
    match detail.filter(|d| !d.is_empty()) {
        Some(detail) => format!("{message}\n{detail}"),
        None => message.to_string(),
    }
}

fn response_info(answer: &ServerAnswer) -> ResponseInfo {
    ResponseInfo {
        status: answer.status,
        status_text: answer.reason_phrase.clone(),
        url: answer.url.clone(),
        headers: answer.headers.clone(),
        body: answer.body.clone(),
    }
}

/// One upload through the Modlight uploader (`UploadData` + `Modlight.Upload`).
///
/// A file larger than one tus chunk goes over tus (see the module notes); `live` is the
/// session such an upload of this task left on an earlier attempt, resumed here, and is
/// written back for the next attempt (or `None` when there is nothing to resume).
pub async fn upload(
    ctx: &TaskContext,
    info: &TaskInfo,
    data: Option<TaskData>,
    cancel: CancellationToken,
    mut on_progress: Box<dyn FnMut(ProgressManager) + Send>,
    live: &mut Option<LiveSession>,
) -> UploadOutcome {
    let Some(credential) = ctx.credential() else {
        return UploadOutcome {
            result: Some(invalid_config(ctx)),
            ..UploadOutcome::default()
        };
    };
    let service = service_for(info);
    let mut errors = UploaderErrors {
        default_title: format(t(s::WorkerTask_ErrorTitle), &[Arg::from("Modlight")]),
        ..UploaderErrors::default()
    };
    let file_name = naming::upload_name(&info.file_name, &info.settings);
    let length = data.as_ref().map_or(0, TaskData::len);
    if let Some(data) = data.as_ref()
        && length > ctx.tus.chunk_size()
    {
        let file = match data {
            TaskData::File(path) => TusFile::file(path.clone(), &file_name, length),
            TaskData::Bytes(bytes) => TusFile::bytes(&file_name, bytes.clone()),
        };
        return upload_resumable(
            ctx,
            &credential,
            &service,
            file,
            errors,
            cancel,
            on_progress,
            live,
        )
        .await;
    }
    let source = match data {
        Some(TaskData::File(path)) => UploadSource::File(path),
        Some(TaskData::Bytes(bytes)) => UploadSource::Bytes(bytes.as_ref().clone()),
        None => {
            errors.add(&transport_error(
                "There is nothing to upload.",
                &credential.url("/upload"),
            ));
            return UploadOutcome {
                result: Some(UploadResult::default()),
                errors,
                ..UploadOutcome::default()
            };
        }
    };
    let buffer = ctx.app_settings().buffer_size();
    let upload = LiveUpload::new(&file_name, source)
        .with_folder(service.folder.as_deref())
        .with_buffer_size(buffer);
    let url = credential.url("/upload");
    let mut measured: Option<ProgressManager> = None;
    let outcome = ctx
        .api
        .upload(
            &credential,
            upload,
            |sent, total| {
                let progress =
                    measured.get_or_insert_with(|| ProgressManager::new(total as i64, 0));
                let moved = sent as i64 - progress.position();
                if moved > 0 && progress.update_progress(moved) {
                    on_progress(progress.clone());
                }
            },
            &cancel,
        )
        .await;
    match outcome {
        LiveOutcome::Cancelled => UploadOutcome {
            result: None,
            errors,
            stopped: true,
            ..UploadOutcome::default()
        },
        LiveOutcome::NoAnswer(error) => {
            errors.add(&transport_error(&error, &url));
            UploadOutcome {
                result: Some(UploadResult::default()),
                errors,
                answer: None,
                ..UploadOutcome::default()
            }
        }
        LiveOutcome::Answered(answer) => {
            let mut result = UploadResult {
                response: answer.body.clone(),
                response_info: answer.is_success().then(|| response_info(&answer)),
                ..UploadResult::default()
            };
            let status = Some(answer.status);
            if !answer.is_success() {
                // `SendRequestFile` threw: `ProcessError` wrote the dump, the response
                // body is not the result, and `ReportRefusal` puts its sentence first.
                result.response = String::new();
                errors.add(&refusal_error(&answer, &url));
                errors.add_first(&refusal_sentence(&answer));
                return UploadOutcome {
                    result: Some(result),
                    errors,
                    answer: status,
                    ..UploadOutcome::default()
                };
            }
            result.set_success(true);
            let Some(response) = answer.uploaded() else {
                errors.add(t(uploaders::Modlight_Invalid_response));
                return UploadOutcome {
                    result: Some(result),
                    errors,
                    answer: status,
                    ..UploadOutcome::default()
                };
            };
            // `copy_url`, not `url`: the account's own preference of what to hand out.
            result.url = response.link().unwrap_or_default().to_string();
            result.thumbnail_url = response.raw_url.clone().unwrap_or_default();
            result.deletion_url = response.deletion_url.clone().unwrap_or_default();
            if let Some(warning) = response.folder_warning.as_deref().filter(|w| !w.is_empty()) {
                log::info!("Modlight: {warning}. Stored in the account's default folder instead.");
            }
            let landed = service
                .reports_landing
                .then(|| response.folder.clone().filter(|f| !f.is_empty()));
            UploadOutcome {
                result: Some(result),
                errors,
                answer: status,
                stopped: false,
                landed_folder: landed,
            }
        }
    }
}

/// A live upload larger than one tus chunk, over tus (see the module notes).
#[allow(clippy::too_many_arguments)]
async fn upload_resumable(
    ctx: &TaskContext,
    credential: &Credential,
    service: &Service,
    file: TusFile,
    mut errors: UploaderErrors,
    cancel: CancellationToken,
    mut on_progress: Box<dyn FnMut(ProgressManager) + Send>,
    live: &mut Option<LiveSession>,
) -> UploadOutcome {
    let url = credential.url(TUS_PATH);
    let mut session = match live.take() {
        Some(kept) if kept.length == file.length && kept.session.url.is_some() => kept.session,
        other => {
            // A session for other bytes (the file changed between attempts) is no use.
            if let Some(stale) = other.and_then(|kept| kept.session.url) {
                abandon_session(ctx, credential.clone(), stale);
            }
            TusSession::new(service.folder.as_deref())
        }
    };
    let mut measured: Option<ProgressManager> = None;
    let result = ctx
        .tus
        .send(
            credential,
            &file,
            &mut session,
            |sent, total| {
                // Counted from where this attempt starts, so a resumed upload does not
                // report the bytes already on the server as speed.
                let progress =
                    measured.get_or_insert_with(|| ProgressManager::new(total as i64, sent as i64));
                let moved = sent as i64 - progress.position();
                if moved > 0 && progress.update_progress(moved) {
                    on_progress(progress.clone());
                }
            },
            &cancel,
        )
        .await;
    if session.url.is_some() {
        *live = Some(LiveSession {
            length: file.length,
            session: session.clone(),
        });
    }

    let failed = |errors: UploaderErrors, answer: Option<u16>| UploadOutcome {
        result: Some(UploadResult::default()),
        errors,
        answer,
        ..UploadOutcome::default()
    };
    match result.outcome {
        TusOutcome::Canceled => UploadOutcome {
            result: None,
            errors,
            stopped: true,
            ..UploadOutcome::default()
        },
        TusOutcome::Success => {
            let mut upload = UploadResult {
                url: result.copy_url.clone(),
                thumbnail_url: result.raw_url.clone(),
                ..UploadResult::default()
            };
            upload.set_success(true);
            UploadOutcome {
                result: Some(upload),
                errors,
                answer: result.status,
                stopped: false,
                // The folder the session was made with: the one asked for, or none when
                // the server refused it and the file went to the default folder.
                landed_folder: service.reports_landing.then(|| session.folder_id.clone()),
            }
        }
        TusOutcome::Retry | TusOutcome::Blocked => match conclusive_status(&result) {
            None => {
                errors.add(&transport_error(&no_answer_text(&result), &url));
                failed(errors, None)
            }
            Some(status) if (200..300).contains(&status) => {
                // Stored, but the answer named no link: `Modlight_Invalid_response`.
                let mut upload = UploadResult::default();
                upload.set_success(true);
                errors.add(t(uploaders::Modlight_Invalid_response));
                UploadOutcome {
                    result: Some(upload),
                    errors,
                    answer: Some(status),
                    ..UploadOutcome::default()
                }
            }
            Some(status) => {
                let words = if result.outcome == TusOutcome::Blocked {
                    result.detail.clone()
                } else {
                    String::new()
                };
                let answer = ServerAnswer {
                    status,
                    reason_phrase: reqwest::StatusCode::from_u16(status)
                        .ok()
                        .and_then(|code| code.canonical_reason())
                        .unwrap_or_default()
                        .to_string(),
                    url: url.clone(),
                    headers: Vec::new(),
                    body: words.clone(),
                    response: None,
                };
                errors.add(&refusal_error(&answer, &url));
                errors.add_first(&resumable_refusal_sentence(&result, status, words));
                failed(errors, Some(status))
            }
        },
    }
}

/// The status a tus attempt is judged by, as `LastAnswer` for the live upload: `None`
/// when nothing conclusive answered. That is a link that died or stalled, and the three
/// answers about the session rather than the file (404, 409, 410 to a `PATCH`), after
/// which tus asks the server again where it stands: the task's next attempt or the queue
/// carries on from there.
fn conclusive_status(result: &TusResult) -> Option<u16> {
    if result.reason == TransferReason::Unreachable {
        return None;
    }
    match result.status {
        Some(404 | 409 | 410) if result.outcome == TusOutcome::Retry => None,
        status => status,
    }
}

/// The text of a tus attempt that got no conclusive answer.
fn no_answer_text(result: &TusResult) -> String {
    match result.status {
        Some(status) if result.reason != TransferReason::Unreachable => {
            format!("The server answered {status} for the upload session.")
        }
        _ if !result.detail.is_empty() => result.detail.clone(),
        _ => "The transfer stopped before the server confirmed the file.".to_string(),
    }
}

/// `ReportRefusal`'s sentence for a refusal of a tus upload, from what the refusal means
/// rather than the bare status (404 or 409 mean something else to `POST /upload`).
fn resumable_refusal_sentence(result: &TusResult, status: u16, words: String) -> String {
    let message = match result.reason {
        TransferReason::TooLarge => t(uploaders::Modlight_File_too_large).to_string(),
        TransferReason::OutOfSpace => t(uploaders::Modlight_Out_of_space).to_string(),
        TransferReason::TokenRejected if status == 403 => {
            t(uploaders::Modlight_Token_not_allowed).to_string()
        }
        TransferReason::TokenRejected => t(uploaders::Modlight_Token_not_recognized).to_string(),
        _ => format(
            t(uploaders::Modlight_Server_refused),
            &[Arg::from(status as i64)],
        ),
    };
    with_server_detail(&message, Some(words))
}

/// Tell the server to forget a session nobody will resume, in the background: it would
/// otherwise hold the bytes until it reclaims them a day later.
pub fn abandon_session(ctx: &TaskContext, credential: Credential, url: String) {
    let tus = ctx.tus.clone();
    ctx.runtime.spawn(async move {
        if let Err(error) = tus.abandon(&credential, &url).await {
            log::warn!("Modlight: a resumable session could not be abandoned: {error}");
        }
    });
}

/// `ShortenURL` through the Modlight shortener: a result with the short link, or the
/// errors the shortener recorded.
pub async fn shorten(ctx: &TaskContext, settings: &TaskSettings, url: &str) -> UploadResult {
    let Some(credential) = ctx.credential() else {
        return invalid_config(ctx);
    };
    let mut result = UploadResult {
        url: url.to_string(),
        ..UploadResult::default()
    };
    let request_url = credential.url("/api/shortlinks");
    match ctx
        .api
        .shorten(&credential, url, settings.modlight_short_link.as_ref())
        .await
    {
        Ok(link) => result.shortened_url = link.url.unwrap_or_default(),
        Err(ApiError::Decode(_)) => result.errors.add(t(uploaders::Modlight_Invalid_response)),
        Err(ApiError::Refused {
            status,
            detail,
            reason,
        }) => {
            result.errors.add(&format!(
                "{}\nThe remote server returned an error: ({status}).\n\n{}\n{request_url}",
                t(uploaders::Uploader_Error_message),
                t(uploaders::Uploader_Request_URL)
            ));
            let message = reason
                .as_deref()
                .and_then(short_link_reason)
                .unwrap_or_else(|| short_link_status(status));
            let detail = if detail.is_empty() {
                reason.clone()
            } else {
                Some(detail)
            };
            result
                .errors
                .add_first(&with_server_detail(&message, detail));
        }
        Err(error) => result
            .errors
            .add(&transport_error(&error.to_string(), &request_url)),
    }
    result
}

/// `ModlightURLShortener.MessageForReason`.
fn short_link_reason(reason: &str) -> Option<String> {
    let key = match reason {
        "invalid-url" => uploaders::Modlight_Link_invalid_url,
        "invalid-code" => uploaders::Modlight_Link_invalid_code,
        "code-taken" => uploaders::Modlight_Address_taken,
        "code-too-long" => uploaders::Modlight_Link_code_too_long,
        "root-not-allowed" => uploaders::Modlight_Link_root_not_allowed,
        "no-tag" => uploaders::Modlight_Link_no_tag,
        "rate-limited" => uploaders::Modlight_Too_many_requests,
        _ => return None,
    };
    Some(t(key).to_string())
}

/// `ModlightURLShortener.MessageForStatus`.
fn short_link_status(status: u16) -> String {
    match status {
        401 => t(uploaders::Modlight_Token_not_recognized).to_string(),
        403 => t(uploaders::Modlight_Token_not_allowed).to_string(),
        409 => t(uploaders::Modlight_Address_taken).to_string(),
        429 => t(uploaders::Modlight_Too_many_requests).to_string(),
        other => format(
            t(uploaders::Modlight_Server_refused),
            &[Arg::from(other as i64)],
        ),
    }
}

/// The page a sharing service opens for `url` (`SimpleURLSharer`).
pub fn share_url(service: UrlSharingServices, url: &str) -> String {
    let encoded = naming::url_encode(url);
    match service {
        UrlSharingServices::GoogleImageSearch => {
            format!("https://lens.google.com/uploadbyurl?url={encoded}")
        }
        UrlSharingServices::BingVisualSearch => {
            format!("https://www.bing.com/images/search?view=detailv2&iss=sbi&q=imgurl:{encoded}")
        }
    }
}

/// The sharing service's localized name.
pub fn sharing_service_name(service: UrlSharingServices) -> String {
    t(match service {
        UrlSharingServices::GoogleImageSearch => uploaders::URLSharingServices_GoogleImageSearch,
        UrlSharingServices::BingVisualSearch => uploaders::URLSharingServices_BingVisualSearch,
    })
    .to_string()
}

/// Whether a path is worth trying to upload at all (exists and is a file).
pub fn is_uploadable(path: &Path) -> bool {
    path.is_file()
}
