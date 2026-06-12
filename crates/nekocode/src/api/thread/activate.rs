use std::sync::Arc;

use dashmap::Entry::{Occupied, Vacant};
use nekocode_core::agent::Agent;
use tokio::sync::RwLock;

use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct ActivateThread {
    pub id: u64,
}

pub async fn activate_thread(
    State(state): State<AppState>,
    Json(payload): Json<ActivateThread>,
) -> ApiResult {
    let thread_id = payload.id;
    if state.active_threads.contains_key(&thread_id) {
        return Err(ApiError::ThreadAlreadyActivated);
    }

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
    let provider = nekocode_provider::build_from_config(&model_config.data);

    let extensions = Arc::new(dashmap::DashMap::new());

    let mut middlewares: Vec<Box<dyn nekocode_core::middleware::Middleware>> = Vec::new();

    for i in thread.middlewares.get() {
        match i.name.as_str() {
            "shell" => {
                middlewares.push(Box::new(nekocode_shell::Shell::new(extensions.clone())));
            }
            _ => {
                tracing::warn!("Unknown middleware: {}", i.name);
            }
        }
    }

    match state.active_threads.entry(thread_id) {
        Occupied(_) => {
            // This should never happen due to the check above, but we handle it just in case.
            return Err(ApiError::ThreadAlreadyActivated);
        }
        Vacant(entry) => {
            entry.insert(Arc::new(RwLock::new(Agent {
                thread_id,
                db: state.db.clone(),
                middlewares: Arc::new(middlewares),
                provider: Arc::from(provider),
                extensions,
            })));
        }
    }

    ApiResponse::ok(())
}
