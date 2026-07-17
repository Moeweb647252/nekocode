use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use futures_util::FutureExt;
use nekocode_types::generate::{Message, Turn, Usage};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// The captured outcome of one subagent run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRunResult {
    pub usage: Usage,
    pub messages: Vec<Message>,
    pub finished: bool,
}

impl SubagentRunResult {
    pub fn from_turn(turn: Turn) -> Self {
        Self {
            usage: turn.usage,
            messages: turn.messages,
            finished: turn.finished,
        }
    }

    fn empty_partial() -> Self {
        Self {
            usage: Usage::default(),
            messages: Vec::new(),
            finished: false,
        }
    }
}

/// An atomic, externally observable snapshot of one subagent run.
///
/// Results live inside the terminal variants, so callers can never observe a
/// terminal status without its corresponding complete or partial result.
#[derive(Debug, Clone)]
pub enum SubagentSnapshot {
    Running,
    Finished(SubagentRunResult),
    Error {
        error: String,
        partial: SubagentRunResult,
    },
}

impl SubagentSnapshot {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished(_) => "finished",
            Self::Error { .. } => "error",
        }
    }

    pub fn is_ready(&self) -> bool {
        !matches!(self, Self::Running)
    }

    pub fn result(&self) -> Option<&SubagentRunResult> {
        match self {
            Self::Running => None,
            Self::Finished(result) => Some(result),
            Self::Error { partial, .. } => Some(partial),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Error { error, .. } => Some(error),
            Self::Running | Self::Finished(_) => None,
        }
    }
}

/// Terminal value returned by the subagent runner to the registry-owned task.
#[derive(Debug)]
pub enum SubagentRunOutcome {
    Finished(SubagentRunResult),
    Error {
        error: String,
        partial: SubagentRunResult,
    },
}

impl From<SubagentRunOutcome> for SubagentSnapshot {
    fn from(value: SubagentRunOutcome) -> Self {
        match value {
            SubagentRunOutcome::Finished(result) => Self::Finished(result),
            SubagentRunOutcome::Error { error, partial } => Self::Error { error, partial },
        }
    }
}

/// Context supplied by [`SubagentRegistry::spawn`] to the registered task.
#[derive(Debug, Clone)]
pub struct SubagentTask {
    pub agent_id: u64,
    pub cancel: CancellationToken,
}

#[derive(Debug)]
struct SubagentEntry {
    snapshot: RwLock<SubagentSnapshot>,
    task_handle: Mutex<Option<JoinHandle<()>>>,
    cancel: CancellationToken,
}

impl SubagentEntry {
    fn new() -> Self {
        Self {
            snapshot: RwLock::new(SubagentSnapshot::Running),
            task_handle: Mutex::new(None),
            cancel: CancellationToken::new(),
        }
    }

    fn snapshot(&self) -> SubagentSnapshot {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn complete(&self, outcome: SubagentRunOutcome) {
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = outcome.into();
        self.task_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    fn take_handle(&self) -> Option<JoinHandle<()>> {
        self.task_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentNotFound(pub u64);

impl std::fmt::Display for SubagentNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent {} not found", self.0)
    }
}

impl std::error::Error for SubagentNotFound {}

#[derive(Debug)]
pub enum WaitAnyOutcome {
    Ready {
        agent_id: u64,
        snapshot: SubagentSnapshot,
    },
    Timeout {
        pending: Vec<u64>,
    },
}

#[derive(Debug)]
pub enum WaitAllOutcome {
    Ready {
        results: Vec<(u64, SubagentSnapshot)>,
    },
    Timeout {
        ready: Vec<u64>,
        pending: Vec<u64>,
    },
}

/// In-memory owner of every subagent spawned during one parent turn.
#[derive(Debug)]
pub struct SubagentRegistry {
    states: DashMap<u64, Arc<SubagentEntry>>,
    next_id: AtomicU64,
    change_version: AtomicU64,
    changes: watch::Sender<u64>,
    accepting_spawns: Mutex<bool>,
}

impl Default for SubagentRegistry {
    fn default() -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            states: DashMap::new(),
            next_id: AtomicU64::new(0),
            change_version: AtomicU64::new(0),
            changes,
            accepting_spawns: Mutex::new(true),
        }
    }
}

impl SubagentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reopen the registry for a new parent turn after the previous turn's
    /// cleanup closed it to concurrent spawns.
    pub fn begin_turn(&self) {
        *self
            .accepting_spawns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }

    /// Allocate, register, and launch a subagent task as one operation.
    ///
    /// The start channel is released only after the task handle is stored in
    /// the entry. An immediately-ready future therefore cannot publish a
    /// terminal snapshot and subsequently be overwritten as Running.
    pub fn spawn<F, Fut>(self: &Arc<Self>, run: F) -> Result<u64, anyhow::Error>
    where
        F: FnOnce(SubagentTask) -> Fut + Send + 'static,
        Fut: Future<Output = SubagentRunOutcome> + Send + 'static,
    {
        let accepting = self
            .accepting_spawns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*accepting {
            anyhow::bail!("subagent registry is closing");
        }

        let agent_id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = Arc::new(SubagentEntry::new());
        self.states.insert(agent_id, entry.clone());

        let (start_tx, start_rx) = oneshot::channel();
        let registry = Arc::downgrade(self);
        let task_entry = entry.clone();
        let task_cancel = entry.cancel.clone();
        let handle = tokio::spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            let task = SubagentTask {
                agent_id,
                cancel: task_cancel,
            };
            let outcome = AssertUnwindSafe(async move { run(task).await })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| SubagentRunOutcome::Error {
                    error: panic_message(panic),
                    partial: SubagentRunResult::empty_partial(),
                });
            task_entry.complete(outcome);
            if let Some(registry) = registry.upgrade() {
                registry.notify_change();
            }
        });
        *entry
            .task_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(handle);
        let _ = start_tx.send(());
        self.notify_change();
        drop(accepting);
        Ok(agent_id)
    }

    pub fn snapshot(&self, agent_id: u64) -> Option<SubagentSnapshot> {
        self.states.get(&agent_id).map(|entry| entry.snapshot())
    }

    pub fn contains(&self, agent_id: u64) -> bool {
        self.states.contains_key(&agent_id)
    }

    pub fn all_agent_ids(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self.states.iter().map(|entry| *entry.key()).collect();
        ids.sort_unstable();
        ids
    }

    pub fn running_agent_ids(&self) -> Vec<u64> {
        self.all_agent_ids()
            .into_iter()
            .filter(|id| matches!(self.snapshot(*id), Some(SubagentSnapshot::Running)))
            .collect()
    }

    pub async fn wait_any(
        &self,
        ids: &[u64],
        timeout: Duration,
    ) -> Result<WaitAnyOutcome, SubagentNotFound> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut changes = self.changes.subscribe();
        loop {
            for &agent_id in ids {
                let snapshot = self.snapshot(agent_id).ok_or(SubagentNotFound(agent_id))?;
                if snapshot.is_ready() {
                    return Ok(WaitAnyOutcome::Ready { agent_id, snapshot });
                }
            }
            if tokio::time::timeout_at(deadline, changes.changed())
                .await
                .is_err()
            {
                return Ok(WaitAnyOutcome::Timeout {
                    pending: ids.to_vec(),
                });
            }
        }
    }

    pub async fn wait_all(
        &self,
        ids: &[u64],
        timeout: Duration,
    ) -> Result<WaitAllOutcome, SubagentNotFound> {
        if ids.is_empty() {
            return Ok(WaitAllOutcome::Ready {
                results: Vec::new(),
            });
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let mut changes = self.changes.subscribe();
        loop {
            let mut results = Vec::with_capacity(ids.len());
            let mut ready = Vec::new();
            let mut pending = Vec::new();
            for &agent_id in ids {
                let snapshot = self.snapshot(agent_id).ok_or(SubagentNotFound(agent_id))?;
                if snapshot.is_ready() {
                    ready.push(agent_id);
                } else {
                    pending.push(agent_id);
                }
                results.push((agent_id, snapshot));
            }
            if pending.is_empty() {
                return Ok(WaitAllOutcome::Ready { results });
            }
            if tokio::time::timeout_at(deadline, changes.changed())
                .await
                .is_err()
            {
                return Ok(WaitAllOutcome::Timeout { ready, pending });
            }
        }
    }

    /// Cooperatively cancel a subagent, wait for cleanup, and remove it.
    pub async fn abort(&self, agent_id: u64) -> bool {
        let Some((_, entry)) = self.states.remove(&agent_id) else {
            return false;
        };
        self.notify_change();
        entry.cancel.cancel();
        await_task(entry.take_handle()).await;
        true
    }

    /// Close spawning, cancel every tracked subagent, and wait for cleanup.
    pub async fn abort_all_and_clear(&self) -> Vec<u64> {
        let entries = {
            let mut accepting = self
                .accepting_spawns
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *accepting = false;
            let ids = self.all_agent_ids();
            ids.into_iter()
                .filter_map(|id| self.states.remove(&id).map(|(_, entry)| (id, entry)))
                .collect::<Vec<_>>()
        };
        self.notify_change();
        for (_, entry) in &entries {
            entry.cancel.cancel();
        }
        for (_, entry) in &entries {
            await_task(entry.take_handle()).await;
        }
        entries.into_iter().map(|(id, _)| id).collect()
    }

    fn notify_change(&self) {
        let version = self.change_version.fetch_add(1, Ordering::Relaxed) + 1;
        self.changes.send_replace(version);
    }
}

async fn await_task(handle: Option<JoinHandle<()>>) {
    let Some(mut handle) = handle else {
        return;
    };
    if tokio::time::timeout(Duration::from_secs(5), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
        let _ = handle.await;
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        format!("subagent task panicked: {message}")
    } else if let Some(message) = panic.downcast_ref::<String>() {
        format!("subagent task panicked: {message}")
    } else {
        "subagent task panicked".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished_result() -> SubagentRunResult {
        SubagentRunResult {
            usage: Usage::default(),
            messages: Vec::new(),
            finished: true,
        }
    }

    #[tokio::test]
    async fn instant_completion_never_regresses_to_running() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry
            .spawn(|_| async { SubagentRunOutcome::Finished(finished_result()) })
            .unwrap();
        let outcome = registry
            .wait_any(&[id], Duration::from_secs(1))
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            WaitAnyOutcome::Ready {
                snapshot: SubagentSnapshot::Finished(_),
                ..
            }
        ));
        assert!(matches!(
            registry.snapshot(id),
            Some(SubagentSnapshot::Finished(_))
        ));
    }

    #[tokio::test]
    async fn terminal_snapshot_contains_its_result_atomically() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry
            .spawn(|_| async { SubagentRunOutcome::Finished(finished_result()) })
            .unwrap();
        registry
            .wait_any(&[id], Duration::from_secs(1))
            .await
            .unwrap();
        let snapshot = registry.snapshot(id).unwrap();
        assert!(snapshot.is_ready());
        assert!(snapshot.result().is_some());
    }

    #[tokio::test]
    async fn error_snapshot_keeps_partial_result() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry
            .spawn(|_| async {
                SubagentRunOutcome::Error {
                    error: "boom".into(),
                    partial: SubagentRunResult {
                        usage: Usage::default(),
                        messages: Vec::new(),
                        finished: false,
                    },
                }
            })
            .unwrap();
        registry
            .wait_any(&[id], Duration::from_secs(1))
            .await
            .unwrap();
        let snapshot = registry.snapshot(id).unwrap();
        assert_eq!(snapshot.error(), Some("boom"));
        assert!(!snapshot.result().unwrap().finished);
    }

    #[tokio::test]
    async fn wait_all_observes_fast_completions_without_lost_wakeup() {
        let registry = Arc::new(SubagentRegistry::new());
        let first = registry
            .spawn(|_| async { SubagentRunOutcome::Finished(finished_result()) })
            .unwrap();
        let second = registry
            .spawn(|_| async { SubagentRunOutcome::Finished(finished_result()) })
            .unwrap();
        let outcome = registry
            .wait_all(&[first, second], Duration::from_secs(1))
            .await
            .unwrap();
        assert!(matches!(outcome, WaitAllOutcome::Ready { .. }));
    }

    #[tokio::test]
    async fn abort_during_start_or_run_cancels_and_removes_task() {
        let registry = Arc::new(SubagentRegistry::new());
        let (observed_tx, observed_rx) = oneshot::channel();
        let id = registry
            .spawn(move |task| async move {
                task.cancel.cancelled().await;
                let _ = observed_tx.send(());
                SubagentRunOutcome::Error {
                    error: "cancelled".into(),
                    partial: SubagentRunResult::empty_partial(),
                }
            })
            .unwrap();
        assert!(registry.abort(id).await);
        observed_rx.await.unwrap();
        assert!(registry.snapshot(id).is_none());
    }

    #[tokio::test]
    async fn abort_all_closes_registry_until_next_turn() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry
            .spawn(|task| async move {
                task.cancel.cancelled().await;
                SubagentRunOutcome::Error {
                    error: "cancelled".into(),
                    partial: SubagentRunResult::empty_partial(),
                }
            })
            .unwrap();
        assert_eq!(registry.abort_all_and_clear().await, vec![id]);
        assert!(
            registry
                .spawn(|_| async { SubagentRunOutcome::Finished(finished_result()) })
                .is_err()
        );
        registry.begin_turn();
        assert!(
            registry
                .spawn(|_| async { SubagentRunOutcome::Finished(finished_result()) })
                .is_ok()
        );
    }
}
