use axum::routing::{get, post};
use std::sync::Arc;

use crate::AppState;
use nekocode_core::extensions::Extensions;

pub mod activate;
pub mod create;
pub mod delete;
pub mod get;
pub mod list;
pub mod subagent_factory;
pub mod subthread_controller;
pub mod update;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/create", post(create::create_thread))
        .route("/list", get(list::list_threads))
        .route("/delete", post(delete::delete_thread))
        .route("/activate", post(activate::activate_thread))
        .route("/update", post(update::update_thread))
        .route("/get", post(get::get_thread))
}

/// Shared context needed to build middleware instances from DB rows.
/// Both `activate_thread` and `ApiThreadController` use this to avoid
/// duplicating the middleware construction logic.
pub(crate) struct MiddlewareBuildContext {
    pub db: toasty::Db,
    pub config: Arc<tokio::sync::RwLock<nekocode_types::config::Config>>,
    pub extensions: Extensions,
    pub thread_id: u64,
    pub working_directory: String,
    pub subthread_controller: Arc<dyn nekocode_subthread::controller::ThreadController>,
    pub provider: Arc<dyn nekocode_core::provider::Provider>,
}

pub(crate) async fn shutdown_and_remove_agent(
    active_threads: &dashmap::DashMap<u64, Arc<tokio::sync::RwLock<nekocode_core::agent::Agent>>>,
    thread_id: u64,
) {
    if let Some((_, agent)) = active_threads.remove(&thread_id) {
        agent.read().await.shutdown().await;
    }
}

/// Build the middleware chain from a thread's persisted middleware rows.
/// The `ctx` parameter carries all the state needed to construct each
/// middleware. The caller is responsible for providing the correct
/// `subthread_controller` (which differs between top-level activation
/// and subthread activation).
pub(crate) async fn build_middlewares(
    ctx: &MiddlewareBuildContext,
    middleware_rows: &[nekocode_entities::middleware::Middleware],
) -> Vec<Box<dyn nekocode_core::middleware::Middleware>> {
    let mut middlewares: Vec<Box<dyn nekocode_core::middleware::Middleware>> = Vec::new();

    for i in middleware_rows {
        // Skip disabled middlewares — they stay persisted but aren't built
        // into the agent.
        if !i.enabled {
            continue;
        }
        match i.name.as_str() {
            "shell" => {
                let cfg = nekocode_shell::config::ShellConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_shell::Shell::new(
                    ctx.extensions.clone(),
                    cfg,
                )));
            }
            "tool" => {
                let cfg = nekocode_file::config::FileConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_file::ToolMiddleware::new(
                    cfg,
                    ctx.db.clone(),
                    ctx.thread_id,
                )));
            }
            "mcp" => {
                let cfg = nekocode_mcp::config::McpConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_mcp::McpMiddleware::new(cfg)));
            }
            "skills" => {
                let cfg = nekocode_skills::SkillsConfig::from_value(&i.config);
                let skills_dir = {
                    let config = ctx.config.read().await;
                    std::path::PathBuf::from(config.skills.directory.clone())
                };
                middlewares.push(Box::new(nekocode_skills::SkillsMiddleware::new(
                    cfg, skills_dir,
                )));
            }
            "subthread" => {
                let cfg = nekocode_subthread::SubthreadConfig::from_value(&i.config);
                middlewares.push(Box::new(nekocode_subthread::SubthreadMiddleware::new(
                    ctx.extensions.clone(),
                    ctx.db.clone(),
                    ctx.thread_id,
                    ctx.working_directory.clone(),
                    cfg,
                    ctx.subthread_controller.clone(),
                )));
            }
            "subagent" => {
                let cfg = nekocode_subagent::SubagentConfig::from_value(&i.config);
                // Build specs from the parent's enabled middleware rows,
                // excluding "subagent" itself (prevents recursive self-
                // registration; nesting is governed by depth+max_depth, not
                // by omission here).
                let specs: Vec<nekocode_core::middleware::MiddlewareSpec> = middleware_rows
                    .iter()
                    .filter(|r| r.enabled && r.name != "subagent")
                    .map(|r| nekocode_core::middleware::MiddlewareSpec {
                        name: r.name.clone(),
                        config: r.config.0.clone(),
                    })
                    .collect();
                let skills_dir = {
                    let config = ctx.config.read().await;
                    std::path::PathBuf::from(config.skills.directory.clone())
                };
                let factory = std::sync::Arc::new(
                    crate::api::thread::subagent_factory::ApiSubagentMiddlewareFactory {
                        db: ctx.db.clone(),
                        skills_dir,
                    },
                );
                middlewares.push(Box::new(nekocode_subagent::SubagentMiddleware::new(
                    specs,
                    factory,
                    ctx.provider.clone(),
                    ctx.extensions.clone(),
                    ctx.db.clone(),
                    ctx.working_directory.clone(),
                    cfg,
                    0, // depth = 0 for the top-level parent
                    true, // allow_nested = true at the root: the top-level
                       // thread is not itself a subagent, so it is always
                       // permitted to spawn its first-level subagents.
                       // The depth gate + each child's own profile
                       // allow_nested bound further nesting.
                )));
            }
            _ => {
                tracing::warn!("Unknown middleware: {}", i.name);
            }
        }
    }

    middlewares
}
