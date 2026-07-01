use std::sync::Arc;

use nekocode_core::extensions::Extensions;
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
    /// A single `CancellationToken` shared across the whole spawn tree (the
    /// root created it; every descendant clones the same flag into its own
    /// context). When the parent turn ends, the root's `on_turn_end` cancels
    /// this token, so every descendant `run_subagent`'s `select!` observes the
    /// cancellation concurrently and bails — no reliance on the runtime
    /// re-poling one layer before the next. This is what makes the cascade
    /// "real recursion across depth" rather than best-effort.
    pub run_cancel: tokio_util::sync::CancellationToken,
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
        parent_extensions: Extensions,
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
        parent_extensions.insert(registry.clone());
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
            // The root creates the shared run-cancel flag; descendants clone
            // it (passed down via the child SubagentMiddleware's context) so
            // the whole tree subscribes to the same cancellation.
            run_cancel: tokio_util::sync::CancellationToken::new(),
        };
        Self::from_context(ctx)
    }

    /// Build a middleware from an already-constructed context (tests that
    /// build `SubagentContext` directly use this; `new` builds the ctx
    /// then delegates here).
    pub fn from_context(ctx: SubagentContext) -> Self {
        Self { ctx }
    }

    /// Replace this middleware's tree cancellation flag with `token`. Used by
    /// `spawn_subagent` so the child and all its descendants subscribe to the
    /// SAME flag the root created — cancelling once from the root's
    /// `on_turn_end` wakes every descendant `run_subagent` across all depth.
    /// (`new` mints its own fresh flag; at nested spawn we must re-point the
    /// child's flag at the inherited one.)
    pub fn with_run_cancel(mut self, token: tokio_util::sync::CancellationToken) -> Self {
        self.ctx.run_cancel = token;
        self
    }
}

#[async_trait::async_trait]
impl Middleware for SubagentMiddleware {
    async fn before_generate(
        &self,
        _request: &mut nekocode_core::types::GenerateRequest,
        registry: &mut ToolRegistry,
        mev_tx: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        let ctx = &self.ctx;
        registry.insert(
            "spawn_subagent".into(),
            Arc::new(SpawnSubagentTool::new(ctx.clone(), mev_tx.clone())),
        );
        registry.insert("inspect_subagent".into(), Arc::new(InspectSubagentTool::new(ctx.clone())));
        registry.insert("read_subagent".into(), Arc::new(ReadSubagentTool::new(ctx.clone())));
        registry.insert("wait_any_subagent".into(), Arc::new(WaitAnySubagentTool::new(ctx.clone())));
        registry.insert("wait_all_subagents".into(), Arc::new(WaitAllSubagentsTool::new(ctx.clone())));
        registry.insert("abort_subagent".into(), Arc::new(AbortSubagentTool::new(ctx.clone())));
        Ok(())
    }

    async fn on_turn_end(&self) -> Result<(), anyhow::Error> {
        // Parent turn is over: no subagent may outlive it. First cancel the
        // shared `run_cancel` token — every descendant `run_subagent` across
        // the whole spawn tree subscribes to it and bails at its next await,
        // each driving its *own* middlewares' `on_turn_end` so grandchildren
        // cascade down too (real recursion across depth, no reliance on the
        // runtime re-poling one layer before the next). Then `abort_all_and_clear`
        // JoinHandle-aborts any still-running direct children that hadn't
        // yielded to the token yet and clears this registry.
        self.ctx.run_cancel.cancel();
        self.ctx.registry.abort_all_and_clear();
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
