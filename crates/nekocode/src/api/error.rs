use axum::response::IntoResponse;
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] toasty::Error),
    #[error("Error while serializing data: {0}")]
    SerializationError(serde_json::Error),

    #[error("thread alreay activated")]
    ThreadAlreadyActivated,
    #[error("Thread not activated")]
    ThreadNotActivated,
    #[error("Thread generating")]
    ThreadGenerating,
    #[error("Item not found")]
    ItemNotFound,

    #[error("Unauthorized")]
    Unauthorized,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ApiError {
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::DatabaseError(_) => "database_error",
            ApiError::SerializationError(_) => "serialization_error",
            ApiError::ThreadAlreadyActivated => "thread_already_activated",
            ApiError::ThreadNotActivated => "thread_not_activated",
            ApiError::ThreadGenerating => "thread_generating",
            ApiError::ItemNotFound => "item_not_found",
            ApiError::Unauthorized => "unauthorized",
            ApiError::Other(_) => "other",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        error!("Failed while handling api request: {}", self);
        let body = serde_json::json!({
            "code": self.code(),
            "data": null,
            "msg": self.to_string(),
        });
        axum::response::Json(body).into_response()
    }
}
