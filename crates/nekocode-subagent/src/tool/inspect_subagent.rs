use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::SubagentContext;
use crate::tool::parse_agent_id;

/// The `inspect_subagent` tool: polls a subagent's current run state
/// (running/finished/error) and surfaces the error message when it errored.
/// Non-blocking companion to the `wait_*` tools.
pub struct InspectSubagentTool {
    ctx: SubagentContext,
}

impl InspectSubagentTool {
    /// Construct holding a clone of the shared subagent context.
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for InspectSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "inspect_subagent".to_string(),
            description: "Inspect a subagent's current run state. Returns the state ('running', 'finished', or 'error') and, when errored, the error message.".to_string(),
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
        let snapshot =
            self.ctx.registry.snapshot(agent_id).ok_or_else(|| {
                ToolError::ExecutionError(format!("agent {} not found", agent_id))
            })?;
        let mut out = serde_json::json!({
            "agent_id": agent_id,
            "status": snapshot.name(),
        });
        if let Some(error) = snapshot.error() {
            out["error"] = serde_json::Value::String(error.to_string());
        }
        Ok(out)
    }
}
