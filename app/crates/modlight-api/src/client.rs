//! The one HTTP client every call goes through, and the credential it carries.
//!
//! Two rules from the security audit live here so that no call can forget them:
//!
//! * the token travels ONLY in the `x-api-key` header, never in a URL;
//! * redirects are never followed. The Windows client followed them with .NET's default
//!   handler, which keeps custom headers, so a redirect could carry `x-api-key` to
//!   another host. Here a 3xx is just a status that is not a success.

use std::time::Duration;

use reqwest::header::{CACHE_CONTROL, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Method, RequestBuilder, Response, StatusCode};

use crate::error::{ApiError, ApiResult, RefusalBody};

/// Long enough for a cold server, short enough that nobody wonders whether it hung.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The name of the header the token travels in.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Where to talk to and with what: the origin we signed in to and its API token.
///
/// The token is never printed: `Debug` shows only the host.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    /// Origin of the server without a trailing slash, e.g. `https://files.example.com`.
    pub host: String,
    pub token: String,
}

impl Credential {
    pub fn new(host: impl Into<String>, token: impl Into<String>) -> Self {
        let host: String = host.into();
        Credential {
            host: host.trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    /// An absolute URL on this origin for a root-relative path.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.host, path)
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("host", &self.host)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// The HTTP client of the app. Cheap to clone.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    /// The live upload's client: see [`transfer_client`].
    transfer: reqwest::Client,
}

impl Api {
    /// A client identifying itself as `Modlight/<version>`, like the Windows one does.
    pub fn new(version: &str) -> ApiResult<Api> {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(USER_AGENT, user_agent(version));

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(ApiError::from_reqwest)?;

        Ok(Api {
            http,
            transfer: transfer_client(version)?,
        })
    }

    /// The client the live upload goes through.
    pub(crate) fn transfer(&self) -> &reqwest::Client {
        &self.transfer
    }

    /// A request to `url` carrying the credential in the header and nowhere else.
    pub(crate) fn request(
        &self,
        method: Method,
        url: &str,
        credential: &Credential,
    ) -> RequestBuilder {
        self.http
            .request(method, url)
            .header(API_KEY_HEADER, credential.token.as_str())
    }

    /// The bare client, for the few calls that carry no credential.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// One JSON request, one JSON answer, with the refusals turned into errors
    /// (`ModlightLibrary.SendJsonAsync`): 401/403 is [`ApiError::Unauthorized`], any other
    /// failure is [`ApiError::Refused`] carrying the server's own sentence and reason.
    pub(crate) async fn send_json(
        &self,
        method: Method,
        path_or_url: &str,
        credential: &Credential,
        body: &serde_json::Value,
        timeout: Duration,
    ) -> ApiResult<String> {
        let url = absolute(path_or_url, credential);
        let response = self
            .request(method, &url, credential)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )
            .body(serde_json::to_vec(body).map_err(|e| ApiError::Decode(e.to_string()))?)
            .timeout(timeout)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;

        let status = response.status();
        let text = response.text().await.map_err(ApiError::from_reqwest)?;

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::Unauthorized);
        }

        if !status.is_success() {
            // A refusal that is not JSON at all (a proxy's error page) leaves the status
            // code as the whole of what we know.
            let refusal: RefusalBody = serde_json::from_str(&text).unwrap_or_default();
            return Err(ApiError::Refused {
                status: status.as_u16(),
                detail: refusal.message.unwrap_or_default(),
                reason: refusal.reason,
            });
        }

        Ok(text)
    }

    /// A read (`ModlightLibrary.GetStringAsync`): 401/403 is [`ApiError::Unauthorized`],
    /// any other failure is the bare [`ApiError::Status`].
    pub(crate) async fn get_string(
        &self,
        path_or_url: &str,
        credential: &Credential,
        timeout: Duration,
    ) -> ApiResult<String> {
        let url = absolute(path_or_url, credential);
        let response = self
            .request(Method::GET, &url, credential)
            .timeout(timeout)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;

        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16()));
        }
        response.text().await.map_err(ApiError::from_reqwest)
    }

    /// Read a response body up to `limit` bytes; `None` when it is longer.
    pub(crate) async fn read_limited(
        response: Response,
        limit: usize,
    ) -> ApiResult<Option<Vec<u8>>> {
        use futures_util::StreamExt;

        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Ok(None);
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(ApiError::from_reqwest)?;
            if bytes.len() + chunk.len() > limit {
                return Ok(None);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }
}

/// `ShareXResources.UserAgent`: `Modlight/<version>`, or bare `Modlight` when the
/// version is not something a header can carry.
pub(crate) fn user_agent(version: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Modlight/{version}"))
        .unwrap_or(HeaderValue::from_static("Modlight"))
}

/// A client for moving files: the User-Agent and nothing else by default, no deadline of
/// any kind, redirects never followed.
///
/// Separate from the JSON client for the reasons upstream kept its transfers apart from
/// `HttpClientFactory`. That client carries `Cache-Control: no-cache`, which neither
/// `HttpWebRequest` (the live upload) nor the queue's own `HttpClient` (tus) sends, and
/// a connect timeout neither of them has. A transfer gets no overall timeout because a
/// four-gigabyte upload legitimately takes hours; what stops a dead one is the caller's
/// cancellation, and for tus the stall watch.
pub(crate) fn transfer_client(version: &str) -> ApiResult<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, user_agent(version));
    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(ApiError::from_reqwest)
}

pub(crate) fn absolute(path_or_url: &str, credential: &Credential) -> String {
    if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
        path_or_url.to_string()
    } else {
        credential.url(path_or_url)
    }
}

/// Parse a JSON answer, or say why it could not be read.
pub(crate) fn parse<T: serde::de::DeserializeOwned + Default>(text: &str) -> ApiResult<T> {
    if text.trim().is_empty() || text.trim() == "null" {
        return Ok(T::default());
    }
    serde_json::from_str(text).map_err(|e| ApiError::Decode(e.to_string()))
}
