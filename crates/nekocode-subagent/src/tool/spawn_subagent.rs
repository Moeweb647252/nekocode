use std::sync::Arc;

use nekocode_core::agent::Agent;
use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::MiddlewareSpec;
use nekocode_types::tool::{Tool, ToolError, ToolSpec};
use tokio::sync::mpsc;

use crate::SubagentContext;
use crate::middleware::SubagentMiddleware;
use crate::runner::run_subagent;

/// The `spawn_subagent` tool: configures a single-turn child agent under a
/// named profile (intersecting the profile's middlewares with the parent's
/// enabled set, enforcing the nesting gates), sends it to the background via
/// `run_subagent`, and returns immediately with status `running`. It also
/// spawns the relay task that forwards the child's `AgentEvent`s to the parent
/// stream as `MiddlewareEvent`s. Holds the parent's `MiddlewareEvent` sender in
/// addition to the shared context.
pub struct SpawnSubagentTool {
    ctx: SubagentContext,
    mev_tx: tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
}

impl SpawnSubagentTool {
    /// Construct with the subagent context and the parent's `MiddlewareEvent`
    /// sender (so the relay task can forward child events onto the parent
    /// stream).
    pub fn new(
        ctx: SubagentContext,
        mev_tx: tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Self {
        Self { ctx, mev_tx }
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
            .map_err(|e| ToolError::InvalidParameters(e.to_string()))?
            .clone();

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

        let ctx = self.ctx.clone();
        let mev_tx = self.mev_tx.clone();
        let registry = self.ctx.registry.clone();
        let agent_id = registry
            .spawn(move |task| {
                let agent_id = task.agent_id;
                let child_extensions = Extensions::new();
                let mut child_middlewares: Vec<Box<dyn nekocode_core::middleware::Middleware>> =
                    selected_specs
                        .iter()
                        .map(|spec| {
                            ctx.factory
                                .build(spec.clone(), agent_id, child_extensions.clone())
                        })
                        .collect();

                let working_directory = profile
                    .working_directory
                    .clone()
                    .unwrap_or_else(|| ctx.parent_working_directory.clone());
                let run_cancel = ctx
                    .run_cancel
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                let child_subagent_mw = SubagentMiddleware::new(
                    selected_specs,
                    ctx.factory.clone(),
                    ctx.parent_provider.clone(),
                    child_extensions.clone(),
                    ctx.parent_db.clone(),
                    working_directory.clone(),
                    crate::SubagentConfig {
                        max_depth: ctx.max_depth,
                    },
                    ctx.depth + 1,
                    profile.allow_nested,
                )
                .with_run_cancel(run_cancel.clone());
                child_middlewares.push(Box::new(child_subagent_mw));

                let child = Agent {
                    thread_id: agent_id,
                    working_directory,
                    db: ctx.parent_db.clone(),
                    middlewares: Arc::new(child_middlewares),
                    provider: ctx.parent_provider.clone(),
                    extensions: child_extensions,
                };
                let (child_tx, mut child_rx) = mpsc::unbounded_channel();
                async move {
                    let relay = tokio::spawn(async move {
                        while let Some(child_event) = child_rx.recv().await {
                            let event = nekocode_core::agent::MiddlewareEvent {
                                source: std::borrow::Cow::Borrowed("subagent"),
                                source_id: agent_id,
                                event_type: "agentEvent".into(),
                                data: serde_json::to_value(&child_event)
                                    .unwrap_or(serde_json::Value::Null),
                            };
                            let _ = mev_tx.send(event);
                        }
                    });
                    let outcome = run_subagent(
                        child,
                        prompt,
                        nekocode_core::agent::AgentEventSink::new(child_tx),
                        task.cancel,
                        run_cancel,
                    )
                    .await;
                    let _ = relay.await;
                    outcome
                }
            })
            .map_err(|error| ToolError::ExecutionError(error.to_string()))?;

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": "running",
        }))
    }
}
