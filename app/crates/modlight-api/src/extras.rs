//! Duplicates, the skin, client settings, the administration report, the URL shortener
//! and the plain upload: the rest of the endpoints the Windows client uses.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::client::{Api, Credential, REQUEST_TIMEOUT, parse};
use crate::error::{ApiError, ApiResult, RefusalBody};
use crate::models::{
    CloudDocument, DiskReport, DuplicateDecision, DuplicateGroup, DuplicateScan, ShortLink,
    ShortLinkOptions, ThemeInfo, UploadResponse,
};
use crate::upload::{LiveOutcome, LiveUpload, UploadSource};

/// A scan groups the whole account in SQL, so it may take longer than a listing.
pub const DUPLICATE_SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// The skin endpoint's own budget (`ModlightTheme.requestTimeout`).
pub const THEME_TIMEOUT: Duration = Duration::from_secs(15);

/// The administration report's budget, quoted by its screen.
pub const DISK_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the administration report lives.
pub const DISK_PATH: &str = "/api/admin/disk";

/// The client discriminator the settings endpoint stores this row under.
pub const SETTINGS_CLIENT: &str = "desktop";

/// How a call to `/api/client-settings` ended.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsAnswer {
    /// The server answered (2xx, or 409 with the winning document in `conflict`).
    Document(CloudDocument),
    /// 401/403 or 413: an answer that will not change until somebody signs in again.
    Refused(u16),
    /// Any other status: nothing was learned, keep what we have.
    Failed(u16),
}

/// How a `POST /upload` ended, with what the uploader needs to explain a refusal.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadAnswer {
    pub status: u16,
    pub body: String,
    pub response: Option<UploadResponse>,
}

impl Api {
    /// Every set of byte-for-byte identical files on this account.
    pub async fn get_duplicates(&self, credential: &Credential) -> ApiResult<DuplicateScan> {
        let text = self
            .get_string("/api/duplicates", credential, DUPLICATE_SCAN_TIMEOUT)
            .await?;
        parse(&text)
    }

    /// Say that sets' copies are meant to exist. Returns how many sets were kept.
    pub async fn keep_duplicate_sets(
        &self,
        credential: &Credential,
        sets: &[DuplicateGroup],
    ) -> ApiResult<i64> {
        let keep: Vec<serde_json::Value> = sets
            .iter()
            .filter_map(|set| {
                set.checksum.as_deref().filter(|c| !c.is_empty()).map(|checksum| {
                    json!({ "checksum": checksum, "copies": set.copies, "bytes": set.bytes })
                })
            })
            .collect();
        if keep.is_empty() {
            return Ok(0);
        }
        let text = self
            .send_json(
                Method::POST,
                "/api/duplicates",
                credential,
                &json!({ "keep": keep }),
                REQUEST_TIMEOUT,
            )
            .await?;
        Ok(parse::<DuplicateDecision>(&text)?.kept)
    }

    /// Start counting these sets again. Returns how many were restored.
    pub async fn count_duplicate_sets_again(
        &self,
        credential: &Credential,
        checksums: &[String],
    ) -> ApiResult<i64> {
        if checksums.is_empty() {
            return Ok(0);
        }
        let text = self
            .send_json(
                Method::POST,
                "/api/duplicates",
                credential,
                &json!({ "countAgain": checksums }),
                REQUEST_TIMEOUT,
            )
            .await?;
        Ok(parse::<DuplicateDecision>(&text)?.restored)
    }

    /// The skin the account holds, or `None` if the server would not say.
    pub async fn get_theme(&self, credential: &Credential) -> ApiResult<Option<String>> {
        let response = self
            .request(Method::GET, &credential.url("/api/theme"), credential)
            .timeout(THEME_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        theme_of(response).await
    }

    /// Tell the server which skin the account now holds; returns what it ended up with.
    pub async fn set_theme(
        &self,
        credential: &Credential,
        skin: &str,
    ) -> ApiResult<Option<String>> {
        let response = self
            .request(Method::PUT, &credential.url("/api/theme"), credential)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )
            .body(json!({ "theme": skin }).to_string())
            .timeout(THEME_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        theme_of(response).await
    }

    /// One call to `/api/client-settings?client=desktop`.
    ///
    /// `body` is `None` for the read. A write always states the version it replaces:
    /// the endpoint reads a missing version as "take this, whatever was there".
    pub async fn client_settings(
        &self,
        credential: &Credential,
        body: Option<(&serde_json::Value, i64)>,
    ) -> ApiResult<SettingsAnswer> {
        let url = credential.url(&format!("/api/client-settings?client={SETTINGS_CLIENT}"));
        let mut request = self
            .request(
                if body.is_some() {
                    Method::PUT
                } else {
                    Method::GET
                },
                &url,
                credential,
            )
            .timeout(REQUEST_TIMEOUT);
        if let Some((settings, expected_version)) = body {
            let envelope = json!({ "version": expected_version, "settings": settings });
            request = request
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )
                .body(envelope.to_string());
        }
        let response = request.send().await.map_err(ApiError::from_reqwest)?;
        let status = response.status();

        if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || status == StatusCode::PAYLOAD_TOO_LARGE
        {
            return Ok(SettingsAnswer::Refused(status.as_u16()));
        }
        let conflict = status == StatusCode::CONFLICT;
        if !status.is_success() && !conflict {
            return Ok(SettingsAnswer::Failed(status.as_u16()));
        }

        let text = response.text().await.map_err(ApiError::from_reqwest)?;
        let answer: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(SettingsAnswer::Document(CloudDocument {
            version: answer.get("version").and_then(|v| v.as_i64()).unwrap_or(0),
            settings: answer
                .get("settings")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default(),
            limit_bytes: answer
                .get("limitBytes")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            conflict,
        }))
    }

    /// What the server's storage is holding (administrators only).
    ///
    /// 401 is [`ApiError::Unauthorized`], 403 [`ApiError::AdminForbidden`] (a wider token
    /// is the remedy), 404 [`ApiError::NotSupported`] (either no such endpoint or not an
    /// administrator; the caller settles which by asking `/api/me` again).
    pub async fn get_disk_report(&self, credential: &Credential) -> ApiResult<DiskReport> {
        let response = self
            .request(Method::GET, &credential.url(DISK_PATH), credential)
            .timeout(DISK_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(ApiError::Unauthorized),
            StatusCode::FORBIDDEN => return Err(ApiError::AdminForbidden),
            StatusCode::NOT_FOUND => return Err(ApiError::NotSupported),
            status if !status.is_success() => return Err(ApiError::Status(status.as_u16())),
            _ => {}
        }
        let text = response.text().await.map_err(ApiError::from_reqwest)?;
        parse(&text)
    }

    /// Shorten `target` in the signed-in account's own namespace.
    ///
    /// A refusal comes back as [`ApiError::Refused`] with the server's `reason` token,
    /// which the shortener's caller turns into a sentence (see `short_link_message` in
    /// modlight-core).
    pub async fn shorten(
        &self,
        credential: &Credential,
        target: &str,
        options: Option<&ShortLinkOptions>,
    ) -> ApiResult<ShortLink> {
        let mut body = serde_json::Map::new();
        body.insert("target".into(), json!(target));
        if let Some(options) = options {
            if let Some(code) = options.custom_code.as_deref().filter(|c| !c.is_empty()) {
                body.insert("customCode".into(), json!(code));
            }
            if let Some(days) = options.expires_in_days {
                body.insert("expiresInDays".into(), json!(days));
            }
            if let Some(password) = options.password.as_deref().filter(|p| !p.is_empty()) {
                body.insert("password".into(), json!(password));
            }
            // Sent only when it is being turned OFF; absent means the server's default.
            if !options.use_user_code {
                body.insert("useUserCode".into(), json!(false));
            }
        }
        let response = self
            .request(Method::POST, &credential.url("/api/shortlinks"), credential)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::Value::Object(body).to_string())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = response.status();
        let text = response.text().await.map_err(ApiError::from_reqwest)?;
        if !status.is_success() {
            let refusal: RefusalBody = serde_json::from_str(&text).unwrap_or_default();
            return Err(ApiError::Refused {
                status: status.as_u16(),
                detail: refusal.message.unwrap_or_default(),
                reason: refusal.reason,
            });
        }
        let link: ShortLink = parse(&text)?;
        if link.url.as_deref().is_none_or(str::is_empty) {
            return Err(ApiError::Decode("the short link has no url".into()));
        }
        Ok(link)
    }

    /// One plain `POST /upload` of bytes in memory: the live upload of [`crate::upload`]
    /// with the default buffer, no progress and nothing to cancel it. No timeout: the
    /// server keeps this endpoint uncapped for header-authenticated clients.
    pub async fn upload_bytes(
        &self,
        credential: &Credential,
        file_name: &str,
        bytes: Vec<u8>,
        folder_id: Option<&str>,
    ) -> ApiResult<UploadAnswer> {
        let upload = LiveUpload::new(file_name, UploadSource::Bytes(bytes)).with_folder(folder_id);
        match self
            .upload(credential, upload, |_, _| {}, &CancellationToken::new())
            .await
        {
            LiveOutcome::Answered(answer) => {
                let answer = *answer;
                Ok(UploadAnswer {
                    status: answer.status,
                    body: answer.body,
                    response: answer.response,
                })
            }
            LiveOutcome::NoAnswer(text) => Err(ApiError::Transport(text)),
            LiveOutcome::Cancelled => Err(ApiError::Cancelled),
        }
    }

    /// `GET /api/health`, the server's own liveness probe; true when it answers 2xx.
    pub async fn health(&self, host: &str) -> bool {
        match self
            .http()
            .get(format!("{}/api/health", host.trim_end_matches('/')))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }
}

/// A failed or unreadable theme answer reads as "the server would not say".
async fn theme_of(response: reqwest::Response) -> ApiResult<Option<String>> {
    if !response.status().is_success() {
        return Ok(None);
    }
    let text = response.text().await.map_err(ApiError::from_reqwest)?;
    Ok(serde_json::from_str::<ThemeInfo>(&text)
        .ok()
        .and_then(|info| info.theme))
}

/// Whether an upload that failed this way is worth keeping and sending later
/// (`ModlightUploadQueue.IsWorthQueuing`): no answer at all, or 408/429/502/503/504.
/// Everything else, 507 above all, is reported now rather than retried.
pub fn is_worth_queuing(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(408 | 429 | 502 | 503 | 504) => true,
        Some(_) => false,
    }
}

/// The server's own explanation for a refusal (`ModlightAPI.ServerText`): `message`,
/// else `reason`; "OK" is not an explanation; a body that is not JSON gives nothing.
pub fn server_text(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let text = parsed
        .get("message")
        .and_then(|v| v.as_str())
        .or_else(|| parsed.get("reason").and_then(|v| v.as_str()))?;
    if text == "OK" {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_failures_are_queued() {
        assert!(is_worth_queuing(None));
        for status in [408, 429, 502, 503, 504] {
            assert!(is_worth_queuing(Some(status)));
        }
        for status in [400, 401, 404, 409, 413, 500, 507] {
            assert!(!is_worth_queuing(Some(status)));
        }
    }

    #[test]
    fn server_text_prefers_the_message() {
        assert_eq!(
            server_text(r#"{"message":"No room","reason":"x"}"#).as_deref(),
            Some("No room")
        );
        assert_eq!(
            server_text(r#"{"reason":"code-taken"}"#).as_deref(),
            Some("code-taken")
        );
        assert_eq!(server_text(r#"{"message":"OK"}"#), None);
        assert_eq!(server_text("<html>502</html>"), None);
    }
}
