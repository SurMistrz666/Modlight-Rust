//! `GET /api/me`: who a token belongs to (`ModlightApi.GetAccountAsync`).

use reqwest::{Method, StatusCode};

use crate::client::{Api, Credential, REQUEST_TIMEOUT, parse};
use crate::error::{ApiError, ApiResult};
use crate::models::AccountInfo;

impl Api {
    /// Ask the server who this token belongs to.
    ///
    /// `Ok(None)` when the server answered and refused the token (401/403), or answered
    /// with no account at all (an empty body or `null`, which upstream's
    /// `JsonConvert.DeserializeObject` reads as null and its callers as a token the
    /// server does not recognise). Anything that stopped the request from completing is
    /// an error, because "your token is wrong" and "your server is unreachable" are
    /// different things to tell a person.
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
        if text.trim().is_empty() || text.trim() == "null" {
            return Ok(None);
        }
        parse::<AccountInfo>(&text).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::signin::{SignInFailure, use_token};

    /// A stand-in that answers every request with `status` and `body`.
    async fn answering(status: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        origin
    }

    /// `JsonConvert.DeserializeObject` reads an empty body or `null` as no account, and
    /// upstream's callers treat that as a token the server does not recognise. An empty
    /// account must not be signed in with (a stub or a proxy answering 200 to anything).
    #[tokio::test]
    async fn an_answer_without_an_account_is_not_one() {
        let api = Api::new("0.7.7").unwrap();
        for body in ["", "null", "  null \n"] {
            let origin = answering("200 OK", body).await;
            let credential = Credential::new(origin.as_str(), "token-0123456789");
            assert_eq!(api.get_account(&credential).await, Ok(None), "{body:?}");
            assert_eq!(
                use_token(
                    &api,
                    &origin,
                    "token-0123456789",
                    &tokio_util::sync::CancellationToken::new()
                )
                .await,
                Err(SignInFailure::TokenRejected),
                "{body:?}"
            );
        }

        let origin = answering("200 OK", r#"{"tag":"T","email":"a@b.c"}"#).await;
        let credential = Credential::new(origin.as_str(), "token-0123456789");
        let account = api.get_account(&credential).await.unwrap().unwrap();
        assert_eq!(account.tag.as_deref(), Some("T"));
    }
}
