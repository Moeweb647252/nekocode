use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_subagent::SubagentMiddlewareFactory;

/// API-layer implementation of `SubagentMiddlewareFactory`. Builds isolated
/// child middleware instances by name + config — the match arms mirror
/// `build_middlewares`, but each instance is constructed with the child's
/// `subagent_id` and the child's fresh `extensions` (so shell gets its own
/// session map, file's thread_id is the synthetic subagent id).
#[derive(Clone)]
pub struct ApiSubagentMiddlewareFactory {
    pub db: toasty::Db,
    pub skills_dir: std::path::PathBuf,
}

#[async_trait::async_trait]
impl SubagentMiddlewareFactory for ApiSubagentMiddlewareFactory {
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware> {
        match spec.name.as_str() {
            "shell" => Box::new(nekocode_shell::Shell::new(
                extensions.clone(),
                nekocode_shell::config::ShellConfig::from_value(&spec.config),
            )),
            "tool" => Box::new(nekocode_file::ToolMiddleware::new(
                nekocode_file::config::FileConfig::from_value(&spec.config),
                self.db.clone(),
                subagent_id,
            )),
            "mcp" => Box::new(nekocode_mcp::McpMiddleware::new(
                nekocode_mcp::config::McpConfig::from_value(&spec.config),
            )),
            "skills" => Box::new(nekocode_skills::SkillsMiddleware::new(
                nekocode_skills::SkillsConfig::from_value(&spec.config),
                self.skills_dir.clone(),
            )),
            other => {
                tracing::warn!("unknown middleware in subagent spec: {other}; skipping");
                Box::new(NoopMiddleware)
            }
        }
    }
}

struct NoopMiddleware;
#[async_trait::async_trait]
impl Middleware for NoopMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        _: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
