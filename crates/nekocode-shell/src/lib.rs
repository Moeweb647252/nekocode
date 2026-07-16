use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, atomic::AtomicU32},
};

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::Middleware;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

pub mod config;
pub mod tool;

/// Maximum amount of unread output retained for one spawned shell. Older
/// unread lines are discarded with an explicit `truncated` signal in the next
/// fetch/wait response.
pub const MAX_BUFFERED_OUTPUT_BYTES: usize = 1_048_576;
pub const MAX_BUFFERED_OUTPUT_LINES: usize = 10_000;
pub const MAX_COMPLETED_SHELLS: usize = 64;

/// Buffered output and final status for one spawned shell.
#[derive(Default)]
pub struct ShellOutput {
    pub lines: VecDeque<String>,
    pub buffered_bytes: usize,
    pub truncated: bool,
    pub exit_code: Option<Option<i32>>,
}

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
    /// Bounded queue of unread output lines (stdout + stderr interleaved),
    /// plus the process's final exit status once it has stopped.
    pub output: Arc<Mutex<ShellOutput>>,
    pub input: mpsc::UnboundedSender<String>,
    pub cancellation_token: CancellationToken,
    pub is_running: Arc<std::sync::atomic::AtomicBool>,
    pub done: Arc<Notify>,
}

/// State carrier for the shell tool middleware. Holds the shared map of
/// running and recently completed shells (also inserted into the thread's
/// `Extensions` so the spawned-shell tools can reach it), the resolved
/// `ShellConfig`, and the monotonic id allocator. As a `Middleware` it
/// registers the shell tool family (`shell`, `spawn_shell`, `cancel_shell`,
/// `send_shell_input`, `fetch_shell_output`, `wait_shell_done`) in
/// `before_generate`.
pub struct Shell {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
    pub config: Arc<config::ShellConfig>,
    next_id: Arc<AtomicU32>,
}

impl Shell {
    /// Construct the middleware, inserting a shared clone of the shell-state
    /// map into `extensions` so the individual shell tools can access it.
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
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

    async fn shutdown(&self) -> Result<(), anyhow::Error> {
        let states: Vec<ShellTaskState> = self
            .shell_states
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        for state in &states {
            state.cancellation_token.cancel();
        }
        for state in states {
            let wait = async {
                loop {
                    let notified = state.done.notified();
                    if !state.is_running.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
            };
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), wait).await;
        }
        self.shell_states.clear();
        Ok(())
    }
}
