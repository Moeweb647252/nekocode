use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::{parse_agent_id, run_state_name};
use crate::SubagentContext;

pub struct InspectSubagentTool {
    ctx: SubagentContext,
}

impl InspectSubagentTool {
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
        if !self.ctx.registry.contains(agent_id) {
            return Err(ToolError::ExecutionError(format!(
                "agent {} not found",
                agent_id
            )));
        }
        let state = self.ctx.registry.run_state(agent_id);
        let mut out = serde_json::json!({
            "agent_id": agent_id,
            "status": run_state_name(&state),
        });
        if let crate::registry::SubagentRunState::Error(msg) = &state {
            out["error"] = serde_json::Value::String(msg.clone());
        }
        Ok(out)
    }
}
