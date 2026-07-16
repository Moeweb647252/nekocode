use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::SubagentContext;
use crate::tool::parse_agent_id;

/// The `abort_subagent` tool: fires the subagent's per-agent cancellation
/// token, aborts its background task if still running, and removes its entry
/// from the registry — discarding any in-memory result.
pub struct AbortSubagentTool {
    ctx: SubagentContext,
}

impl AbortSubagentTool {
    /// Construct holding a clone of the shared subagent context.
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for AbortSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "abort_subagent".to_string(),
            description: "Abort a subagent's background task (if running) and remove it from the registry. The subagent's in-memory result is discarded.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "integer", "description": "The agent id returned by spawn_subagent." }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let agent_id = parse_agent_id(&params)?;
        if !self.ctx.registry.contains(agent_id) {
            return Err(ToolError::ExecutionError(format!(
                "agent {} not found",
                agent_id
            )));
        }
        self.ctx.registry.abort(agent_id).await;
        Ok(serde_json::json!({
            "agent_id": agent_id,
            "aborted": true,
        }))
    }
}
