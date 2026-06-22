//! Integration tests for the API auth middleware. Drives the full Axum
//! router via `tower::ServiceExt::oneshot` (no TCP listener) so the
//! `auth_middleware` layer is actually exercised end-to-end.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use http::{Request, StatusCode};
use nekocode::{AppState, build_router};
use nekocode_entities::prepare_db;
use nekocode_types::config::{AuthenticationConfig, Config};
use tokio::sync::RwLock;
use tower::ServiceExt;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn test_db_path() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nekocode_api_test_{}_{}.db",
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
    AppState {
        db,
        config: Arc::new(RwLock::new(config)),
        generate_states: Arc::new(dashmap::DashMap::new()),
        active_threads: Arc::new(dashmap::DashMap::new()),
    }
}

#[tokio::test]
async fn protected_route_rejects_when_no_token() {
    let state = test_state(AuthenticationConfig::Password {
        password: "secret".into(),
    })
    .await;
    let router = build_router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspace/list")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_rejects_invalid_token() {
    let state = test_state(AuthenticationConfig::Password {
        password: "secret".into(),
    })
    .await;
    let router = build_router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspace/list")
        .header("Token", "nonexistent-token")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_passes_with_no_auth_config() {
    // When auth is None, all requests pass through.
    let state = test_state(AuthenticationConfig::None).await;
    let router = build_router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspace/list")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    // Status should not be 401 — the handler runs and returns 200 (with empty list).
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn public_auth_endpoint_is_reachable_without_token() {
    let state = test_state(AuthenticationConfig::Password {
        password: "secret".into(),
    })
    .await;
    let router = build_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/auth")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "Password": { "password": "secret" }
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    // Auth endpoint itself is not behind the middleware, so it should respond
    // 200 with a token (not 401).
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn public_auth_endpoint_rejects_wrong_password() {
    let state = test_state(AuthenticationConfig::Password {
        password: "correct".into(),
    })
    .await;
    let router = build_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/auth")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "Password": { "password": "wrong" }
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_then_authenticated_request_succeeds() {
    let state = test_state(AuthenticationConfig::Password {
        password: "secret".into(),
    })
    .await;
    let router = build_router(state);

    // 1. Login.
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/auth/auth")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "Password": { "password": "secret" }
            }))
            .unwrap(),
        ))
        .unwrap();
    let login_resp = router.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(login_resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token = json
        .get("data")
        .and_then(|d| d.get("token"))
        .and_then(|t| t.as_str())
        .expect("response data should contain token field");

    // 2. Use token on a protected route.
    let req = Request::builder()
        .method("GET")
        .uri("/api/workspace/list")
        .header("Token", token)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}
