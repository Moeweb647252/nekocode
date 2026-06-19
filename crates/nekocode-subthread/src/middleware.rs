use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::Middleware;
use nekocode_types::tool::ToolRegistry;

use crate::{
    SubthreadConfig, SubthreadRegistry,
    activator::ThreadActivator,
    tool::{
        DeleteSubthreadTool, InspectSubthreadTool, ListSubthreadsTool, ReadSubthreadTool,
        SetSubthreadSettingsTool, SpawnSubthreadTool, StartSubthreadTool, SubthreadContext,
        SubthreadSettingsTool, WaitAllSubthreadsTool, WaitAnySubthreadTool,
    },
};

/// Extension key under which the per-parent `SubthreadRegistry` is stored on
/// the parent's `Agent.extensions`. The API layer (cascade delete) reads it
/// from there to abort any in-flight subthread tasks.
pub const SUBTHREAD_EXTENSION_KEY: &str = "subthread";

/// Per-parent subthread middleware. On each generation it registers the nine
/// subthread tools into the parent's `ToolRegistry`. The middleware owns the
/// per-parent `SubthreadRegistry`, exposed via `Agent.extensions` under
/// `SUBTHREAD_EXTENSION_KEY` so the API layer can reach it for cascade
/// cleanup. This mirrors `nekocode_shell::Shell`'s ownership of `shell_states`.
pub struct SubthreadMiddleware {
    ctx: SubthreadContext,
}

impl SubthreadMiddleware {
    /// Build a middleware for `parent_thread_id`. A fresh `SubthreadRegistry`
    /// is created and shared with the parent's `Agent.extensions` so external
    /// callers (cascade delete) can locate it.
    pub fn new(
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
        db: toasty::Db,
        parent_thread_id: u64,
        parent_working_directory: String,
        config: SubthreadConfig,
        activator: Arc<dyn ThreadActivator>,
    ) -> Self {
        let registry = Arc::new(SubthreadRegistry::new());
        // Publish the registry to the agent's extensions so the API layer can
        // reach it (e.g. delete_thread → abort all subthreads). Keep a clone
        // for our own tools' use.
        extensions.insert(
            SUBTHREAD_EXTENSION_KEY.to_string(),
            Box::new(registry.clone()),
        );
        let ctx = SubthreadContext {
            db,
            parent_thread_id,
            parent_working_directory,
            registry,
            config: Arc::new(config),
            activator: Some(activator),
        };
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Middleware for SubthreadMiddleware {
    async fn before_generate(
        &self,
        _request: &mut nekocode_core::types::GenerateRequest,
        registry: &mut ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        let ctx = self.ctx.clone();
        registry.insert(
            "spawn_subthread".into(),
            Arc::new(SpawnSubthreadTool::new(ctx.clone())),
        );
        registry.insert(
            "list_subthreads".into(),
            Arc::new(ListSubthreadsTool::new(ctx.clone())),
        );
        registry.insert(
            "inspect_subthread".into(),
            Arc::new(InspectSubthreadTool::new(ctx.clone())),
        );
        registry.insert(
            "read_subthread".into(),
            Arc::new(ReadSubthreadTool::new(ctx.clone())),
        );
        registry.insert(
            "subthread_settings".into(),
            Arc::new(SubthreadSettingsTool::new(ctx.clone())),
        );
        registry.insert(
            "set_subthread_settings".into(),
            Arc::new(SetSubthreadSettingsTool::new(ctx.clone())),
        );
        registry.insert(
            "start_subthread".into(),
            Arc::new(StartSubthreadTool::new(ctx.clone())),
        );
        registry.insert(
            "wait_any_subthread".into(),
            Arc::new(WaitAnySubthreadTool::new(ctx.clone())),
        );
        registry.insert(
            "wait_all_subthreads".into(),
            Arc::new(WaitAllSubthreadsTool::new(ctx.clone())),
        );
        registry.insert(
            "delete_subthread".into(),
            Arc::new(DeleteSubthreadTool::new(ctx)),
        );
        Ok(())
    }
}
