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
                let client = match self.config.transport {
                    crate::config::Transport::Http => {
                        let Some(url) = self.config.server_url.clone() else {
                            warn!("MCP http transport configured but no serverUrl");
                            return None;
                        };
                        McpClient::connect_http(url, self.config.auth_headers.clone())
                    }
                    crate::config::Transport::Stdio => {
                        let Some(cmd) = self.config.server_command.clone() else {
                            warn!("MCP stdio transport configured but no serverCommand");
                            return None;
                        };
                        match McpClient::spawn(&cmd, &self.config.envs).await {
                            Ok(c) => c,
                            Err(e) => {
                                warn!("MCP spawn failed ({cmd}): {e}");
                                return None;
                            }
                        }
                    }
                };
                if let Err(e) = client.initialize().await {
                    warn!("MCP initialize failed: {e}");
                    client.kill().await;
                    return None;
                }
                Some(Arc::new(client))
            })
            .await
            .clone()
    }
}

/// Probe an MCP server with the given config: connect, initialize, list tools,
/// then tear down the connection. Returns the discovered tools. Used by the
/// settings UI's "Test connection" button to populate the tool list.
pub async fn probe(config: &McpConfig) -> anyhow::Result<Vec<client::McpToolInfo>> {
    let client = match config.transport {
        crate::config::Transport::Http => {
            let url = config.server_url.clone().ok_or_else(|| {
                anyhow::anyhow!("no serverUrl configured for http transport")
            })?;
            McpClient::connect_http(url, config.auth_headers.clone())
        }
        crate::config::Transport::Stdio => {
            let cmd = config.server_command.clone().ok_or_else(|| {
                anyhow::anyhow!("no serverCommand configured for stdio transport")
            })?;
            McpClient::spawn(&cmd, &config.envs).await?
        }
    };
    client.initialize().await?;
    let tools = client.list_tools().await?;
    client.kill().await;
    Ok(tools)
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