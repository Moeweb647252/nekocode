use crate::api::prelude::*;
use axum::routing::post;
use constant_time_eq::constant_time_eq;
use nekocode_types::config::AuthenticationConfig;

use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/auth", post(auth))
}

#[derive(Deserialize)]
pub enum Auth {
    Password { password: String },
}

pub async fn auth(State(state): State<AppState>, Json(payload): Json<Auth>) -> ApiResult {
    match payload {
        Auth::Password {
            password: payload_password,
        } => {
            if let AuthenticationConfig::Password { password } = {
                let config = state.config();
                let lock = config.read().await;
                lock.auth.clone()
            } {
                // Constant-time comparison to prevent timing side-channel attacks.
                if constant_time_eq(password.as_bytes(), payload_password.as_bytes()) {
                    let mut db = state.db();
                    let token = toasty::create!(Token {
                        token: uuid::Uuid::new_v4().to_string(),
                        expires_at: jiff::Timestamp::now()
                            + jiff::SignedDuration::from_hours(24 * 30),
                    })
                    .exec(&mut db)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::auth_middleware_inner;
    use axum::http::HeaderMap;
    use nekocode_entities::prepare_db;
    use nekocode_types::config::Config;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_db_path() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "nekocode_auth_test_{}_{}.db",
            std::process::id(),
            n
        ))
    }

    async fn test_state(auth: AuthenticationConfig) -> AppState {
        let db = prepare_db(test_db_path()).await.expect("prepare_db");
        let config = Config {
            auth,
            ..Default::default()
        };
        AppState::new(db, config)
    }

    // ── auth_middleware_inner tests ──

    #[tokio::test]
    async fn auth_none_passes_without_token() {
        let state = test_state(AuthenticationConfig::None).await;
        let headers = HeaderMap::new();
        let result = auth_middleware_inner(state, &headers).await;
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn auth_password_requires_token() {
        let state = test_state(AuthenticationConfig::Password {
            password: "pwd".into(),
        })
        .await;
        let headers = HeaderMap::new();
        let result = auth_middleware_inner(state, &headers).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn auth_valid_token_passes() {
        let state = test_state(AuthenticationConfig::Password {
            password: "pwd".into(),
        })
        .await;
        let mut db = state.db();
        let token = toasty::create!(Token {
            token: uuid::Uuid::new_v4().to_string(),
            expires_at: jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("Token", token.token.parse().unwrap());
        let result = auth_middleware_inner(state, &headers).await;
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn auth_websocket_subprotocol_token_passes() {
        let state = test_state(AuthenticationConfig::Password {
            password: "pwd".into(),
        })
        .await;
        let mut db = state.db();
        let token = toasty::create!(Token {
            token: uuid::Uuid::new_v4().to_string(),
            expires_at: jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::SEC_WEBSOCKET_PROTOCOL,
            token.token.parse().unwrap(),
        );

        let result = auth_middleware_inner(state, &headers).await;
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn auth_expired_token_fails() {
        let state = test_state(AuthenticationConfig::Password {
            password: "pwd".into(),
        })
        .await;
        let mut db = state.db();
        let token = toasty::create!(Token {
            token: uuid::Uuid::new_v4().to_string(),
            expires_at: jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("Token", token.token.parse().unwrap());
        let result = auth_middleware_inner(state, &headers).await;
        // Returns Ok(false) — not authorized, token deleted
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn auth_nonexistent_token_fails() {
        let state = test_state(AuthenticationConfig::Password {
            password: "pwd".into(),
        })
        .await;
        let mut headers = HeaderMap::new();
        headers.insert("Token", "does-not-exist".parse().unwrap());
        let result = auth_middleware_inner(state, &headers).await;
        assert!(!result.unwrap());
    }

    // ── auth handler tests ──

    #[tokio::test]
    async fn auth_correct_password_returns_token() {
        let state = test_state(AuthenticationConfig::Password {
            password: "correct".into(),
        })
        .await;
        let payload = Auth::Password {
            password: "correct".into(),
        };
        let result = auth(State(state), Json(payload)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn auth_wrong_password_fails() {
        let state = test_state(AuthenticationConfig::Password {
            password: "correct".into(),
        })
        .await;
        let payload = Auth::Password {
            password: "wrong".into(),
        };
        let result = auth(State(state), Json(payload)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn auth_when_config_none_fails() {
        let state = test_state(AuthenticationConfig::None).await;
        let payload = Auth::Password {
            password: "anything".into(),
        };
        let result = auth(State(state), Json(payload)).await;
        assert!(result.is_err());
    }
}
