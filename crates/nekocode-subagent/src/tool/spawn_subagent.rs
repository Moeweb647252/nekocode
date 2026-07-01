use std::sync::Arc;

use nekocode_core::agent::Agent;
use nekocode_core::middleware::MiddlewareSpec;
use nekocode_types::tool::{Tool, ToolError, ToolSpec};
use tokio::sync::mpsc;

use crate::middleware::SubagentMiddleware;
use crate::runner::run_subagent;
use crate::SubagentContext;

pub struct SpawnSubagentTool {
    ctx: SubagentContext,
}

impl SpawnSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for SpawnSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "spawn_subagent".to_string(),
            description: "Spawn a single-turn subagent that runs a given prompt to completion under a named profile. Returns immediately with status 'running'. Poll completion via inspect_subagent, wait_any_subagent, or wait_all_subagents; read the result via read_subagent. Refuses if the profile is unknown, if the profile requests middlewares the parent did not enable, or if the nesting depth limit is exceeded.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "The profile name to load from agents.toml."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The user message to run as the subagent's single turn."
                    }
                },
                "required": ["profile", "prompt"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let profile_name = params
            .get("profile")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'profile' parameter".into()))?;
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'prompt' parameter".into()))?
            .to_string();

        let profile = self
            .ctx
            .catalog
            .get(profile_name)
            .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;

        // Gate A: the parent's profile must allow nesting.
        if !self.ctx.allow_nested {
            return Err(ToolError::ExecutionError(
                "parent profile does not allow nested subagents".into(),
            ));
        }
        // Gate B: depth bound.
        if self.ctx.depth + 1 > self.ctx.max_depth {
            return Err(ToolError::ExecutionError(
                "max subagent nesting depth exceeded".into(),
            ));
        }

        // Middleware intersection: profile.middlewares must be ⊆ parent specs.
        let spec_names: std::collections::HashSet<&str> =
            self.ctx.specs.iter().map(|s| s.name.as_str()).collect();
        for m in &profile.middlewares {
            if !spec_names.contains(m.as_str()) {
                return Err(ToolError::ExecutionError(format!(
                    "profile '{}' requests middleware '{}', not enabled by parent",
                    profile_name, m
                )));
            }
        }
        let selected_specs: Vec<MiddlewareSpec> = self
            .ctx
            .specs
            .iter()
            .filter(|s| profile.middlewares.contains(&s.name))
            .cloned()
            .collect();

        let agent_id = self.ctx.registry.allocate_running();
        let child_extensions = Arc::new(dashmap::DashMap::new());

        // Build isolated middleware instances via the factory.
        let mut child_middlewares: Vec<Box<dyn nekocode_core::middleware::Middleware>> = Vec::new();
        for spec in &selected_specs {
            child_middlewares.push(self.ctx.factory.build(
                spec.clone(),
                agent_id,
                child_extensions.clone(),
            ));
        }

        // Compute the child's working directory BEFORE building the child
        // middleware: a profile may override `working_directory`, and the
        // child's ProfileCatalog must load relative to the child's directory,
        // not the parent's.
        let working_directory = profile
            .working_directory
            .clone()
            .unwrap_or_else(|| self.ctx.parent_working_directory.clone());

        // Construct the child's own SubagentMiddleware (at depth+1, with the
        // child profile's allow_nested). It registers the subagent tools for
        // the child so it can itself spawn (subject to the gates above).
        // Pass `selected_specs` (parent specs ∩ profile.middlewares), NOT the
        // parent's full spec set, so the intersection gate holds at depth >= 2;
        // and pass the computed `working_directory` so the child catalog loads
        // from the right place.
        let child_subagent_mw = SubagentMiddleware::new(
            selected_specs.clone(),
            self.ctx.factory.clone(),
            self.ctx.parent_provider.clone(),
            child_extensions.clone(),
            self.ctx.parent_db.clone(),
            working_directory.clone(),
            crate::SubagentConfig { max_depth: self.ctx.max_depth },
            self.ctx.depth + 1,
            profile.allow_nested,
        );
        child_middlewares.push(Box::new(child_subagent_mw));

        let child = Agent {
            thread_id: agent_id,
            working_directory,
            db: self.ctx.parent_db.clone(),
            middlewares: Arc::new(child_middlewares),
            provider: self.ctx.parent_provider.clone(),
            extensions: child_extensions,
        };

        // Drained-sender pattern: a companion task drains the event channel so
        // run_loop's send() never blocks when no one consumes.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let registry = self.ctx.registry.clone();
        let handle = tokio::spawn(async move {
            let drain = tokio::spawn(async move {
                while rx.recv().await.is_some() {}
            });
            run_subagent(agent_id, child, prompt, registry, nekocode_core::agent::AgentEventSink::new(tx))
                .await;
            drain.abort();
        });

        self.ctx.registry.set_running(agent_id, handle);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": "running",
        }))
    }
}
