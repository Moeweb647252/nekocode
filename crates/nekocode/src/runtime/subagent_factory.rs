use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_subagent::SubagentMiddlewareFactory;

#[derive(Clone)]
pub(crate) struct RuntimeSubagentMiddlewareFactory {
    pub(crate) skills_dir: std::path::PathBuf,
}

#[async_trait::async_trait]
impl SubagentMiddlewareFactory for RuntimeSubagentMiddlewareFactory {
    fn build(
        &self,
        spec: MiddlewareSpec,
        _subagent_id: u64,
        extensions: Extensions,
    ) -> Box<dyn Middleware> {
        match spec.name.as_str() {
            "shell" => Box::new(nekocode_shell::Shell::new(
                extensions.clone(),
                nekocode_shell::config::ShellConfig::from_value(&spec.config),
            )),
            "tool" => Box::new(nekocode_file::ToolMiddleware::for_ephemeral_agent(
                nekocode_file::config::FileConfig::from_value(&spec.config),
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
        _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
