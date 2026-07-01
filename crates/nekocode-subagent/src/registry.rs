use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::RwLock;

use dashmap::DashMap;
use nekocode_types::generate::{Message, Usage};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentRunState {
    Idle,
    Running,
    Finished,
    Error(String),
}

impl SubagentRunState {
    pub fn is_ready(&self) -> bool {
        matches!(self, SubagentRunState::Finished | SubagentRunState::Error(_))
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRunResult {
    pub usage: Usage,
    pub messages: Vec<Message>,
    pub finished: bool,
}

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

#[derive(Debug, Default)]
pub struct SubagentRegistry {
    states: DashMap<u64, SubagentState>,
    next_id: AtomicU64,
}

impl SubagentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new monotonic agent_id and insert a Running entry.
    /// Returns the allocated id. Called by spawn_subagent.
    pub fn allocate_running(&self) -> u64 {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
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

    /// Abort a subagent's background task (if running) and remove its entry.
    pub fn abort(&self, agent_id: u64) {
        if let Some((_, state)) = self.states.remove(&agent_id)
            && let Some(handle) = state.task_handle
        {
            handle.abort();
        }
    }

    /// Abort every running subagent's background task and clear the registry.
    /// Returns the ids of all tracked subagents (so cascade-delete can evict
    /// them), aborting in-flight task handles where present.
    pub fn abort_all_and_clear(&self) -> Vec<u64> {
        let mut aborted = Vec::new();
        // Cancel every direct child's per-state token so their run_subagent select!
        // bails at its next await. (Note: cross-DEPTH tree-wide cancellation is
        // driven by the shared `run_cancel` on SubagentContext, cancelled once
        // by the root's on_turn_end — every descendant subscribes to the same
        // flag and ends concurrently. This per-state cancel handles direct
        // children/abort_subagent and is the backstop for tasks not yet
        // polled onto run_cancel.) JoinHandle::abort() below is the hard
        // guarantee for tasks that don't yield to either token promptly.
        for entry in self.states.iter() {
            entry.cancel.cancel();
            aborted.push(entry.agent_id);
        }
        // Abort handles, then clear.
        for entry in self.states.iter() {
            if let Some(handle) = &entry.task_handle {
                handle.abort();
            }
        }
        self.states.clear();
        aborted
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

    #[test]
    fn abort_removes_entry() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        reg.abort(id);
        assert!(!reg.contains(id));
        assert!(matches!(reg.run_state(id), SubagentRunState::Idle));
    }

    #[test]
    fn abort_all_and_clear_empties_and_returns_ids() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        let aborted = reg.abort_all_and_clear();
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
