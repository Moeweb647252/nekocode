use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-MCP-server configuration. Supports two connection modes:
/// - **stdio**: `server_command` is the shell command to start the MCP server.
/// - **http**: `server_url` is the HTTP endpoint for Streamable HTTP transport.
/// The two fields are mutually exclusive; if both are set, `server_url` takes
/// precedence (HTTP is preferred when available).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpConfig {
    /// Shell command to spawn an MCP server over stdio (e.g.
    /// `"npx -y @modelcontextprotocol/server-filesystem"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_command: Option<String>,
    /// HTTP URL for Streamable HTTP transport (e.g.
    /// `"http://localhost:8080/mcp"`). Preferred over `server_command` if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
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

    /// Returns `true` if HTTP mode is configured (`server_url` is set).
    pub fn is_http(&self) -> bool {
        self.server_url.is_some()
    }
}