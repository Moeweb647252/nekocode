use std::sync::Arc;

use dashmap::Entry::{Occupied, Vacant};
use nekocode_core::agent::Agent;
use tokio::sync::RwLock;

use crate::api::prelude::*;
use crate::api::thread::{MiddlewareBuildContext, build_middlewares};

#[derive(Deserialize)]
pub struct ActivateThread {
    pub id: u64,
}

pub async fn activate_thread(
    State(state): State<AppState>,
    Json(payload): Json<ActivateThread>,
) -> ApiResult {
    let thread_id = payload.id;

    let thread = toasty::query!(Thread FILTER .id == #thread_id)
        .include(Thread::fields().middlewares())
        .first()
        .exec(&mut state.db.clone())
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Thread not found: {}",
            thread_id
        )))?;
    let model_configs = {
        let config = state.config.read().await;
        config.models.clone()
    };
    let model_config = model_configs
        .into_iter()
        .find(|cfg| cfg.name == thread.model)
        .ok_or(ApiError::ItemNotFound(format!(
            "Model config not found: {}",
            thread.model
        )))?;
    // Build the provider once and share it via Arc — both the middleware-build
    // context (for the subagent middleware) and the Agent struct itself need
    // the same provider instance.
    let provider: Arc<dyn nekocode_core::provider::Provider> =
        Arc::from(nekocode_provider::build_from_config(&model_config.data));

    let extensions = Arc::new(dashmap::DashMap::new());

    let subthread_activator = std::sync::Arc::new(
        crate::api::thread::subthread_activator::ApiThreadActivator {
            db: state.db.clone(),
            config: state.config.clone(),
            active_threads: state.active_threads.clone(),
            generate_states: state.generate_states.clone(),
        },
    );
    let ctx = MiddlewareBuildContext {
        db: state.db.clone(),
        config: state.config.clone(),
        extensions: extensions.clone(),
        thread_id,
        working_directory: thread.working_directory.clone(),
        subthread_activator,
    };
    let middlewares = build_middlewares(&ctx, &thread.middlewares.get()).await;

    // Single atomic check-and-insert via the dashmap entry API. The redundant
    // pre-check that used to live here raced with concurrent activations and
    // surfaced a misleading generic error; the entry match below is the source
    // of truth.
    match state.active_threads.entry(thread_id) {
        Occupied(_) => Err(ApiError::ThreadAlreadyActivated),
        Vacant(entry) => {
            entry.insert(Arc::new(RwLock::new(Agent {
                thread_id,
                working_directory: thread.working_directory.clone(),
                db: state.db.clone(),
                middlewares: Arc::new(middlewares),
                provider,
                extensions,
            })));
            ApiResponse::ok(())
        }
    }
}
