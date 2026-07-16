use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;

use dashmap::DashMap;
use nekocode_types::generate::{Message, Usage};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Lifecycle state of a single subagent's background run, driven by the
/// registry as the spawned task progresses. `is_ready` distinguishes terminal
/// states (from which a result can be read or waited-on unblocks) from the
/// still-running ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentRunState {
    Idle,
    Running,
    Finished,
    Error(String),
}

impl SubagentRunState {
    /// Whether `read_subagent`'s result is available to consume: true once the
    /// subagent reached a terminal state (`Finished` or `Error`).
    pub fn is_ready(&self) -> bool {
        matches!(
            self,
            SubagentRunState::Finished | SubagentRunState::Error(_)
        )
    }
}

fn run_state_name(s: &SubagentRunState) -> &'static str {
    match s {
        SubagentRunState::Idle => "idle",
        SubagentRunState::Running => "running",
        SubagentRunState::Finished => "finished",
        SubagentRunState::Error(_) => "error",
    }
}

/// The captured outcome of one finished subagent run: its token `Usage`, the
/// full ordered message list from the single turn, and whether `run_loop`
/// reported `finished`. Serialized back to the model by `read_subagent`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRunResult {
    pub usage: Usage,
    pub messages: Vec<Message>,
    pub finished: bool,
}

/// Per-subagent in-memory tracking record: its allocated id, current run state,
/// the spawned task's `JoinHandle`, a `Notify` for waiters to subscribe to
/// state changes, the result slot (filled on finish), and the per-agent
/// cancellation token. Lives inside a `SubagentRegistry` under the id key.
#[derive(Debug)]
pub struct SubagentState {
    pub agent_id: u64,
    pub run_state: SubagentRunState,
    pub task_handle: Option<JoinHandle<()>>,
    pub notify: Arc<Notify>,
    // Wrapped in `Arc` so the `RwLock` can be cloned out of the DashMap guard
    // before writing (see `set_finished`), mirroring `notify: Arc<Notify>`.
    pub result: Arc<RwLock<Option<SubagentRunResult>>>,
    /// Per-child cancellation token, fired by `abort_all_and_clear` (batch) and
    /// `abort_subagent` (single). This is the per-agent abort path; the
    /// cross-depth TREE-WIDE cancellation lives on `SubagentContext.run_cancel`
    /// (a single flag the root mints and every descendant shares), which the
    /// root's `on_turn_end` cancels so the whole spawn tree bails concurrently.
    /// This per-state `cancel` is the hard backstop for tasks that haven't
    /// yielded to `run_cancel` yet (and the path for explicit single-agent abort).
    pub cancel: Arc<tokio_util::sync::CancellationToken>,
}

impl SubagentState {
    /// Construct the tracking record for a freshly-spawned subagent: start it
    /// in `Running` with an unset result slot and its own cancel token.
    pub fn new(agent_id: u64) -> Self {
        Self {
            agent_id,
            run_state: SubagentRunState::Running,
            task_handle: None,
            notify: Arc::new(Notify::new()),
            result: Arc::new(RwLock::new(None)),
            cancel: Arc::new(tokio_util::sync::CancellationToken::new()),
        }
    }
}

/// In-memory registry of all subagents spawned by one parent thread, keyed by
/// the AtomicU64-allocated agent id. Published into `Agent.extensions` (typed
/// slot `TypeId::of::<Arc<SubagentRegistry>>()`) by the parent's subagent
/// middleware, and read by the six subagent tools. Lives only for the parent
/// turn — `on_turn_end` cancels the tree and clears it.
#[derive(Debug, Default)]
pub struct SubagentRegistry {
    states: DashMap<u64, SubagentState>,
    next_id: AtomicU64,
}

impl SubagentRegistry {
    /// Construct an empty per-parent registry. Used by the subagent middleware
    /// at parent activation and by tests.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new monotonic agent_id and insert a Running entry.
    /// Returns the allocated id. Called by spawn_subagent.
    pub fn allocate_running(&self) -> u64 {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        self.states.insert(id, SubagentState::new(id));
        id
    }

    /// Snapshot the run state of a subagent, defaulting to Idle if absent.
    pub fn run_state(&self, agent_id: u64) -> SubagentRunState {
        self.states
            .get(&agent_id)
            .map(|s| s.run_state.clone())
            .unwrap_or(SubagentRunState::Idle)
    }

    /// Snapshot the cancel token for a running subagent (for tests / cascade).
    pub fn cancel_token(&self, agent_id: u64) -> Option<Arc<tokio_util::sync::CancellationToken>> {
        self.states.get(&agent_id).map(|s| s.cancel.clone())
    }

    /// Mark a subagent as Running and store its task handle.
    pub fn set_running(&self, agent_id: u64, handle: JoinHandle<()>) {
        if let Some(mut s) = self.states.get_mut(&agent_id) {
            s.run_state = SubagentRunState::Running;
            s.task_handle = Some(handle);
        }
    }

    /// Mark a subagent as Finished, store its result, and wake waiters.
    pub fn set_finished(&self, agent_id: u64, result: SubagentRunResult) {
        // Clone the result slot and notify handle out of the DashMap guard,
        // then write + notify after dropping the guard. This avoids holding
        // the guard across a blocking lock acquisition AND closes the race
        // where abort() could remove the entry between a guard drop and a
        // re-acquire (which would skip notify_waiters).
        let (result_slot, notify) = if let Some(mut s) = self.states.get_mut(&agent_id) {
            s.run_state = SubagentRunState::Finished;
            s.task_handle = None;
            (s.result.clone(), s.notify.clone())
        } else {
            return;
        };
        // result is Arc<std::sync::RwLock<Option<..>>>; a sync write is fine
        // (no await while holding the guard) and, unlike tokio's
        // `blocking_write`, does not panic when called from an async context
        // (the runner awaits `run_loop` and Task 6 spawns it on a runtime).
        *result_slot.write().unwrap() = Some(result);
        notify.notify_waiters();
    }

    /// Mark a subagent as Error and wake waiters.
    pub fn set_error(&self, agent_id: u64, msg: String) {
        if let Some(mut s) = self.states.get_mut(&agent_id) {
            s.run_state = SubagentRunState::Error(msg);
            s.task_handle = None;
            s.notify.notify_waiters();
        }
    }

    /// Cooperatively cancel a subagent, wait for its cleanup, and remove it.
    pub async fn abort(&self, agent_id: u64) {
        if let Some((_, state)) = self.states.remove(&agent_id) {
            state.cancel.cancel();
            if let Some(mut handle) = state.task_handle
                && tokio::time::timeout(std::time::Duration::from_secs(5), &mut handle)
                    .await
                    .is_err()
            {
                handle.abort();
                let _ = handle.await;
            }
        }
    }

    /// Cooperatively cancel every running subagent, wait for cleanup, and clear
    /// the registry. A timed-out cleanup is force-aborted only after its token
    /// has been delivered.
    pub async fn abort_all_and_clear(&self) -> Vec<u64> {
        let ids = self.all_agent_ids();
        let mut states = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some((_, state)) = self.states.remove(id) {
                state.cancel.cancel();
                states.push(state);
            }
        }
        for state in states {
            if let Some(mut handle) = state.task_handle
                && tokio::time::timeout(std::time::Duration::from_secs(5), &mut handle)
                    .await
                    .is_err()
            {
                handle.abort();
                let _ = handle.await;
            }
        }
        ids
    }

    /// Whether the registry currently tracks `agent_id`.
    pub fn contains(&self, agent_id: u64) -> bool {
        self.states.contains_key(&agent_id)
    }

    /// Clone of the Notify for a subagent, so waiters can subscribe without
    /// holding a DashMap guard. Returns None if not tracked.
    pub fn notify(&self, agent_id: u64) -> Option<Arc<Notify>> {
        self.states.get(&agent_id).map(|s| s.notify.clone())
    }

    /// All agent ids tracked by this (per-parent) registry.
    pub fn all_agent_ids(&self) -> Vec<u64> {
        self.states.iter().map(|s| s.agent_id).collect()
    }

    /// Snapshot of a finished subagent's result (clone of the stored
    /// SubagentRunResult). Returns None if absent or not yet finished.
    pub fn result(&self, agent_id: u64) -> Option<SubagentRunResult> {
        let s = self.states.get(&agent_id)?;
        // std::sync read avoids holding the DashMap guard across an await and
        // is safe to call from async tool handlers (no panic on runtime threads).
        s.result.read().unwrap().clone()
    }
}

impl SubagentRunState {
    /// Lowercase name for JSON serialization in tool results.
    pub fn name(&self) -> &'static str {
        run_state_name(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_running_returns_monotonic_ids_and_running_state() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        assert!(id2 > id1, "ids must be monotonic");
        assert!(matches!(reg.run_state(id1), SubagentRunState::Running));
    }

    #[test]
    fn run_state_absent_defaults_to_idle() {
        let reg = SubagentRegistry::new();
        assert!(matches!(reg.run_state(999), SubagentRunState::Idle));
    }

    #[test]
    fn set_finished_stores_result() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        let result = SubagentRunResult {
            usage: Usage::default(),
            messages: Vec::new(),
            finished: true,
        };
        reg.set_finished(id, result.clone());
        assert!(matches!(reg.run_state(id), SubagentRunState::Finished));
        let got = reg.result(id);
        assert!(got.is_some(), "result should be stored");
    }

    #[test]
    fn set_error_marks_ready() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        reg.set_error(id, "boom".into());
        assert!(matches!(reg.run_state(id), SubagentRunState::Error(_)));
        assert!(reg.run_state(id).is_ready());
    }

    #[tokio::test]
    async fn abort_removes_entry() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        reg.abort(id).await;
        assert!(!reg.contains(id));
        assert!(matches!(reg.run_state(id), SubagentRunState::Idle));
    }

    #[tokio::test]
    async fn abort_all_and_clear_empties_and_returns_ids() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        let aborted = reg.abort_all_and_clear().await;
        assert_eq!(aborted.len(), 2, "both running entries aborted");
        assert!(!reg.contains(id1));
        assert!(!reg.contains(id2));
    }

    #[test]
    fn all_agent_ids_lists_tracked() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        let mut ids = reg.all_agent_ids();
        ids.sort();
        assert_eq!(ids, vec![id1, id2]);
    }

    #[test]
    fn notify_returns_handle_for_tracked() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        assert!(reg.notify(id).is_some());
        assert!(reg.notify(999).is_none());
    }
}
