use std::sync::Arc;

use nekocode_core::middleware::Middleware;

pub mod config;
pub mod tool;

/// Middleware that registers the file read/write/edit tools into the tool
/// registry for the lifetime of a thread, plus a `set_title` tool that can
/// update the owning thread's title in the database.
///
/// The file tools only need the config, so they share an `Arc<FileConfig>`.
/// The `set_title` tool needs to mutate DB state, so the middleware also
/// carries a cloned `toasty::Db` handle and the owning thread id, both
/// injected when the thread is activated.
pub struct ToolMiddleware {
    pub config: Arc<config::FileConfig>,
    pub db: toasty::Db,
    pub thread_id: u64,
}

impl ToolMiddleware {
    pub fn new(config: config::FileConfig, db: toasty::Db, thread_id: u64) -> Self {
        Self {
            config: Arc::new(config),
            db,
            thread_id,
        }
    }
}

#[async_trait::async_trait]
impl Middleware for ToolMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        registry: &mut nekocode_types::tool::ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
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
        registry.insert(
            "set_title".into(),
            Arc::new(tool::SetTitleTool {
                db: self.db.clone(),
                thread_id: self.thread_id,
            }),
        );
        Ok(())
    }
}
