//! The uploaders a task calls (`UploadData` with the Modlight file, image and text
//! services, `ShortenURL` with the Modlight shortener, `ShareURL` with the sharing
//! services): the request, and the errors upstream records for each way it can end.
//!
//! The error texts keep upstream's shape: one sentence a person can act on first
//! (`ReportRefusal`), then the dump `ProcessError` writes (message, request URL, status,
//! headers, response text). The stack trace section has no counterpart and is left out.

use std::path::Path;

use modlight_api::ApiError;
use modlight_api::upload::{LiveOutcome, LiveUpload, ServerAnswer, UploadRefusal, UploadSource};
use tokio_util::sync::CancellationToken;

use crate::l10n::{Arg, format, s, t, uploaders};
use crate::tasks::context::TaskContext;
use crate::tasks::info::{DataType, ResponseInfo, TaskInfo, UploadResult, UploaderErrors};
use crate::tasks::naming;
use crate::tasks::settings::{TaskSettings, UrlSharingServices};
use crate::tasks::worker::TaskData;
use crate::upload::progress::ProgressManager;

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
pub async fn upload(
    ctx: &TaskContext,
    info: &TaskInfo,
    data: Option<TaskData>,
    cancel: CancellationToken,
    mut on_progress: Box<dyn FnMut(ProgressManager) + Send>,
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
