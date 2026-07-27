use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Application-wide error type.
#[derive(Debug)]
pub enum AppError {
    /// Resource not found (room, game, player).
    NotFound(String),
    /// Malformed or invalid request data.
    BadRequest(String),
    /// Attempted to create something that already exists.
    AlreadyExists(String),
    /// Missing or invalid auth token / join code.
    Unauthorized(String),
    /// Unexpected internal failure.
    Internal(String),
    /// Failed to fetch data from Wikimedia APIs.
    WikimediaFetch(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg),
            AppError::AlreadyExists(msg) => (StatusCode::CONFLICT, "already_exists", msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg),
            AppError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg)
            }
            AppError::WikimediaFetch(msg) => {
                (StatusCode::BAD_GATEWAY, "wikimedia_fetch_error", msg)
            }
        };

        (
            status,
            Json(ErrorResponse {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound(msg) => write!(f, "Not found: {msg}"),
            AppError::BadRequest(msg) => write!(f, "Bad request: {msg}"),
            AppError::AlreadyExists(msg) => write!(f, "Already exists: {msg}"),
            AppError::Unauthorized(msg) => write!(f, "Unauthorized: {msg}"),
            AppError::Internal(msg) => write!(f, "Internal error: {msg}"),
            AppError::WikimediaFetch(msg) => write!(f, "Wikimedia fetch error: {msg}"),
        }
    }
}

impl std::error::Error for AppError {}

// ── Convenient From conversions ───────────────────────────────────────────

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::WikimediaFetch(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::BadRequest(err.to_string())
    }
}
