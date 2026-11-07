//! Not upstream: the account's "Remove metadata from files on upload" switch,
//! `GET` and `PUT /api/strip-metadata` (the server's `docs/CONTRACT.md` section 11). The
//! server removes the metadata; the app only reads and sets the account's choice.

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Response, StatusCode};

use crate::client::{Api, Credential, REQUEST_TIMEOUT};
use crate::error::{ApiError, ApiResult, RefusalBody};

/// Where the switch lives.
pub const STRIP_METADATA_PATH: &str = "/api/strip-metadata";

#[derive(serde::Deserialize)]
struct Switch {
    enabled: bool,
}

impl Api {
    /// The switch as the account holds it.
    pub async fn get_strip_metadata(&self, credential: &Credential) -> ApiResult<bool> {
        let response = self
            .request(
                Method::GET,
                &credential.url(STRIP_METADATA_PATH),
                credential,
            )
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        switch_of(response).await
    }

    /// Turn the switch on or off; returns what the server stored. Files already
    /// uploaded are not touched.
    pub async fn set_strip_metadata(
        &self,
        credential: &Credential,
        enabled: bool,
    ) -> ApiResult<bool> {
        let response = self
            .request(
                Method::PUT,
                &credential.url(STRIP_METADATA_PATH),
                credential,
            )
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .body(serde_json::json!({ "enabled": enabled }).to_string())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        switch_of(response).await
    }
}

/// 401 is the token the server no longer takes; 403 a token too narrow for this
/// (a refusal, not a reason to sign in again); 404 a server without the switch.
async fn switch_of(response: Response) -> ApiResult<bool> {
    let status = response.status();
    let text = response.text().await.map_err(ApiError::from_reqwest)?;
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::NotSupported);
    }
    if !status.is_success() {
        let body: RefusalBody = serde_json::from_str(&text).unwrap_or_default();
        return Err(ApiError::Refused {
            status: status.as_u16(),
            detail: body.message.unwrap_or_default(),
            reason: body.reason,
        });
    }
    serde_json::from_str::<Switch>(&text)
        .map(|s| s.enabled)
        .map_err(|e| ApiError::Decode(e.to_string()))
}
