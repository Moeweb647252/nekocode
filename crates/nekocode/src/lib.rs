//! nekocode API server — library entry point.
//!
//! Provides the application state and router for integration tests.
//! The main binary is a thin wrapper that reads config and starts the server.

use axum::Router;
use nekocode_types::config::Config;

mod api;
mod runtime;

/// Server-wide shared state, cloned and injected into every axum route.
///
#[derive(Clone)]
pub struct AppState {
    runtime: std::sync::Arc<runtime::ThreadRuntime>,
}

impl AppState {
    pub fn new(db: toasty::Db, config: Config) -> Self {
        Self {
            runtime: runtime::ThreadRuntime::new(db, config),
        }
    }

    pub(crate) fn runtime(&self) -> std::sync::Arc<runtime::ThreadRuntime> {
        self.runtime.clone()
    }

    pub(crate) fn db(&self) -> toasty::Db {
        self.runtime.db()
    }

    pub(crate) fn config(&self) -> std::sync::Arc<tokio::sync::RwLock<Config>> {
        self.runtime.config()
    }
}

/// Build an axum [`Router`] with all API routes mounted under `/api`.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .nest(
            "/api",
            api::public_router().merge(api::protected_router().layer(
                axum::middleware::from_fn_with_state(state.clone(), api::auth_middleware),
            )),
        )
        .with_state(state)
}
