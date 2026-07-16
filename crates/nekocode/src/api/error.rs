use axum::http::StatusCode;
use axum::response::IntoResponse;
use tracing::error;

/// Errors an API handler can surface. Maps to a JSON `{code, data, msg}`
/// response body and an HTTP status (see [`ApiError::status_code`]).
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] toasty::Error),
    #[error("Error while serializing data: {0}")]
    SerializationError(serde_json::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Thread already activated")]
    ThreadAlreadyActivated,
    #[error("Thread not activated")]
    ThreadNotActivated,
    #[error("Thread generating")]
    ThreadGenerating,
    #[error("Item not found: {0}")]
    ItemNotFound(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Unauthorized")]
    Unauthorized,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ApiError {
    /// Stable string slug for this error, emitted as the response `code`
    /// field (e.g. `"unauthorized"`, `"item_not_found"`).
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::DatabaseError(_) => "database_error",
            ApiError::SerializationError(_) => "serialization_error",
            ApiError::IoError(_) => "io_error",
            ApiError::ThreadAlreadyActivated => "thread_already_activated",
            ApiError::ThreadNotActivated => "thread_not_activated",
            ApiError::ThreadGenerating => "thread_generating",
            ApiError::ItemNotFound(_) => "item_not_found",
            ApiError::InvalidInput(_) => "invalid_input",
            ApiError::Internal(_) => "internal_error",
            ApiError::Unauthorized => "unauthorized",
            ApiError::Other(_) => "other",
        }
    }

    /// Map each error to the most appropriate HTTP status code. Without this,
    /// every error response was returned as `200 OK`, defeating HTTP semantics
    /// for REST clients.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::ThreadNotActivated => StatusCode::CONFLICT,
            ApiError::ThreadAlreadyActivated | ApiError::ThreadGenerating => StatusCode::CONFLICT,
            ApiError::ItemNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            ApiError::SerializationError(_) => StatusCode::BAD_REQUEST,
            ApiError::DatabaseError(_)
            | ApiError::IoError(_)
            | ApiError::Internal(_)
            | ApiError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        error!("Failed while handling api request: {}", self);
        let status = self.status_code();
        let body = serde_json::json!({
            "code": self.code(),
            "data": null,
            "msg": self.to_string(),
        });
        (status, axum::response::Json(body)).into_response()
    }
}
