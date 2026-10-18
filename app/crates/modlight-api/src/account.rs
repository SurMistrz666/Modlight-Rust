//! `GET /api/me`: who a token belongs to (`ModlightApi.GetAccountAsync`).

use reqwest::{Method, StatusCode};

use crate::client::{Api, Credential, REQUEST_TIMEOUT, parse};
use crate::error::{ApiError, ApiResult};
use crate::models::AccountInfo;

impl Api {
    /// Ask the server who this token belongs to.
    ///
    /// `Ok(None)` when the server answered and refused the token (401/403). Anything
    /// that stopped the request from completing is an error, because "your token is
    /// wrong" and "your server is unreachable" are different things to tell a person.
    pub async fn get_account(&self, credential: &Credential) -> ApiResult<Option<AccountInfo>> {
        let response = self
            .request(Method::GET, &credential.url("/api/me"), credential)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;

        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16()));
        }

        let text = response.text().await.map_err(ApiError::from_reqwest)?;
        parse::<AccountInfo>(&text).map(Some)
    }
}
