use std::path::PathBuf;
use std::sync::Arc;

use nekocode_core::agent::Agent;
use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::Provider;
use nekocode_entities::thread::Thread;

use super::subagent_factory::RuntimeSubagentMiddlewareFactory;
use super::subthread_controller::RuntimeSubthreadController;
use super::{RuntimeError, ThreadRuntime};

impl ThreadRuntime {
    pub(crate) async fn activate_new(
        self: &Arc<Self>,
        thread_id: u64,
    ) -> Result<Arc<Agent>, RuntimeError> {
        let lifecycle = self.lifecycle_guard().await;
        lifecycle.ensure_idle(thread_id)?;
        if self.agents.contains(thread_id) {
            return Err(RuntimeError::ThreadAlreadyActivated);
        }
        let agent = self.build_agent(thread_id).await?;
        self.agents.activate_new(thread_id, agent)
    }

    pub(crate) async fn invalidate_agent(&self, thread_id: u64) -> Result<(), RuntimeError> {
        let lifecycle = self.lifecycle_guard().await;
        lifecycle.ensure_idle(thread_id)?;
        lifecycle.remove_and_shutdown(thread_id).await;
        Ok(())
    }

    pub(crate) async fn build_agent(
        self: &Arc<Self>,
        thread_id: u64,
    ) -> Result<Agent, RuntimeError> {
        let mut db = self.db.clone();
        let thread = toasty::query!(Thread FILTER .id == #thread_id)
            .include(Thread::fields().middlewares())
            .first()
            .exec(&mut db)
            .await?
            .ok_or_else(|| RuntimeError::ItemNotFound(format!("Thread not found: {thread_id}")))?;
        let model = {
            let config = self.config.read().await;
            config
                .models
                .iter()
                .find(|model| model.name == thread.model)
                .cloned()
                .ok_or_else(|| RuntimeError::ModelNotFound(thread.model.clone()))?
        };
        let provider: Arc<dyn Provider> =
            Arc::from(nekocode_provider::build_from_config(&model.data));
        let extensions = Extensions::new();
        let middlewares = self
            .build_middlewares(
                thread_id,
                thread.working_directory.clone(),
                thread.middlewares.get(),
                extensions.clone(),
                provider.clone(),
            )
            .await;
        Ok(Agent {
            thread_id,
            working_directory: thread.working_directory,
            db: self.db.clone(),
            middlewares: Arc::new(middlewares),
            provider,
            extensions,
        })
    }

    async fn build_middlewares(
        self: &Arc<Self>,
        thread_id: u64,
        working_directory: String,
        middleware_rows: &[nekocode_entities::middleware::Middleware],
        extensions: Extensions,
        provider: Arc<dyn Provider>,
    ) -> Vec<Box<dyn Middleware>> {
        let mut rows = middleware_rows.to_vec();
        rows.sort_by_key(|row| (row.order_index, row.id));
        let skills_dir = {
            let config = self.config.read().await;
            PathBuf::from(config.skills.directory.clone())
        };
        let subthread_controller: Arc<dyn nekocode_subthread::controller::SubthreadController> =
            Arc::new(RuntimeSubthreadController::new(Arc::downgrade(self)));
        let mut middlewares: Vec<Box<dyn Middleware>> = Vec::new();

        for row in &rows {
            if !row.enabled {
                continue;
            }
            match row.name.as_str() {
                "shell" => middlewares.push(Box::new(nekocode_shell::Shell::new(
                    extensions.clone(),
                    nekocode_shell::config::ShellConfig::from_value(&row.config),
                ))),
                "tool" => middlewares.push(Box::new(nekocode_file::ToolMiddleware::new(
                    nekocode_file::config::FileConfig::from_value(&row.config),
                    self.db.clone(),
                    thread_id,
                ))),
                "mcp" => middlewares.push(Box::new(nekocode_mcp::McpMiddleware::new(
                    nekocode_mcp::config::McpConfig::from_value(&row.config),
                ))),
                "skills" => middlewares.push(Box::new(nekocode_skills::SkillsMiddleware::new(
                    nekocode_skills::SkillsConfig::from_value(&row.config),
                    skills_dir.clone(),
                ))),
                "subthread" => {
                    middlewares.push(Box::new(nekocode_subthread::SubthreadMiddleware::new(
                        extensions.clone(),
                        self.db.clone(),
                        thread_id,
                        working_directory.clone(),
                        nekocode_subthread::SubthreadConfig::from_value(&row.config),
                        subthread_controller.clone(),
                    )))
                }
                "subagent" => {
                    let specs: Vec<MiddlewareSpec> = rows
                        .iter()
                        .filter(|candidate| candidate.enabled && candidate.name != "subagent")
                        .map(|candidate| MiddlewareSpec {
                            name: candidate.name.clone(),
                            config: candidate.config.0.clone(),
                        })
                        .collect();
                    middlewares.push(Box::new(nekocode_subagent::SubagentMiddleware::new(
                        specs,
                        Arc::new(RuntimeSubagentMiddlewareFactory {
                            skills_dir: skills_dir.clone(),
                        }),
                        provider.clone(),
                        extensions.clone(),
                        self.db.clone(),
                        working_directory.clone(),
                        nekocode_subagent::SubagentConfig::from_value(&row.config),
                        0,
                        true,
                    )));
                }
                unknown => tracing::warn!("Unknown middleware: {unknown}"),
            }
        }
        middlewares
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekocode_types::config::{Config, DeepSeekConfig, ModelConfig, ModelConfigType};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    async fn runtime() -> Arc<ThreadRuntime> {
        let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
        let db = nekocode_entities::prepare_db(std::env::temp_dir().join(format!(
            "nekocode_runtime_agent_{}_{}.db",
            std::process::id(),
            sequence
        )))
        .await
        .unwrap();
        ThreadRuntime::new(
            db,
            Config {
                default_model: "test".to_string(),
                models: vec![ModelConfig {
                    name: "test".to_string(),
                    data: ModelConfigType::DeepSeek(DeepSeekConfig {
                        api_base: "http://127.0.0.1:1".to_string(),
                        api_key: "test".to_string(),
                        model: "test".to_string(),
                        temperature: None,
                        top_p: None,
                        top_k: None,
                        max_tokens: None,
                        context_window_size: None,
                        endpoint: None,
                    }),
                }],
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn activation_is_unique_and_invalidation_evicts_the_agent() {
        let runtime = runtime().await;
        let thread = runtime.create_root("/tmp".to_string()).await.unwrap();
        runtime.activate_new(thread.id).await.unwrap();
        assert!(runtime.is_active(thread.id));
        assert!(matches!(
            runtime.activate_new(thread.id).await,
            Err(RuntimeError::ThreadAlreadyActivated)
        ));
        runtime.invalidate_agent(thread.id).await.unwrap();
        assert!(!runtime.is_active(thread.id));
    }

    #[tokio::test]
    async fn activation_reports_missing_thread_before_model_resolution() {
        let runtime = runtime().await;
        assert!(matches!(
            runtime.activate_new(999).await,
            Err(RuntimeError::ItemNotFound(_))
        ));
    }
}
