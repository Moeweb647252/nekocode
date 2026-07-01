//! Cheaply cloneable handle onto the parent's outbound stream. `index`
//! is a shared atomic so every producer (the parent's own `Agent::send`
//! and any merge relay forwarding `MiddlewareEvent`s) allocates a
//! unique, contiguous monotonic index — required once the stream has
//! more than one producer.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc::UnboundedSender;

use crate::agent::error::AgentError;
use crate::agent::{AgentEvent, AgentEventType};

#[derive(Clone)]
pub struct AgentEventSink {
    tx: UnboundedSender<AgentEvent>,
    index: Arc<AtomicUsize>,
}

impl AgentEventSink {
    pub fn new(tx: UnboundedSender<AgentEvent>) -> Self {
        Self { tx, index: Arc::new(AtomicUsize::new(0)) }
    }

    /// Allocate the next unique index and send. Fails only on client
    /// disconnect (the receiver gone).
    pub fn send(&self, data: AgentEventType) -> Result<(), AgentError> {
        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(AgentEvent { index: idx, data })
            .map_err(|e| AgentError::Other(anyhow::anyhow!("error sending agent event {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekocode_types::generate::{StreamEvent, StreamEventData};
    use tokio::sync::mpsc;

    fn turn_end() -> AgentEventType {
        AgentEventType::StreamEvent(StreamEvent {
            data: StreamEventData::TurnEnd,
            created_at: jiff::Timestamp::now(),
        })
    }

    #[tokio::test]
    async fn send_allocates_unique_contiguous_indices_across_clones() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = AgentEventSink::new(tx);
        // Two producers sharing one sink via clone.
        let sink_b = sink.clone();

        sink.send(turn_end()).unwrap();        // index 0
        sink_b.send(turn_end()).unwrap();      // index 1
        sink.send(turn_end()).unwrap();        // index 2

        let mut got = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            got.push(ev.index);
        }
        assert_eq!(got, vec![0, 1, 2]);
    }

    #[test]
    fn send_returns_err_when_receiver_dropped() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink = AgentEventSink::new(tx);
        drop(rx);
        // Receiver gone → SendError → AgentError::Other. Locks the only
        // failure contract every downstream caller relies on.
        match sink.send(turn_end()) {
            Err(crate::agent::error::AgentError::Other(_)) => (),
            other => panic!("expected AgentError::Other, got {other:?}"),
        }
    }
}
