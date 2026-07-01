use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize},
    },
};

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::Middleware;
use sdd::AtomicOwned;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub mod config;
pub mod tool;

/// Per-spawned-shell bookkeeping. Keyed in [`Shell::shell_states`] by its
/// `shell_id` (an internal monotonic id, NOT the OS pid) to avoid PID-reuse
/// races and stay stable across the thread activation's lifetime.
#[derive(Clone)]
pub struct ShellTaskState {
    /// Internal monotonic id used as the public identifier for a shell.
    pub shell_id: u32,
    /// OS process id of the underlying child, reported for diagnostics only.
    pub pid: Option<u32>,
    pub command: String,
    /// Append-only buffer of output lines (stdout + stderr interleaved).
    pub output: Arc<AtomicOwned<boxcar::Vec<String>>>,
    /// Read cursor: smallest buffer index not yet returned by
    /// `fetch_shell_output`. Lets fetch be incremental and lossless.
    pub output_cursor: Arc<AtomicUsize>,
    pub input: mpsc::UnboundedSender<String>,
    pub cancellation_token: CancellationToken,
    pub is_running: Arc<std::sync::atomic::AtomicBool>,
}

pub struct Shell {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
    pub config: Arc<config::ShellConfig>,
    next_id: Arc<AtomicU32>,
}

impl Shell {
    pub fn new(extensions: Extensions, config: config::ShellConfig) -> Self {
        let shell_states = Arc::new(dashmap::DashMap::new());
        extensions.insert(shell_states.clone());
        Self {
            shell_states,
            config: Arc::new(config),
            next_id: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Allocate a fresh shell id.
    pub fn allocate_id(&self) -> u32 {
        self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl Middleware for Shell {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        registry: &mut nekocode_types::tool::ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        registry.insert(
            "shell".into(),
            std::sync::Arc::new(tool::OnceShellTool {
                config: self.config.clone(),
            }),
        );
        registry.insert(
            "spawn_shell".into(),
            std::sync::Arc::new(tool::SpawnShellTool {
                shell_states: self.shell_states.clone(),
                config: self.config.clone(),
                allocate_id: self.next_id.clone(),
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
        registry.insert(
            "wait_shell_done".into(),
            std::sync::Arc::new(tool::WaitShellDoneTool {
                shell_states: self.shell_states.clone(),
            }),
        );
        Ok(())
    }
}
