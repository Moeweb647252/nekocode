pub mod auth;
pub mod error;
pub mod generate;
pub mod middleware;
pub mod thread;
pub mod util;

use axum::{
    Router,
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use nekocode_entities::token::Token;
use nekocode_types::config::AuthenticationConfig;
use serde::Serialize;

use crate::{AppState, api::error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/thread", thread::router())
        .nest("/auth", auth::router())
        .nest("/generate", generate::router())
        .nest("/util", util::router())
}

pub type ApiResult = Result<ApiResponse, ApiError>;

#[derive(Debug, Serialize)]
pub struct ApiResponse {
    pub code: String,
    pub data: serde_json::Value,
    pub msg: Option<String>,
}

impl ApiResponse {
    pub fn ok<T: Serialize>(data: T) -> Result<Self, ApiError> {
        Ok(Self {
            code: "ok".to_string(),
            data: serde_json::to_value(data).map_err(|e| ApiError::SerializationError(e))?,
            msg: None,
        })
    }

    pub fn msg(self, msg: String) -> Self {
        Self {
            msg: Some(msg),
            ..self
        }
    }
}

impl IntoResponse for ApiResponse {
    fn into_response(self) -> Response {
        axum::response::Json(self).into_response()
    }
}

async fn auth_middleware_inner(
    mut state: AppState,
    headers: &axum::http::HeaderMap,
) -> Result<bool, ApiError> {
    let config = {
        let lock = state.config.read().await;
        lock.auth.clone()
    };
    if config == AuthenticationConfig::None {
        return Ok(true);
    }
    let token = headers
        .get("Token")
        .ok_or(ApiError::Unauthorized)?
        .to_str()
        .map_err(|e| anyhow::Error::from(e))?;
    if let Some(token) = toasty::query!(Token FILTER .token == #token)
        .first()
        .exec(&mut state.db)
        .await?
    {
        if token.expires_at > jiff::Timestamp::now() {
            Ok(true)
        } else {
            token.delete().exec(&mut state.db).await?;
            Ok(false)
        }
    } else {
        Ok(false)
    }
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if match auth_middleware_inner(state, request.headers()).await {
        Ok(valid) => valid,
        Err(e) => return e.into_response(),
    } {
        next.run(request).await
    } else {
        ApiError::Unauthorized.into_response()
    }
}

pub mod prelude {
    pub use crate::{
        AppState,
        api::{ApiResponse, ApiResult, error::ApiError},
    };
    pub use axum::{Json, extract::State};
    pub use nekocode_entities::{message::Message, thread::Thread, token::Token, turn::Turn};
    pub use serde::{Deserialize, Serialize};
}
