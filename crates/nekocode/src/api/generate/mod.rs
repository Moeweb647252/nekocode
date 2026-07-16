mod cancel;
mod stream_generate;
pub(super) mod turn_io;
mod watch_stream;

use nekocode_core::agent::AgentEvent;
use serde::Serialize;
use std::sync::Arc;

use crate::AppState;

pub type ThreadId = u64;

pub struct GenerateState {
    pub deltas: boxcar::Vec<AgentEvent>,
    pub broadcast: tokio::sync::broadcast::Sender<AgentEvent>,
    pub cancellation_token: tokio_util::sync::CancellationToken,
    terminal: tokio::sync::watch::Sender<Option<StopReason>>,
}

impl GenerateState {
    pub fn new() -> Arc<Self> {
        Self::with_cancellation(tokio_util::sync::CancellationToken::new())
    }

    pub fn with_cancellation(cancellation_token: tokio_util::sync::CancellationToken) -> Arc<Self> {
        let (broadcast, _) = tokio::sync::broadcast::channel(100);
        let (terminal, _) = tokio::sync::watch::channel(None);
        Arc::new(Self {
            deltas: boxcar::Vec::new(),
            broadcast,
            cancellation_token,
            terminal,
        })
    }

    pub fn publish(&self, event: AgentEvent) {
        self.deltas.push(event.clone());
        let _ = self.broadcast.send(event);
    }

    pub fn finish(&self, stop: StopReason) {
        let _ = self.terminal.send_if_modified(|current| {
            if current.is_some() {
                false
            } else {
                *current = Some(stop);
                true
            }
        });
    }

    pub fn terminal(&self) -> tokio::sync::watch::Receiver<Option<StopReason>> {
        self.terminal.subscribe()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WebSocketEvent {
    Delta(AgentEvent),
    Stop(StopReason),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopReason {
    pub reason: Reason,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    Finished,
    Interrupted,
    Error,
}

/// Send a terminal `Stop` frame, tolerating serialization / socket errors so
/// the upgrade future itself never panics.
pub(super) async fn send_stop(
    socket: &mut axum::extract::ws::WebSocket,
    reason: Reason,
    detail: serde_json::Value,
) {
    let Ok(payload) = serde_json::to_string(&WebSocketEvent::Stop(StopReason { reason, detail }))
    else {
        return;
    };
    // `String → ws::Message` is infallible (Text variant).
    let payload: axum::extract::ws::Message = payload.into();
    let _ = socket.send(payload).await;
}

pub(super) async fn send_terminal(socket: &mut axum::extract::ws::WebSocket, stop: StopReason) {
    let Ok(payload) = serde_json::to_string(&WebSocketEvent::Stop(stop)) else {
        return;
    };
    let _ = socket
        .send(axum::extract::ws::Message::Text(payload.into()))
        .await;
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/stream",
            axum::routing::any(stream_generate::stream_generate),
        )
        .route(
            "/watch/{thread_id}",
            axum::routing::any(watch_stream::watch_stream),
        )
        .route("/cancel", axum::routing::post(cancel::cancel_generation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_result_is_written_once() {
        let state = GenerateState::new();
        state.finish(StopReason {
            reason: Reason::Interrupted,
            detail: serde_json::Value::Null,
        });
        state.finish(StopReason {
            reason: Reason::Finished,
            detail: serde_json::json!({ "unexpected": true }),
        });

        let terminal = state.terminal();
        let value = terminal.borrow().clone().expect("terminal result");
        assert!(matches!(value.reason, Reason::Interrupted));
    }
}
