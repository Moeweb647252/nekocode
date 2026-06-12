use std::{
    any::Any,
    sync::{Arc, atomic::AtomicBool},
};

use nekocode_core::middleware::Middleware;
use sdd::AtomicOwned;
use tokio::sync::mpsc;

pub mod config;
pub mod tool;

pub struct ShellTaskState {
    pub output: Arc<AtomicOwned<boxcar::Vec<String>>>,
    pub input: mpsc::UnboundedSender<String>,
    pub cancellation_token: tokio_util::sync::CancellationToken,
    pub is_running: Arc<AtomicBool>,
}

pub struct Shell {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

impl Shell {
    pub fn new(extensions: Arc<dashmap::DashMap<String, Box<dyn Any + Send + Sync>>>) -> Self {
        let shell_states = Arc::new(dashmap::DashMap::new());
        extensions.insert("shell".into(), Box::new(shell_states.clone()));
        Self {
            shell_states: shell_states,
        }
    }
}

#[async_trait::async_trait]
impl Middleware for Shell {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        registry: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        registry.insert("shell".into(), std::sync::Arc::new(tool::OnceShellTool {}));
        registry.insert(
            "spawn_shell".into(),
            std::sync::Arc::new(tool::SpawnShellTool {
                shell_states: self.shell_states.clone(),
            }),
        );
        registry.insert(
            "cancel_shell".into(),
            std::sync::Arc::new(tool::CancelShellTool {
                shell_states: self.shell_states.clone(),
            }),
        );
        registry.insert(
            "send_shell_input".into(),
            std::sync::Arc::new(tool::SendShellInputTool {
                shell_states: self.shell_states.clone(),
            }),
        );
        registry.insert(
            "fetch_shell_output".into(),
            std::sync::Arc::new(tool::FetchShellOutputTool {
                shell_states: self.shell_states.clone(),
            }),
        );
        Ok(())
    }
}
