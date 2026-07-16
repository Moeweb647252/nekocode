use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeMcp {
    pub transport: String,
    pub server_command: Option<String>,
    pub server_url: Option<String>,
    #[serde(default)]
    pub envs: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub auth_headers: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpProbeResponse {
    pub tools: Vec<McpProbeToolInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpProbeToolInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Test an MCP server connection: connect, initialize, list tools, then
/// disconnect. Returns the discovered tool list so the UI can populate the
/// enabled-tools map. Does not modify any persisted state.
pub async fn probe_mcp(Json(payload): Json<ProbeMcp>) -> ApiResult {
    let transport = match payload.transport.as_str() {
        "stdio" => nekocode_mcp::config::Transport::Stdio,
        "http" => nekocode_mcp::config::Transport::Http,
        other => {
            return Err(ApiError::InvalidInput(format!(
                "invalid transport: {}",
                other
            )));
        }
    };
    let config = nekocode_mcp::config::McpConfig {
        transport,
        server_command: payload.server_command,
        server_url: payload.server_url,
        envs: payload.envs,
        auth_headers: payload.auth_headers,
        tools_enabled: Default::default(),
    };
    let tools = match nekocode_mcp::probe(&config).await {
        Ok(t) => t,
        Err(e) => {
            return Err(ApiError::InvalidInput(format!("MCP probe failed: {}", e)));
        }
    };
    let items: Vec<McpProbeToolInfo> = tools
        .into_iter()
        .map(|t| McpProbeToolInfo {
            name: t.name,
            description: t.description,
        })
        .collect();
    ApiResponse::ok(McpProbeResponse { tools: items })
}
