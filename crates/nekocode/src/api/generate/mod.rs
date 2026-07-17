mod cancel;
mod stream_generate;
mod watch_stream;

use nekocode_core::agent::AgentEvent;
use serde::Serialize;

use crate::AppState;
use crate::runtime::generation::{GenerationEvent, GenerationSubscription, GenerationTerminal};

pub(crate) type ThreadId = u64;

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

pub(super) async fn forward_subscription(
    socket: &mut axum::extract::ws::WebSocket,
    subscription: &mut GenerationSubscription,
) -> anyhow::Result<()> {
    loop {
        match subscription.next().await {
            GenerationEvent::Delta(event) => {
                let payload = serde_json::to_string(&WebSocketEvent::Delta(event))?;
                // A disconnected subscriber is intentionally just dropped.
                // The generation lease remains owned by its detached runtime task.
                if socket
                    .send(axum::extract::ws::Message::Text(payload.into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            GenerationEvent::Terminal(terminal) => {
                send_terminal(socket, terminal.into_stop_reason()).await;
                return Ok(());
            }
        }
    }
}

trait IntoStopReason {
    fn into_stop_reason(self) -> StopReason;
}

impl IntoStopReason for GenerationTerminal {
    fn into_stop_reason(self) -> StopReason {
        match self {
            GenerationTerminal::Finished(usage) => StopReason {
                reason: Reason::Finished,
                detail: serde_json::to_value(usage).unwrap_or(serde_json::Value::Null),
            },
            GenerationTerminal::Interrupted => StopReason {
                reason: Reason::Interrupted,
                detail: serde_json::Value::Null,
            },
            GenerationTerminal::Error(error) => StopReason {
                reason: Reason::Error,
                detail: error.into(),
            },
        }
    }
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
