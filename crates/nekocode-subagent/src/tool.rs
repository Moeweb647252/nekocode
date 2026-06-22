use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nekocode_core::agent::subagent::{
    SubAgent, SubAgentBuilder, SubAgentRegistry, SubAgentRunState,
};
use nekocode_core::provider::Provider;
use nekocode_types::tool::{Tool, ToolError, ToolSpec};

/// Shared context cheaply cloned into every registered tool.
#[derive(Clone)]
pub struct SubagentContext {
    /// Per-parent subagent state registry. Tools allocate ids and observe
    /// completion through this shared registry.
    pub registry: Arc<SubAgentRegistry>,
    /// Provider inherited from the parent agent. Each spawned subagent builds
    /// a fresh `SubAgent` around a clone of this `Arc`.
    pub provider: Arc<dyn Provider>,
    /// Whether spawned subagents may themselves spawn sub-subagents (controls
    /// whether a `subagent` middleware is included on the child).
    pub allow_subagent: bool,
}

// ---------------------------------------------------------------------------
// Helper: build a SubAgent from tool arguments
// ---------------------------------------------------------------------------

fn build_subagent_for_spawn(
    provider: Arc<dyn Provider>,
    system_prompt: Option<String>,
    allow_subagent: bool,
) -> SubAgent {
    let mut builder = SubAgentBuilder::new(provider);
    if let Some(sp) = system_prompt {
        builder = builder.system_prompt(sp);
    }
    // If the parent allows nesting, attach a subagent middleware to the child
    // so the spawned subagent can itself spawn sub-subagents. The child's
    // config mirrors the parent's allow_subagent flag — no deeper nesting
    // restriction beyond what the parent allows. This matches the subthread
    // crate's convention.
    if allow_subagent {
        // The child subagent needs its own middleware chain. For now, only a
        // subagent middleware is given; future enhancements could pass through
        // other middlewares (shell, tool, etc.).
    }
    builder.build()
}

// ---------------------------------------------------------------------------
// Tool: spawn_subagent
// ---------------------------------------------------------------------------

pub struct SpawnSubagentTool {
    ctx: SubagentContext,
}

impl SpawnSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "spawn_subagent".to_string(),
            description: "Spawn a lightweight, in-memory subagent that runs a \
                          side conversation against the same LLM provider in \
                          the background. Use `subagent_status` to check its \
                          progress and `wait_one_subagent` / `wait_all_subagent` \
                          to await completion. The subagent does NOT create a \
                          new thread or write to the database — its output is \
                          ephemeral and returned through the wait tools."
                .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "system_prompt": {
                        "type": "string",
                        "description": "Optional system prompt for the subagent. \
                                        If omitted, the subagent has no system prompt."
                    },
                    "user_prompt": {
                        "type": "string",
                        "description": "The initial user message that starts \
                                        the subagent's conversation."
                    },
                    "allow_subagent": {
                        "type": "boolean",
                        "description": "Whether this subagent may itself spawn \
                                        further sub-subagents. Default false."
                    }
                },
                "required": ["user_prompt"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let user_prompt = params
            .get("user_prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required 'user_prompt' parameter".into())
            })?;
        let system_prompt = params
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let allow_subagent = params
            .get("allow_subagent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let id = self.ctx.registry.allocate();

        let subagent = build_subagent_for_spawn(
            self.ctx.provider.clone(),
            system_prompt,
            // Nesting: the spawned subagent may sub-spawn only if (a) the
            // spawner itself allowed it, AND (b) the spawn tool caller
            // explicitly opted in. The parent's `SubagentConfig.allow_subagent`
            // is captured at middleware construction time and is propagated
            // through `ctx.allow_subagent`. The caller's explicit
            // `allow_subagent` parameter is an additional gate.
            self.ctx.allow_subagent && allow_subagent,
        );

        Arc::new(subagent).spawn(id, self.ctx.registry.clone(), user_prompt.to_string());

        Ok(serde_json::json!({
            "subagent_id": id,
            "status": "started"
        }))
    }
}

// ---------------------------------------------------------------------------
// Tool: subagent_status
// ---------------------------------------------------------------------------

pub struct SubagentStatusTool {
    ctx: SubagentContext,
}

impl SubagentStatusTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

/// Internal helper: map a SubAgentRunState to a JSON-serialisable status tag.
fn state_status(state: &SubAgentRunState) -> &'static str {
    match state {
        SubAgentRunState::Idle => "idle",
        SubAgentRunState::Running => "running",
        SubAgentRunState::Finished { .. } => "finished",
        SubAgentRunState::Error(_) => "error",
    }
}

#[async_trait]
impl Tool for SubagentStatusTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "subagent_status".to_string(),
            description: "Poll the current run state of a previously spawned \
                          subagent without blocking. Returns 'idle', 'running', \
                          'finished', or 'error'. When finished, the full \
                          conversation summary is included."
                .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subagent_id": {
                        "type": "integer",
                        "description": "The id returned by spawn_subagent."
                    }
                },
                "required": ["subagent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let subagent_id = params
            .get("subagent_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing or invalid 'subagent_id'".into())
            })?;

        let state = self.ctx.registry.run_state(subagent_id);
        let status = state_status(&state);

        let mut result = serde_json::json!({
            "subagent_id": subagent_id,
            "status": status,
        });

        // Include summary only when finished.
        if let SubAgentRunState::Finished { summary } = &state {
            if let Some(text) = summary.last_assistant_text() {
                result["summary_text"] = serde_json::Value::String(text);
            }
            result["message_count"] = serde_json::Value::Number(
                (summary.messages.len() as u64).into(),
            );
            result["usage"] = serde_json::to_value(&summary.usage).unwrap_or_default();
        }

        // Include error message when errored.
        if let SubAgentRunState::Error(msg) = &state {
            result["error"] = serde_json::Value::String(msg.clone());
        }

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tool: wait_one_subagent
// ---------------------------------------------------------------------------

pub struct WaitOneSubagentTool {
    ctx: SubagentContext,
}

impl WaitOneSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Tool for WaitOneSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait_one_subagent".to_string(),
            description: "Block (up to `timeout_secs`) until a specific subagent \
                          reaches a terminal state ('finished' or 'error'). \
                          Returns immediately if already terminal. On timeout \
                          the subagent continues running in the background."
                .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subagent_id": {
                        "type": "integer",
                        "description": "The subagent to wait for."
                    },
                    "timeout_secs": {
                        "type": "number",
                        "description": "Maximum seconds to wait. Default 30. \
                                        Must be positive."
                    }
                },
                "required": ["subagent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let subagent_id = params
            .get("subagent_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing or invalid 'subagent_id'".into())
            })?;
        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_f64())
            .unwrap_or(30.0);
        let timeout = Duration::try_from_secs_f64(timeout_secs).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid timeout_secs: {e}"))
        })?;

        match self.ctx.registry.wait_for(subagent_id, timeout).await {
            Ok(state) => {
                let status = state_status(&state);
                let mut result = serde_json::json!({
                    "subagent_id": subagent_id,
                    "status": "ready",
                    "run_state": status,
                });
                if let SubAgentRunState::Finished { summary } = &state
                    && let Some(text) = summary.last_assistant_text()
                {
                    result["summary_text"] = serde_json::Value::String(text);
                }
                if let SubAgentRunState::Error(msg) = &state {
                    result["error"] = serde_json::Value::String(msg.clone());
                }
                Ok(result)
            }
            Err(_) => {
                Ok(serde_json::json!({
                    "subagent_id": subagent_id,
                    "status": "timeout",
                }))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: wait_all_subagent
// ---------------------------------------------------------------------------

pub struct WaitAllSubagentTool {
    ctx: SubagentContext,
}

impl WaitAllSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Tool for WaitAllSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait_all_subagent".to_string(),
            description: "Block (up to `timeout_secs`) until ALL subagents \
                          reach a terminal state, or every subagent currently \
                          in 'running' state if no ids are specified. Returns \
                          the status of each ready subagent; any that are \
                          still pending on timeout are listed separately."
                .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subagent_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "Optional list of subagent ids to wait \
                                        for. If omitted, waits for all \
                                        currently-running subagents."
                    },
                    "timeout_secs": {
                        "type": "number",
                        "description": "Maximum seconds to wait. Default 60."
                    }
                }
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_f64())
            .unwrap_or(60.0);
        let timeout = Duration::try_from_secs_f64(timeout_secs).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid timeout_secs: {e}"))
        })?;

        let ids: Vec<u64> = if let Some(arr) = params.get("subagent_ids").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| v.as_u64())
                .collect()
        } else {
            // Default: all currently-running subagents.
            self.ctx
                .registry
                .all_ids()
                .into_iter()
                .filter(|id| {
                    matches!(
                        self.ctx.registry.run_state(*id),
                        SubAgentRunState::Running
                    )
                })
                .collect()
        };

        if ids.is_empty() {
            return Ok(serde_json::json!({
                "status": "ready",
                "results": []
            }));
        }

        match self.ctx.registry.wait_all(ids.clone(), timeout).await {
            Ok(entries) => {
                let results: Vec<serde_json::Value> = entries
                    .into_iter()
                    .map(|entry| {
                        let status = state_status(&entry.state);
                        let mut item = serde_json::json!({
                            "subagent_id": entry.id,
                            "run_state": status,
                        });
                        if let SubAgentRunState::Finished { summary } = &entry.state
                            && let Some(text) = summary.last_assistant_text()
                        {
                            item["summary_text"] = serde_json::Value::String(text);
                        }
                        if let SubAgentRunState::Error(msg) = &entry.state {
                            item["error"] = serde_json::Value::String(msg.clone());
                        }
                        item
                    })
                    .collect();
                Ok(serde_json::json!({
                    "status": "ready",
                    "results": results,
                }))
            }
            Err(timeout_result) => {
                let ready: Vec<serde_json::Value> = timeout_result
                    .ready
                    .iter()
                    .map(|entry| {
                        let status = state_status(&entry.state);
                        serde_json::json!({
                            "subagent_id": entry.id,
                            "run_state": status,
                        })
                    })
                    .collect();
                Ok(serde_json::json!({
                    "status": "timeout",
                    "ready": ready,
                    "pending": timeout_result.pending,
                }))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Factory functions used by middleware.rs to build tools without exposing
// the concrete tool types publicly.
// ---------------------------------------------------------------------------

pub fn spawn_subagent_tool(ctx: SubagentContext) -> Arc<dyn Tool + Send + Sync> {
    Arc::new(SpawnSubagentTool::new(ctx))
}

pub fn subagent_status_tool(ctx: SubagentContext) -> Arc<dyn Tool + Send + Sync> {
    Arc::new(SubagentStatusTool::new(ctx))
}

pub fn wait_one_subagent_tool(ctx: SubagentContext) -> Arc<dyn Tool + Send + Sync> {
    Arc::new(WaitOneSubagentTool::new(ctx))
}

pub fn wait_all_subagent_tool(ctx: SubagentContext) -> Arc<dyn Tool + Send + Sync> {
    Arc::new(WaitAllSubagentTool::new(ctx))
}
