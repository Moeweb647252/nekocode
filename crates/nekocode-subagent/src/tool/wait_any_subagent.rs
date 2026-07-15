use std::time::Duration;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::{notify_any, parse_agent_ids, parse_timeout, run_state_name};
use crate::SubagentContext;

/// The `wait_any_subagent` tool: blocks until any one of the listed subagents
/// reaches a terminal state (returning that one) or the timeout elapses
/// (returning the still-pending ids). Never kills running subagents on timeout.
pub struct WaitAnySubagentTool {
    ctx: SubagentContext,
}

impl WaitAnySubagentTool {
    /// Construct holding a clone of the shared subagent context.
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for WaitAnySubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait_any_subagent".to_string(),
            description: "Wait until any one of the specified subagents becomes ready (finished or errored), or until the timeout elapses. Returns the first ready subagent on success, or the list of still-pending subagents on timeout. Does NOT kill or affect running subagents on timeout.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "The agent ids to wait on."
                    },
                    "timeout": { "type": "number", "description": "Maximum seconds to wait. Must be positive." }
                },
                "required": ["agent_ids", "timeout"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let ids = parse_agent_ids(&params)?;
        let timeout_secs = parse_timeout(&params)?;
        for id in &ids {
            if !self.ctx.registry.contains(*id) {
                return Err(ToolError::ExecutionError(format!("agent {} not found", id)));
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
        loop {
            for id in &ids {
                let state = self.ctx.registry.run_state(*id);
                if state.is_ready() {
                    return Ok(serde_json::json!({
                        "status": "ready",
                        "agent_id": id,
                        "run_state": run_state_name(&state),
                    }));
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "pending": ids,
                }));
            }
            let notifies: Vec<_> = ids
                .iter()
                .filter_map(|id| self.ctx.registry.notify(*id))
                .collect();
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            if notifies.is_empty() {
                (&mut sleep).await;
            } else {
                tokio::select! {
                    _ = sleep => {}
                    _ = notify_any(&notifies) => {}
                }
            }
        }
    }
}
