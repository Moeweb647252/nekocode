use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Notify;

/// Run state of a subthread, tracked in-memory only (not persisted across
/// server restarts; the DB holds the messages).
#[derive(Debug, Clone)]
pub enum SubthreadRunState {
    /// Created but never started via `start_subthread`.
    Idle,
    /// A background `run_loop` task is in flight.
    Running,
    /// The background task completed successfully.
    Finished,
    /// The background task errored; carries the error message.
    Error(String),
}

impl SubthreadRunState {
    /// "Ready" means the subthread has completed a `run_loop` and its message
    /// history is persisted and readable. `Idle` and `Running` are NOT ready.
    pub fn is_ready(&self) -> bool {
        matches!(self, SubthreadRunState::Finished | SubthreadRunState::Error(_))
    }
}

/// In-memory bookkeeping for one subthread. Keyed in [`SubthreadRegistry`] by
/// the subthread's `thread_id`.
///
/// Ownership: a `SubthreadRegistry` belongs to exactly one parent thread
/// (it lives in that parent's `Agent.extensions` as the typed slot
/// `TypeId::of::<Arc<SubthreadRegistry>>()`),
/// so `SubthreadState` does not carry a `parent_thread_id` — its container
/// already encodes the parent. This mirrors how `nekocode-shell`'s
/// `shell_states` is owned by the thread that spawned the shells.
#[derive(Debug)]
pub struct SubthreadState {
    pub thread_id: u64,
    pub run_state: SubthreadRunState,
    pub task_handle: Option<tokio::task::JoinHandle<()>>,
    pub notify: Arc<Notify>,
}

impl SubthreadState {
    pub fn new(thread_id: u64) -> Self {
        Self {
            thread_id,
            run_state: SubthreadRunState::Idle,
            task_handle: None,
            notify: Arc::new(Notify::new()),
        }
    }
}

/// Per-parent map of subthread run state. Owned by the parent thread's
/// `Agent.extensions` (typed slot `TypeId::of::<Arc<SubthreadRegistry>>()`),
/// shared via `Arc` with the parent's
/// `SubthreadMiddleware` and its nine tools. NOT a process-global singleton —
/// each parent thread has its own, so subthread state is owned by the parent
/// that spawned it.
#[derive(Debug, Default)]
pub struct SubthreadRegistry {
    states: DashMap<u64, SubthreadState>,
}

impl SubthreadRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an `Idle` subthread entry. Called by `spawn_subthread`.
    pub fn insert_idle(&self, thread_id: u64) {
        self.states
            .insert(thread_id, SubthreadState::new(thread_id));
    }

    /// Snapshot the run state of a subthread, defaulting to `Idle` if absent.
    pub fn run_state(&self, thread_id: u64) -> SubthreadRunState {
        self.states
            .get(&thread_id)
            .map(|s| s.run_state.clone())
            .unwrap_or(SubthreadRunState::Idle)
    }

    /// Mark a subthread as `Running` and store its task handle.
    /// Called by `start_subthread` right after spawning the background task.
    pub fn set_running(
        &self,
        thread_id: u64,
        task_handle: tokio::task::JoinHandle<()>,
    ) {
        if let Some(mut s) = self.states.get_mut(&thread_id) {
            s.run_state = SubthreadRunState::Running;
            s.task_handle = Some(task_handle);
        }
    }

    /// Mark a subthread as `Finished` and wake any waiters. Called from the
    /// background task's completion callback.
    pub fn set_finished(&self, thread_id: u64) {
        if let Some(mut s) = self.states.get_mut(&thread_id) {
            s.run_state = SubthreadRunState::Finished;
            s.task_handle = None;
            s.notify.notify_waiters();
        }
    }

    /// Mark a subthread as `Error` and wake any waiters.
    pub fn set_error(&self, thread_id: u64, msg: String) {
        if let Some(mut s) = self.states.get_mut(&thread_id) {
            s.run_state = SubthreadRunState::Error(msg);
            s.task_handle = None;
            s.notify.notify_waiters();
        }
    }

    /// Abort every running subthread's background task (best-effort) and clear
    /// the registry. Used when the owning parent thread is deleted. Returns
    /// the list of subthread ids that had in-flight tasks aborted, so callers
    /// (cascade delete) can evict those subthreads from `active_threads`.
    pub fn abort_all_and_clear(&self) -> Vec<u64> {
        let mut aborted = Vec::new();
        for entry in self.states.iter() {
            if let Some(handle) = &entry.task_handle {
                handle.abort();
                aborted.push(entry.thread_id);
            }
        }
        self.states.clear();
        aborted
    }

    /// Remove a single subthread's entry from the registry. Aborts its
    /// background task if still running. Used by `delete_subthread` to drop
    /// the in-memory bookkeeping after the DB rows are gone. No-op if the
    /// subthread isn't tracked here.
    pub fn remove(&self, thread_id: u64) {
        if let Some((_, state)) = self.states.remove(&thread_id)
            && let Some(handle) = state.task_handle
        {
            handle.abort();
        }
    }

    /// Whether the registry currently tracks `thread_id`.
    pub fn contains(&self, thread_id: u64) -> bool {
        self.states.contains_key(&thread_id)
    }

    /// Clone of the `Notify` for a subthread, so waiters can subscribe without
    /// holding a DashMap guard. Returns `None` if the subthread isn't tracked.
    pub fn notify(&self, thread_id: u64) -> Option<Arc<Notify>> {
        self.states.get(&thread_id).map(|s| s.notify.clone())
    }

    /// All subthread ids tracked by this (per-parent) registry. Used by
    /// `wait_all_subthreads` default scope.
    pub fn all_thread_ids(&self) -> Vec<u64> {
        self.states.iter().map(|s| s.thread_id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_run_state_idle() {
        let reg = SubthreadRegistry::new();
        reg.insert_idle(1);
        assert!(matches!(reg.run_state(1), SubthreadRunState::Idle));
    }

    #[test]
    fn run_state_absent_defaults_to_idle() {
        let reg = SubthreadRegistry::new();
        assert!(matches!(reg.run_state(999), SubthreadRunState::Idle));
    }

    #[test]
    fn set_finished_wakes_waiters() {
        // The Notify wake is observable via notified() resolving; verify the
        // state transition at minimum.
        let reg = SubthreadRegistry::new();
        reg.insert_idle(1);
        reg.set_finished(1);
        assert!(reg.run_state(1).is_ready());
    }

    #[test]
    fn all_thread_ids_lists_tracked() {
        let reg = SubthreadRegistry::new();
        reg.insert_idle(1);
        reg.insert_idle(2);
        let mut ids = reg.all_thread_ids();
        ids.sort();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn abort_all_and_clear_drops_entries() {
        let reg = SubthreadRegistry::new();
        reg.insert_idle(1);
        reg.insert_idle(2);
        let _ = reg.abort_all_and_clear();
        // After clear, states are gone — run_state falls back to Idle.
        assert!(matches!(reg.run_state(1), SubthreadRunState::Idle));
        assert!(matches!(reg.run_state(2), SubthreadRunState::Idle));
    }
}
