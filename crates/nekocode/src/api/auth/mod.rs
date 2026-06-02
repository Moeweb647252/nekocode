use crate::api::prelude::*;
use axum::routing::post;
use nekocode_types::config::AuthenticationConfig;

use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("auth", post(auth))
}

#[derive(Deserialize)]
pub enum Auth {
    Password { password: String },
}

pub async fn auth(State(mut state): State<AppState>, Json(payload): Json<Auth>) -> ApiResult {
    match payload {
        Auth::Password {
            password: payload_password,
        } => {
            if let AuthenticationConfig::Password { password } = {
                let lock = state.config.read().await;
                lock.auth.clone()
            } {
                if password == payload_password {
                    let token = toasty::create!(Token {
                        token: uuid::Uuid::new_v4().to_string(),
                        expires_at: jiff::Timestamp::now()
                            + jiff::SignedDuration::from_hours(24 * 30),
                    })
                    .exec(&mut state.db)
                    .await?;
                    ApiResponse::ok(token)
                } else {
                    Err(ApiError::Unauthorized)
                }
            } else {
                Err(ApiError::Unauthorized)
            }
        }
    }
}
