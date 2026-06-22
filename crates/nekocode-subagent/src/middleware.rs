use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use nekocode_core::agent::subagent::{
    SubAgentRegistry, SUBAGENT_EXTENSION_KEY,
};
use nekocode_core::middleware::Middleware;
use nekocode_core::provider::Provider;
use nekocode_core::types::GenerateRequest;
use nekocode_types::tool::ToolRegistry;

use crate::config::SubagentConfig;
use crate::tool::{SubagentContext, spawn_subagent_tool, subagent_status_tool, wait_one_subagent_tool, wait_all_subagent_tool};

/// Middleware that registers subagent-management tools (`spawn_subagent`,
/// `subagent_status`, `wait_one_subagent`, `wait_all_subagent`) into the
/// parent agent's tool registry.
///
/// The middleware publishes the shared [`SubAgentRegistry`] into the parent
/// agent's `extensions` map under [`SUBAGENT_EXTENSION_KEY`] so that other
/// middlewares (e.g. `compact`) and API-layer cleanup logic can reach the
/// same registry.
///
/// # Nesting
///
/// When `config.allow_subagent` is true, the spawned subagent will itself
/// receive a `subagent` middleware, bounded by the same flag. This mirrors
/// the subthread crate's nesting convention.
pub struct SubagentMiddleware {
    ctx: SubagentContext,
}

impl SubagentMiddleware {
    pub fn new(
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
        parent_provider: Arc<dyn Provider>,
        config: SubagentConfig,
    ) -> Self {
        let registry = Arc::new(SubAgentRegistry::new());
        extensions.insert(
            SUBAGENT_EXTENSION_KEY.to_string(),
            Box::new(registry.clone()),
        );
        let ctx = SubagentContext {
            registry,
            provider: parent_provider,
            allow_subagent: config.allow_subagent,
        };
        Self { ctx }
    }
}

#[async_trait]
impl Middleware for SubagentMiddleware {
    async fn before_generate(
        &self,
        _request: &mut GenerateRequest,
        tool_registry: &mut ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        let ctx = self.ctx.clone();
        tool_registry.insert(
            "spawn_subagent".into(),
            spawn_subagent_tool(ctx.clone()),
        );
        tool_registry.insert(
            "subagent_status".into(),
            subagent_status_tool(ctx.clone()),
        );
        tool_registry.insert(
            "wait_one_subagent".into(),
            wait_one_subagent_tool(ctx.clone()),
        );
        tool_registry.insert(
            "wait_all_subagent".into(),
            wait_all_subagent_tool(ctx),
        );
        Ok(())
    }
}
