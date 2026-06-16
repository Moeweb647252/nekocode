pub mod client;
pub mod config;
pub mod tool;

use async_trait::async_trait;
use nekocode_core::middleware::Middleware;
use nekocode_types::tool::ToolRegistry;
use std::sync::Arc;
use tracing::warn;

use crate::client::McpClient;
use crate::config::McpConfig;

/// MCP middleware: connects to an MCP server, discovers its tools, and
/// registers the enabled ones into the agent's ToolRegistry so the LLM can
/// invoke them. Supports both stdio and Streamable HTTP transports.
pub struct McpMiddleware {
    pub config: Arc<McpConfig>,
    /// Lazily-initialized MCP client. `None` if the server is unreachable.
    client: tokio::sync::OnceCell<Option<Arc<McpClient>>>,
}

impl McpMiddleware {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config: Arc::new(config),
            client: tokio::sync::OnceCell::new(),
        }
    }

    /// Lazy connect + initialize. Cached for subsequent calls.
    async fn get_client(&self) -> Option<Arc<McpClient>> {
        self.client
            .get_or_init(|| async {
                // Prefer HTTP if configured; otherwise fall back to stdio.
                if let Some(url) = self.config.server_url.clone() {
                    let client = McpClient::connect_http(url);
                    if let Err(e) = client.initialize().await {
                        warn!("MCP HTTP initialize failed: {e}");
                        return None;
                    }
                    return Some(Arc::new(client));
                }
                let Some(cmd) = self.config.server_command.clone() else {
                    warn!("MCP middleware has no server_command or server_url configured");
                    return None;
                };
                match McpClient::spawn(&cmd, &self.config.envs).await {
                    Ok(client) => {
                        let client = Arc::new(client);
                        if let Err(e) = client.initialize().await {
                            warn!("MCP stdio initialize failed: {e}");
                            client.kill().await;
                            return None;
                        }
                        Some(client)
                    }
                    Err(e) => {
                        warn!("MCP spawn failed ({}): {e}", cmd);
                        None
                    }
                }
            })
            .await
            .clone()
    }
}

#[async_trait]
impl Middleware for McpMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        registry: &mut ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        let Some(client) = self.get_client().await else {
            return Ok(());
        };
        let tools = match client.list_tools().await {
            Ok(t) => t,
            Err(e) => {
                warn!("MCP tools/list failed: {e}");
                return Ok(());
            }
        };
        for info in tools {
            if !self.config.tools_enabled.get(&info.name).copied().unwrap_or(false) {
                continue;
            }
            let tool = tool::McpTool {
                client: client.clone(),
                name: info.name.clone(),
                description: info.description.clone().unwrap_or_default(),
                input_schema: info.input_schema.clone(),
            };
            registry.insert(info.name, Arc::new(tool));
        }
        Ok(())
    }
}