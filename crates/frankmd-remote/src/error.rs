//! JSON errors with stable machine codes.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use frankmd_fs::FsError;

/// One failed request. `code` is stable for scripts; `message` is for humans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// HTTP status.
    pub status: StatusCode,
    /// Stable snake_case code.
    pub code: &'static str,
    /// Human-readable message (never contains secrets).
    pub message: String,
}

impl ApiError {
    /// Build one error.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// 400.
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// 401.
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication required",
        )
    }

    /// 403.
    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    /// 404.
    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    /// 409.
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    /// 422.
    pub fn unprocessable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    /// 429.
    pub fn too_many_requests() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            "Too many login attempts. Please try again later.",
        )
    }

    /// 500.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// 503.
    pub fn unavailable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, code, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.message, "code": self.code })),
        )
            .into_response()
    }
}

/// Map a filesystem error to the HTTP taxonomy used by the original Rails app.
pub fn fs_error(err: &FsError) -> ApiError {
    match err {
        FsError::AlreadyExists(message) => ApiError::conflict("already_exists", message.clone()),
        FsError::NotFound(message) => ApiError::not_found("not_found", message.clone()),
        FsError::RevisionConflict(message) => {
            ApiError::conflict("revision_conflict", message.clone())
        }
        FsError::NotEmpty(message) => ApiError::conflict("not_empty", message.clone()),
        FsError::InvalidPath(message) => ApiError::bad_request("invalid_path", message.clone()),
        FsError::PermissionDenied(message) => {
            ApiError::forbidden("permission_denied", message.clone())
        }
        FsError::Io(e) => ApiError::internal(e.to_string()),
    }
}
