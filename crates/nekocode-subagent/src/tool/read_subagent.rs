use nekocode_types::generate::{AssistantContentBlock, MessageType};
use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::parse_agent_id;
use crate::SubagentContext;

pub struct ReadSubagentTool {
    ctx: SubagentContext,
}

impl ReadSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for ReadSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_subagent".to_string(),
            description: "Read a finished subagent's result. By default returns only the last assistant message's text (text_only=true); with text_only=false returns the full message list. Refuses if the subagent is not finished/errored.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "integer", "description": "The agent id returned by spawn_subagent." },
                    "text_only": { "type": "boolean", "description": "If true (default), return only the last assistant text. If false, return the full message list." }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let agent_id = parse_agent_id(&params)?;
        let text_only = params
            .get("text_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if !self.ctx.registry.contains(agent_id) {
            return Err(ToolError::ExecutionError(format!(
                "agent {} not found",
                agent_id
            )));
        }
        let state = self.ctx.registry.run_state(agent_id);
        if !state.is_ready() {
            return Err(ToolError::ExecutionError(format!(
                "agent {} is not ready (state: {})",
                agent_id,
                state.name()
            )));
        }
        let result = self
            .ctx
            .registry
            .result(agent_id)
            .ok_or_else(|| ToolError::ExecutionError(format!("agent {} has no result", agent_id)))?;

        if text_only {
            let text = last_assistant_text(&result.messages).unwrap_or_default();
            Ok(serde_json::json!({
                "agent_id": agent_id,
                "status": state.name(),
                "text": text,
            }))
        } else {
            Ok(serde_json::json!({
                "agent_id": agent_id,
                "status": state.name(),
                "messages": result.messages,
            }))
        }
    }
}

/// Concatenate the text blocks of the last assistant message. Returns None
/// if there is no assistant message or it has no text blocks.
fn last_assistant_text(messages: &[nekocode_types::generate::Message]) -> Option<String> {
    let last = messages.iter().rev().find(|m| {
        matches!(m.data, MessageType::Assistant(_))
    })?;
    if let MessageType::Assistant(a) = &last.data {
        let texts: Vec<&str> = a
            .blocks
            .iter()
            .filter_map(|b| match b {
                AssistantContentBlock::Text { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        }
    } else {
        None
    }
}
