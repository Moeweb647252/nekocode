use async_trait::async_trait;
use nekocode_types::tool::{Tool, ToolError, ToolSpec};
use std::sync::Arc;

use crate::client::McpClient;

/// A tool that forwards calls to the connected MCP server.
pub struct McpTool {
    pub client: Arc<McpClient>,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: self.description.clone(),
            parameter_schema: self.input_schema.clone(),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let result = self
            .client
            .call_tool(&self.name, params)
            .await
            .map_err(|e| ToolError::ExecutionError(e.to_string()))?;
        // The MCP `tools/call` response includes `content` (array of content
        // blocks) and `isError`. We return the whole result object; the agent
        // will serialize it into `ToolCallResultInner::Success`.
        Ok(result)
    }
}
