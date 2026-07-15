//! nekocode API server — library entry point.
//!
//! Provides the application state and router for integration tests.
//! The main binary is a thin wrapper that reads config and starts the server.

use std::sync::Arc;

use axum::Router;
use nekocode_core::agent::Agent;
use nekocode_types::config::Config;
use tokio::sync::RwLock;

mod api;

pub use api::generate::{GenerateState, ThreadId};

/// Server-wide shared state, cloned and injected into every axum route.
///
/// Holds the DB handle, the config behind a `RwLock`, and two
/// `ThreadId`-keyed maps: live generation state (for stream replay/abort) and
/// activated agents. Integration tests build it by hand together with
/// [`build_router`].
#[derive(Clone)]
pub struct AppState {
    pub db: toasty::Db,
    pub config: Arc<RwLock<Config>>,
    pub generate_states: Arc<dashmap::DashMap<ThreadId, Arc<GenerateState>>>,
    pub active_threads: Arc<dashmap::DashMap<ThreadId, Arc<RwLock<Agent>>>>,
}

/// Build an axum [`Router`] with all API routes mounted under `/api`.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .nest(
            "/api",
            api::public_router().merge(
                api::protected_router()
                    .layer(axum::middleware::from_fn_with_state(
                        state.clone(),
                        api::auth_middleware,
                    )),
            ),
        )
        .with_state(state)
}
