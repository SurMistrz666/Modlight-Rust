//! The typed error at the HTTP edge (PORT.md section 4: `anyhow` inside, a
//! typed error where a request is answered).
//!
//! Route handlers return `AppResult<Response>`. Expected refusals carry their
//! status and the exact upstream body; anything else is an
//! [`AppError::Internal`], which is logged with its cause chain and answered
//! like `server.ts` `fail()`: 500 with `{"message":"Internal server error"}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// `{"message": <message>}` with the given status: the shape almost every
    /// upstream route answers a refusal with.
    #[error("{status}: {message}")]
    Message { status: StatusCode, message: String },
    /// An arbitrary JSON body with the given status (token refusals, bodies
    /// with a machine-readable `reason`, ...).
    #[error("{status}: {body}")]
    Json { status: StatusCode, body: Value },
    /// A fully built response, for the rare refusal that needs extra headers.
    #[error("custom response")]
    Response(Box<Response>),
    /// Anything unexpected. Logged, never shown to the caller.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn message(status: StatusCode, message: impl Into<String>) -> Self {
        AppError::Message { status, message: message.into() }
    }

    pub fn json(status: StatusCode, body: Value) -> Self {
        AppError::Json { status, body }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        AppError::message(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        AppError::message(StatusCode::NOT_FOUND, message)
    }

    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        AppError::Internal(err.into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Internal(err.into())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Internal(err.into())
    }
}

/// The body `server.ts` `fail()` answers with.
pub fn internal_error_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "message": "Internal server error" }))).into_response()
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Message { status, message } => (status, Json(json!({ "message": message }))).into_response(),
            AppError::Json { status, body } => (status, Json(body)).into_response(),
            AppError::Response(response) => *response,
            AppError::Internal(err) => {
                tracing::error!(error = ?err, "request failed");
                internal_error_response()
            }
        }
    }
}
