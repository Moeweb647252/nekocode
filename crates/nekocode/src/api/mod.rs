pub mod auth;
pub mod error;
pub mod generate;
pub mod middleware;
pub mod thread;
pub mod util;
pub mod workspace;

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

/// Routes that do not require authentication (e.g. login endpoint).
pub fn public_router() -> Router<AppState> {
    Router::new().nest("/auth", auth::router())
}

/// Routes that require authentication (all other API endpoints).
pub fn protected_router() -> Router<AppState> {
    Router::new()
        .nest("/thread", thread::router())
        .nest("/workspace", workspace::router())
        .nest("/generate", generate::router())
        .nest("/middleware", middleware::router())
        .nest("/util", util::router())
}

/// Standard `Result` alias for API handlers: `ApiResponse` on success,
/// [`ApiError`] on failure.
pub type ApiResult = Result<ApiResponse, ApiError>;

/// The uniform JSON response body for every API endpoint:
/// `{ code: "ok", data, msg }`. `code` is a stable string; `data` is the
/// payload; `msg` carries an optional message.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse {
    pub code: String,
    pub data: serde_json::Value,
    pub msg: Option<String>,
}

impl ApiResponse {
    /// Wrap a serializable payload in a successful `ok` response.
    pub fn ok<T: Serialize>(data: T) -> Result<Self, ApiError> {
        Ok(Self {
            code: "ok".to_string(),
            data: serde_json::to_value(data).map_err(ApiError::SerializationError)?,
            msg: None,
        })
    }
}

impl IntoResponse for ApiResponse {
    fn into_response(self) -> Response {
        axum::response::Json(self).into_response()
    }
}

pub(crate) async fn auth_middleware_inner(
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
        .map_err(anyhow::Error::from)?;
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

/// axum middleware garding the protected router: validates the `Token`
/// header against the DB (skipping the check entirely when auth is
/// `AuthenticationConfig::None`) before forwarding the request. Returns
/// `401 unauthorized` otherwise.
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
    pub use nekocode_entities::{
        message::Message, middleware::Middleware, thread::Thread, token::Token, turn::Turn,
        workspace::Workspace,
    };
    pub use serde::{Deserialize, Serialize};
}
