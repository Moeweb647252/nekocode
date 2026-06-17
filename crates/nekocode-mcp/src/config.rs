use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MCP transport: stdio (spawn a process) or http (Streamable HTTP). The two
/// are mutually exclusive — `transport` selects which config fields apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Stdio,
    Http,
}

/// Per-MCP-server configuration. `transport` selects the connection mode:
/// - **stdio**: `server_command` is the shell command to start the MCP server.
/// - **http**: `server_url` is the HTTP endpoint for Streamable HTTP transport.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpConfig {
    /// Connection mode (stdio or http).
    #[serde(default)]
    pub transport: Transport,
    /// Shell command to spawn an MCP server over stdio (e.g.
    /// `"npx -y @modelcontextprotocol/server-filesystem"`). stdio mode only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_command: Option<String>,
    /// HTTP URL for Streamable HTTP transport (e.g.
    /// `"http://localhost:8080/mcp"`). http mode only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    /// Custom HTTP headers to include in requests (http mode).
    /// Map of header name → value (e.g., `"Authorization"` → `"Bearer xxx"`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub auth_headers: HashMap<String, String>,
    /// Environment variables injected into the spawned process (stdio mode).
    #[serde(default)]
    pub envs: HashMap<String, String>,
    /// Map of tool name → enabled status. Only tools with `true` here are
    /// registered into the agent's ToolRegistry during `before_generate`.
    #[serde(default)]
    pub tools_enabled: HashMap<String, bool>,
}

impl McpConfig {
    /// Deserialize from a JSON value, falling back to defaults on failure.
    pub fn from_value(v: &serde_json::Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}