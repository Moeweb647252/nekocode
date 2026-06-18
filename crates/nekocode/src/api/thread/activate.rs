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
        // Skip disabled middlewares — they stay persisted but aren't built
        // into the agent.
        if !i.enabled {
            continue;
        }
        match i.name.as_str() {
            "shell" => {
                let cfg = nekocode_shell::config::ShellConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_shell::Shell::new(extensions.clone(), cfg)));
            }
            "tool" => {
                let cfg = nekocode_tool::config::FileConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_tool::ToolMiddleware::new(
                    cfg,
                    state.db.clone(),
                    thread_id,
                )));
            }
            "mcp" => {
                let cfg = nekocode_mcp::config::McpConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_mcp::McpMiddleware::new(cfg)));
            }
            "skills" => {
                let cfg = nekocode_skills::SkillsConfig::from_value(&i.config);
                let skills_dir = {
                    let config = state.config.read().await;
                    std::path::PathBuf::from(config.skills.directory.clone())
                };
                middlewares.push(Box::new(nekocode_skills::SkillsMiddleware::new(
                    cfg, skills_dir,
                )));
            }
            _ => {
                tracing::warn!("Unknown middleware: {}", i.name);
            }
        }
    }

    // Single atomic check-and-insert via the dashmap entry API. The redundant
    // pre-check that used to live here raced with concurrent activations and
    // surfaced a misleading generic error; the entry match below is the source
    // of truth.
    match state.active_threads.entry(thread_id) {
        Occupied(_) => Err(ApiError::ThreadAlreadyActivated),
        Vacant(entry) => {
            entry.insert(Arc::new(RwLock::new(Agent {
                thread_id,
                db: state.db.clone(),
                middlewares: Arc::new(middlewares),
                provider: Arc::from(provider),
                extensions,
            })));
            ApiResponse::ok(())
        }
    }
}
