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
        matches!(
            self,
            SubthreadRunState::Finished | SubthreadRunState::Error(_)
        )
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
    pub cancellation_token: tokio_util::sync::CancellationToken,
}

impl SubthreadState {
    /// Construct a fresh entry for `thread_id` in the `Idle` run state, with
    /// no background task yet attached and its own `Notify`.
    pub fn new(thread_id: u64) -> Self {
        Self {
            thread_id,
            run_state: SubthreadRunState::Idle,
            task_handle: None,
            notify: Arc::new(Notify::new()),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
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
    /// Construct an empty per-parent registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an `Idle` subthread entry when it is not already tracked. Called
    /// by `spawn_subthread`; preserving an existing entry avoids resetting a
    /// run that is already in flight.
    pub fn insert_idle(&self, thread_id: u64) {
        self.states
            .entry(thread_id)
            .or_insert_with(|| SubthreadState::new(thread_id));
    }

    /// Snapshot the run state of a subthread, defaulting to `Idle` if absent.
    pub fn run_state(&self, thread_id: u64) -> SubthreadRunState {
        self.states
            .get(&thread_id)
            .map(|s| s.run_state.clone())
            .unwrap_or(SubthreadRunState::Idle)
    }

    /// Atomically reserve a subthread for a background run. This also restores
    /// a registry entry for subthreads persisted before the parent agent was
    /// reactivated.
    pub fn try_start(&self, thread_id: u64) -> bool {
        match self.states.entry(thread_id) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                if matches!(entry.get().run_state, SubthreadRunState::Running) {
                    false
                } else {
                    let state = entry.get_mut();
                    state.run_state = SubthreadRunState::Running;
                    state.task_handle = None;
                    state.cancellation_token = tokio_util::sync::CancellationToken::new();
                    true
                }
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(SubthreadState {
                    thread_id,
                    run_state: SubthreadRunState::Running,
                    task_handle: None,
                    notify: Arc::new(Notify::new()),
                    cancellation_token: tokio_util::sync::CancellationToken::new(),
                });
                true
            }
        }
    }

    pub fn cancellation_token(
        &self,
        thread_id: u64,
    ) -> Option<tokio_util::sync::CancellationToken> {
        self.states
            .get(&thread_id)
            .map(|state| state.cancellation_token.clone())
    }

    /// Associate the task handle with a previously reserved run. If the run
    /// completed before the handle was attached, retain the terminal state and
    /// discard the now-completed handle instead of regressing it to `Running`.
    pub fn attach_task_handle(&self, thread_id: u64, task_handle: tokio::task::JoinHandle<()>) {
        if let Some(mut state) = self.states.get_mut(&thread_id)
            && matches!(state.run_state, SubthreadRunState::Running)
        {
            state.task_handle = Some(task_handle);
        } else {
            // The reservation was removed after its cancellation token fired.
            // Dropping the handle detaches only the cooperative cleanup task;
            // aborting it here would bypass Agent cleanup.
            drop(task_handle);
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

    /// Cooperatively cancel every running subthread, wait for cleanup, and
    /// clear the registry. A timed-out cleanup is force-aborted only after its
    /// cancellation token has been delivered.
    pub async fn abort_all_and_clear(&self) -> Vec<u64> {
        let ids = self.all_thread_ids();
        let mut states = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some((_, state)) = self.states.remove(id) {
                state.cancellation_token.cancel();
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

    /// Remove a single subthread's entry from the registry. Aborts its
    /// background task if still running. Used by `delete_subthread` to drop
    /// the in-memory bookkeeping after the DB rows are gone. No-op if the
    /// subthread isn't tracked here.
    pub async fn remove(&self, thread_id: u64) {
        if let Some((_, state)) = self.states.remove(&thread_id) {
            state.cancellation_token.cancel();
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
    fn try_start_restores_missing_entry_and_rejects_a_second_run() {
        let reg = SubthreadRegistry::new();
        assert!(reg.try_start(7));
        assert!(matches!(reg.run_state(7), SubthreadRunState::Running));
        assert!(!reg.try_start(7));
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

    #[tokio::test]
    async fn abort_all_and_clear_drops_entries() {
        let reg = SubthreadRegistry::new();
        reg.insert_idle(1);
        reg.insert_idle(2);
        let _ = reg.abort_all_and_clear().await;
        // After clear, states are gone — run_state falls back to Idle.
        assert!(matches!(reg.run_state(1), SubthreadRunState::Idle));
        assert!(matches!(reg.run_state(2), SubthreadRunState::Idle));
    }
}
