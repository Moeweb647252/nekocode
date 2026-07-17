use std::time::Duration;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::SubagentContext;
use crate::registry::WaitAnyOutcome;
use crate::tool::{parse_agent_ids, parse_timeout};

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
        match self
            .ctx
            .registry
            .wait_any(&ids, Duration::from_secs_f64(timeout_secs))
            .await
            .map_err(|error| ToolError::ExecutionError(error.to_string()))?
        {
            WaitAnyOutcome::Ready { agent_id, snapshot } => Ok(serde_json::json!({
                "status": "ready",
                "agent_id": agent_id,
                "run_state": snapshot.name(),
            })),
            WaitAnyOutcome::Timeout { pending } => Ok(serde_json::json!({
                "status": "timeout",
                "pending": pending,
            })),
        }
    }
}
