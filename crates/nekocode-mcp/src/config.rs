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

    /// Serialize to a JSON value for persistence in the Middleware config
    /// cell; used when saving middleware state back to the DB, falling back
    /// to `Null` on serialization failure.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trip() {
        let cfg = McpConfig::default();
        let v = cfg.to_value();
        let cfg2 = McpConfig::from_value(&v);
        assert_eq!(cfg.transport, cfg2.transport);
        assert_eq!(cfg.server_command, cfg2.server_command);
        assert_eq!(cfg.server_url, cfg2.server_url);
        assert_eq!(cfg.envs, cfg2.envs);
        assert_eq!(cfg.auth_headers, cfg2.auth_headers);
        assert_eq!(cfg.tools_enabled, cfg2.tools_enabled);
    }

    #[test]
    fn stdio_config_round_trip() {
        let cfg = McpConfig {
            transport: Transport::Stdio,
            server_command: Some("npx -y @mcp/server".into()),
            server_url: None,
            envs: HashMap::from([("NODE_ENV".into(), "development".into())]),
            auth_headers: HashMap::new(),
            tools_enabled: HashMap::from([("read".into(), true)]),
        };
        let v = cfg.to_value();
        assert_eq!(v["transport"], "stdio");
        assert!(v.get("serverCommand").is_some());
        let cfg2 = McpConfig::from_value(&v);
        assert_eq!(cfg.transport, cfg2.transport);
        assert_eq!(cfg.server_command, cfg2.server_command);
        assert_eq!(cfg.envs, cfg2.envs);
    }

    #[test]
    fn http_config_round_trip() {
        let cfg = McpConfig {
            transport: Transport::Http,
            server_command: None,
            server_url: Some("http://localhost:8080/mcp".into()),
            envs: HashMap::new(),
            auth_headers: HashMap::from([("Authorization".into(), "Bearer test".into())]),
            tools_enabled: HashMap::new(),
        };
        let v = cfg.to_value();
        assert_eq!(v["transport"], "http");
        assert!(v.get("serverUrl").is_some());
        let cfg2 = McpConfig::from_value(&v);
        assert_eq!(cfg.transport, cfg2.transport);
        assert_eq!(cfg.server_url, cfg2.server_url);
        assert_eq!(cfg.auth_headers, cfg2.auth_headers);
    }

    #[test]
    fn null_input_gives_default() {
        let cfg = McpConfig::from_value(&serde_json::Value::Null);
        assert_eq!(cfg.transport, Transport::Stdio);
        assert!(cfg.server_command.is_none());
        assert!(cfg.server_url.is_none());
    }
}
