use std::sync::Arc;

use nekocode_core::middleware::Middleware;

pub mod config;
pub mod tool;

/// Middleware that registers the file read/write/edit tools into the tool
/// registry for the lifetime of a thread. Unlike the shell middleware there is
/// no shared mutable per-tool state — each tool only needs the config — so the
/// struct is just the config itself.
pub struct ToolMiddleware {
    pub config: Arc<config::FileConfig>,
}

impl ToolMiddleware {
    pub fn new(config: config::FileConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

#[async_trait::async_trait]
impl Middleware for ToolMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        registry: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        registry.insert(
            "read_file".into(),
            Arc::new(tool::ReadFileTool {
                config: self.config.clone(),
            }),
        );
        registry.insert(
            "write_file".into(),
            Arc::new(tool::WriteFileTool {
                config: self.config.clone(),
            }),
        );
        registry.insert(
            "edit_file".into(),
            Arc::new(tool::EditFileTool {
                config: self.config.clone(),
            }),
        );
        Ok(())
    }
}
