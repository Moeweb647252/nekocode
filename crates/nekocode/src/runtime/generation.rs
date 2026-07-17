use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use dashmap::DashMap;
use nekocode_core::agent::AgentEvent;
use nekocode_types::generate::Usage;
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

use super::RuntimeError;

pub(crate) type ThreadId = u64;

#[derive(Debug, Clone)]
pub(crate) enum GenerationTerminal {
    Finished(Usage),
    Interrupted,
    Error(String),
}

#[derive(Clone)]
pub(crate) enum GenerationEvent {
    Delta(AgentEvent),
    Terminal(GenerationTerminal),
}

struct GenerationState {
    events: Mutex<Vec<AgentEvent>>,
    broadcast: broadcast::Sender<()>,
    cancellation: CancellationToken,
    terminal: watch::Sender<Option<GenerationTerminal>>,
}

impl GenerationState {
    fn new(cancellation: CancellationToken) -> Self {
        let (broadcast, _) = broadcast::channel(32);
        let (terminal, _) = watch::channel(None);
        Self {
            events: Mutex::new(Vec::new()),
            broadcast,
            cancellation,
            terminal,
        }
    }

    fn publish(&self, event: AgentEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
        let _ = self.broadcast.send(());
    }

    fn finish(&self, terminal: GenerationTerminal) {
        let changed = self.terminal.send_if_modified(|current| {
            if current.is_some() {
                false
            } else {
                *current = Some(terminal);
                true
            }
        });
        if changed {
            let _ = self.broadcast.send(());
        }
    }

    fn terminal(&self) -> Option<GenerationTerminal> {
        self.terminal.borrow().clone()
    }
}

/// Private registry for active persisted generations. A lease is the only
/// object allowed to own a reservation and publish its terminal result.
#[derive(Default)]
pub(crate) struct GenerationRegistry {
    runs: DashMap<ThreadId, Arc<GenerationState>>,
}

impl GenerationRegistry {
    pub(crate) fn reserve(
        self: &Arc<Self>,
        thread_id: ThreadId,
        cancellation: CancellationToken,
    ) -> Result<GenerationLease, RuntimeError> {
        let state = Arc::new(GenerationState::new(cancellation));
        match self.runs.entry(thread_id) {
            dashmap::mapref::entry::Entry::Occupied(_) => Err(RuntimeError::ThreadGenerating),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(state.clone());
                Ok(GenerationLease {
                    registry: Arc::downgrade(self),
                    thread_id,
                    state,
                    released: AtomicBool::new(false),
                })
            }
        }
    }

    pub(crate) fn subscribe(
        &self,
        thread_id: ThreadId,
    ) -> Result<GenerationSubscription, RuntimeError> {
        let state = self
            .runs
            .get(&thread_id)
            .map(|entry| entry.value().clone())
            .ok_or(RuntimeError::GenerationNotFound(thread_id))?;
        Ok(GenerationSubscription::new(state))
    }

    pub(crate) fn contains(&self, thread_id: ThreadId) -> bool {
        self.runs.contains_key(&thread_id)
    }

    pub(crate) fn cancel(&self, thread_id: ThreadId) -> Result<(), RuntimeError> {
        let state = self
            .runs
            .get(&thread_id)
            .map(|entry| entry.value().clone())
            .ok_or(RuntimeError::ThreadNotActivated)?;
        state.cancellation.cancel();
        Ok(())
    }

    fn release(&self, thread_id: ThreadId, expected: &Arc<GenerationState>) {
        let is_current = self
            .runs
            .get(&thread_id)
            .map(|current| Arc::ptr_eq(current.value(), expected))
            .unwrap_or(false);
        if is_current {
            self.runs.remove(&thread_id);
        }
    }
}

/// Unique owner for one registry reservation. Releasing is pointer-checked so
/// a stale owner can never erase a newer generation for the same thread.
pub(crate) struct GenerationLease {
    registry: Weak<GenerationRegistry>,
    thread_id: ThreadId,
    state: Arc<GenerationState>,
    released: AtomicBool,
}

impl GenerationLease {
    pub(crate) fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.state.cancellation.clone()
    }

    pub(crate) fn publish(&self, event: AgentEvent) {
        self.state.publish(event);
    }

    pub(crate) fn subscribe(&self) -> GenerationSubscription {
        GenerationSubscription::new(self.state.clone())
    }

    pub(crate) fn finish(&self, terminal: GenerationTerminal) {
        self.state.finish(terminal);
    }

    pub(crate) fn release(&self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(registry) = self.registry.upgrade() {
            registry.release(self.thread_id, &self.state);
        }
    }
}

impl Drop for GenerationLease {
    fn drop(&mut self) {
        if !self.released.load(Ordering::Acquire) {
            if self.state.terminal().is_none() {
                self.state.finish(GenerationTerminal::Error(
                    "generation ended before publishing a terminal result".into(),
                ));
            }
            self.release();
        }
    }
}

/// A replaying subscriber. The in-memory event buffer is authoritative; the
/// broadcast channel merely wakes live subscribers, so lagged receivers can
/// never lose deltas and terminal output always follows the final delta.
pub(crate) struct GenerationSubscription {
    state: Arc<GenerationState>,
    cursor: usize,
    broadcast: broadcast::Receiver<()>,
    terminal: watch::Receiver<Option<GenerationTerminal>>,
}

impl GenerationSubscription {
    fn new(state: Arc<GenerationState>) -> Self {
        Self {
            broadcast: state.broadcast.subscribe(),
            terminal: state.terminal.subscribe(),
            state,
            cursor: 0,
        }
    }

    pub(crate) async fn next(&mut self) -> GenerationEvent {
        loop {
            if let Some(event) = self
                .state
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(self.cursor)
                .cloned()
            {
                self.cursor += 1;
                return GenerationEvent::Delta(event);
            }
            if let Some(terminal) = self.terminal.borrow().clone() {
                return GenerationEvent::Terminal(terminal);
            }
            tokio::select! {
                received = self.broadcast.recv() => {
                    if matches!(received, Err(broadcast::error::RecvError::Closed)) {
                        return GenerationEvent::Terminal(GenerationTerminal::Error(
                            "generation event channel closed without a terminal result".into(),
                        ));
                    }
                }
                changed = self.terminal.changed() => {
                    if changed.is_err() {
                        return GenerationEvent::Terminal(GenerationTerminal::Error(
                            "generation ended without a terminal result".into(),
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(index: usize) -> AgentEvent {
        AgentEvent {
            index,
            data: nekocode_core::agent::AgentEventType::StreamEvent(
                nekocode_types::generate::StreamEvent {
                    data: nekocode_types::generate::StreamEventData::TurnEnd,
                    created_at: jiff::Timestamp::now(),
                },
            ),
        }
    }

    #[tokio::test]
    async fn stale_lease_does_not_release_a_replacement() {
        let registry = Arc::new(GenerationRegistry::default());
        let first = registry
            .reserve(1, CancellationToken::new())
            .expect("first lease");
        first.finish(GenerationTerminal::Finished(Usage::default()));
        first.release();
        let second = registry
            .reserve(1, CancellationToken::new())
            .expect("replacement lease");
        drop(first);
        assert!(registry.contains(1));
        second.release();
    }

    #[tokio::test]
    async fn subscription_replays_deltas_before_terminal() {
        let registry = Arc::new(GenerationRegistry::default());
        let lease = registry.reserve(1, CancellationToken::new()).unwrap();
        lease.publish(event(0));
        lease.publish(event(1));
        lease.finish(GenerationTerminal::Interrupted);
        let mut subscription = lease.subscribe();
        assert!(matches!(
            subscription.next().await,
            GenerationEvent::Delta(_)
        ));
        assert!(matches!(
            subscription.next().await,
            GenerationEvent::Delta(_)
        ));
        assert!(matches!(
            subscription.next().await,
            GenerationEvent::Terminal(GenerationTerminal::Interrupted)
        ));
    }

    #[tokio::test]
    async fn concurrent_reservations_have_one_owner() {
        let registry = Arc::new(GenerationRegistry::default());
        let first_registry = registry.clone();
        let second_registry = registry.clone();
        let first =
            tokio::spawn(async move { first_registry.reserve(7, CancellationToken::new()) });
        let second =
            tokio::spawn(async move { second_registry.reserve(7, CancellationToken::new()) });
        let first = first.await.unwrap();
        let second = second.await.unwrap();
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    }

    #[tokio::test]
    async fn slow_subscription_recovers_every_buffered_delta() {
        let registry = Arc::new(GenerationRegistry::default());
        let lease = registry.reserve(1, CancellationToken::new()).unwrap();
        let mut subscription = lease.subscribe();
        for index in 0..100 {
            lease.publish(event(index));
        }
        lease.finish(GenerationTerminal::Finished(Usage::default()));
        for index in 0..100 {
            let GenerationEvent::Delta(received) = subscription.next().await else {
                panic!("expected delta {index}");
            };
            assert_eq!(received.index, index);
        }
        assert!(matches!(
            subscription.next().await,
            GenerationEvent::Terminal(GenerationTerminal::Finished(_))
        ));
    }

    #[tokio::test]
    async fn dropping_a_subscription_does_not_cancel_the_lease() {
        let registry = Arc::new(GenerationRegistry::default());
        let lease = registry.reserve(1, CancellationToken::new()).unwrap();
        drop(lease.subscribe());
        assert!(!lease.cancellation().is_cancelled());
    }
}
