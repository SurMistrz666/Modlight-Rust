//! Every way a call to the server can end other than with the answer.
//!
//! The variants follow the exception types of the Windows client one for one,
//! because each of them has a different remedy and the screens tell them apart:
//!
//! * [`ApiError::Unauthorized`]: `ModlightUnauthorizedException`, sign in again.
//! * [`ApiError::Refused`]: `ModlightApiException`, the server said no in its own words.
//! * [`ApiError::Status`]: `EnsureSuccessStatusCode` failing on a read.
//! * [`ApiError::Timeout`], [`ApiError::Transport`]: the request never completed.
//! * [`ApiError::NotSupported`], [`ApiError::AdminForbidden`]: the administration screen only.
//! * [`ApiError::Locked`]: not upstream; the account's files are encrypted and this request
//!   carried no grant that opens them (`423 Locked`). The token is fine: the remedy is the
//!   file password, never signing in again.

use std::fmt;

/// The JSON body every refusal from the Modlight API is shaped as.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RefusalBody {
    #[serde(default)]
    pub message: Option<String>,
    /// A stable token for the refusals a client might act on, e.g. `code-taken`.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The server answered 401 or 403: this credential is not one it will act on.
    Unauthorized,
    /// The server understood the request and refused it. `detail` is its own sentence
    /// (empty when it wrote none), `reason` the machine-readable token when it sent one.
    Refused {
        status: u16,
        detail: String,
        reason: Option<String>,
    },
    /// A read that came back with a status that is not a success and not a refusal of
    /// the credential (what `HttpResponseMessage.EnsureSuccessStatusCode` throws on).
    Status(u16),
    /// Our own deadline ran out before the server finished answering.
    Timeout,
    /// Nothing usable came back: DNS, refused connection, TLS, a dropped socket.
    Transport(String),
    /// The server answered with a body that is not the JSON this call expects.
    Decode(String),
    /// The server has no such endpoint (or will not admit to having it): 404 on the
    /// administration report.
    NotSupported,
    /// The credential is real and too narrow for the administration report: 403 there.
    AdminForbidden,
    /// The caller stopped waiting.
    Cancelled,
    /// `423 Locked` (not upstream): the account's files are encrypted on the server and
    /// the request carried no grant that opens them. Never a reason to sign in again.
    Locked(FilesLocked),
}

/// What a `423 Locked` answer said, and which grant the refused request carried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesLocked {
    /// The tag of the account whose files these are, when the server named it.
    pub owner: Option<String>,
    /// The grant the request carried, by the number [`crate::Api::hold_grant`] gave it;
    /// `None` when it carried none. A refusal of the grant still held says that grant
    /// no longer works; a refusal of an older one (or of none) says nothing about it.
    pub grant: Option<u64>,
}

impl ApiError {
    /// Whether the refusal is the one a different name would fix.
    pub fn is_name_taken(&self) -> bool {
        matches!(self, ApiError::Refused { reason: Some(r), .. } if r == "code-taken" || r == "code-reserved")
    }

    /// Whether the account's files are locked (encrypted, and no grant opened them).
    pub fn is_locked(&self) -> bool {
        matches!(self, ApiError::Locked(_))
    }

    /// The HTTP status behind this error, when the server answered at all.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Refused { status, .. } | ApiError::Status(status) => Some(*status),
            ApiError::Unauthorized => Some(401),
            ApiError::Locked(_) => Some(423),
            _ => None,
        }
    }

    pub(crate) fn from_reqwest(error: reqwest::Error) -> ApiError {
        if error.is_timeout() {
            return ApiError::Timeout;
        }
        if error.is_decode() {
            return ApiError::Decode(error.to_string());
        }
        ApiError::Transport(describe_transport(&error))
    }
}

/// The innermost cause of a transport failure, which is the part a person can act on
/// ("connection refused", "dns error"), without the URL reqwest puts in front of it.
fn describe_transport(error: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text = cause.to_string();
        source = cause.source();
    }
    if let Some(url) = error.url() {
        // Never repeat a URL in an error message: a query string can carry anything.
        text = text.replace(url.as_str(), "");
    }
    text.trim().to_string()
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "the server refused the API token"),
            ApiError::Refused {
                status,
                detail,
                reason,
            } => {
                if !detail.is_empty() {
                    write!(f, "{detail}")
                } else if let Some(reason) = reason {
                    write!(f, "{reason} ({status})")
                } else {
                    write!(f, "the server refused the request ({status})")
                }
            }
            ApiError::Status(status) => {
                // .NET's two sentences: with the standard reason phrase, or without one
                // when the status has none.
                match reqwest::StatusCode::from_u16(*status)
                    .ok()
                    .and_then(|code| code.canonical_reason())
                {
                    Some(reason) => write!(
                        f,
                        "Response status code does not indicate success: {status} ({reason})."
                    ),
                    None => write!(
                        f,
                        "Response status code does not indicate success: {status}."
                    ),
                }
            }
            ApiError::Timeout => write!(f, "The request timed out."),
            ApiError::Transport(text) => write!(f, "{text}"),
            ApiError::Decode(text) => write!(
                f,
                "The server sent an answer that could not be read: {text}"
            ),
            ApiError::NotSupported => write!(f, "the server has no such endpoint"),
            ApiError::AdminForbidden => write!(f, "the API token is not allowed to do this"),
            ApiError::Cancelled => write!(f, "cancelled"),
            ApiError::Locked(_) => write!(
                f,
                "the account's files are encrypted; unlock them with the file password"
            ),
        }
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// `HttpResponseMessage.EnsureSuccessStatusCode`'s sentence, which the library's
    /// "could not reach" line quotes: the status with its standard reason phrase.
    #[test]
    fn a_failed_read_says_its_status_as_dotnet_does() {
        assert_eq!(
            ApiError::Status(500).to_string(),
            "Response status code does not indicate success: 500 (Internal Server Error)."
        );
        assert_eq!(
            ApiError::Status(404).to_string(),
            "Response status code does not indicate success: 404 (Not Found)."
        );
        // A status with no reason phrase is said without one, as .NET says it.
        assert_eq!(
            ApiError::Status(599).to_string(),
            "Response status code does not indicate success: 599."
        );
    }
}
