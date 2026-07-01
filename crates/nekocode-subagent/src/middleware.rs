use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::Provider;
use nekocode_types::tool::ToolRegistry;

use crate::factory::SubagentMiddlewareFactory;
use crate::profile::ProfileCatalog;
use crate::registry::SubagentRegistry;
use crate::tool::{
    AbortSubagentTool, InspectSubagentTool, ReadSubagentTool, SpawnSubagentTool,
    WaitAllSubagentsTool, WaitAnySubagentTool,
};

/// Shared, cheaply-cloneable context for all subagent tools. All fields are
/// Arc/Clone, so this is safe to hand to every tool.
#[derive(Clone)]
pub struct SubagentContext {
    pub registry: Arc<SubagentRegistry>,
    pub specs: Vec<MiddlewareSpec>,
    pub factory: Arc<dyn SubagentMiddlewareFactory>,
    pub parent_provider: Arc<dyn Provider>,
    pub parent_working_directory: String,
    pub parent_db: toasty::Db,
    pub catalog: Arc<ProfileCatalog>,
    pub depth: u32,
    pub max_depth: u32,
    pub allow_nested: bool,
}

/// The subagent middleware. Registered on a parent agent's middleware chain;
/// in `before_generate` it inserts the 6 subagent tools and publishes the
/// per-parent `SubagentRegistry` to `Agent.extensions["subagent"]`.
pub struct SubagentMiddleware {
    ctx: SubagentContext,
}

impl SubagentMiddleware {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        specs: Vec<MiddlewareSpec>,
        factory: Arc<dyn SubagentMiddlewareFactory>,
        parent_provider: Arc<dyn Provider>,
        parent_extensions: Arc<DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
        parent_db: toasty::Db,
        parent_working_directory: String,
        config: crate::SubagentConfig,
        depth: u32,
        allow_nested: bool,
    ) -> Self {
        let registry = Arc::new(SubagentRegistry::new());
        let global_path = global_agents_toml_path();
        let workspace_path = workspace_agents_toml_path(&parent_working_directory);
        let catalog = Arc::new(
            ProfileCatalog::load(&global_path, workspace_path.as_deref())
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to load agents.toml: {e}; using empty catalog");
                    ProfileCatalog::empty()
                }),
        );
        parent_extensions.insert(
            crate::SUBAGENT_EXTENSION_KEY.into(),
            Box::new(registry.clone()) as Box<dyn std::any::Any + Send + Sync>,
        );
        let ctx = SubagentContext {
            registry: registry.clone(),
            specs,
            factory,
            parent_provider,
            parent_working_directory,
            parent_db,
            catalog,
            depth,
            max_depth: config.max_depth,
            allow_nested,
        };
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Middleware for SubagentMiddleware {
    async fn before_generate(
        &self,
        _request: &mut nekocode_core::types::GenerateRequest,
        registry: &mut ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        let ctx = &self.ctx;
        registry.insert("spawn_subagent".into(), Arc::new(SpawnSubagentTool::new(ctx.clone())));
        registry.insert("inspect_subagent".into(), Arc::new(InspectSubagentTool::new(ctx.clone())));
        registry.insert("read_subagent".into(), Arc::new(ReadSubagentTool::new(ctx.clone())));
        registry.insert("wait_any_subagent".into(), Arc::new(WaitAnySubagentTool::new(ctx.clone())));
        registry.insert("wait_all_subagents".into(), Arc::new(WaitAllSubagentsTool::new(ctx.clone())));
        registry.insert("abort_subagent".into(), Arc::new(AbortSubagentTool::new(ctx.clone())));
        Ok(())
    }
}

/// Resolve the global agents.toml path: same dir as config.toml, i.e.
/// `<config_dir>/nekocode/agents.toml`.
fn global_agents_toml_path() -> std::path::PathBuf {
    dirs::config_dir()
        .map(|p| p.join("nekocode").join("agents.toml"))
        .unwrap_or_else(|| std::path::PathBuf::from("agents.toml"))
}

/// Resolve the workspace agents.toml path: `<working_directory>/.nekocode/agents.toml`.
fn workspace_agents_toml_path(working_directory: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(working_directory).join(".nekocode").join("agents.toml");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}
