mod stream_generate;
mod watch_stream;

use nekocode_core::agent::AgentEvent;
use serde::Serialize;

use crate::AppState;

pub type ThreadId = u64;

pub struct GenerateState {
    pub deltas: boxcar::Vec<AgentEvent>,
    pub broadcast: tokio::sync::broadcast::Receiver<AgentEvent>,
    pub cancellation_token: tokio_util::sync::CancellationToken,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WebSocketEvent {
    Delta(AgentEvent),
    Stop(StopReason),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopReason {
    pub reason: Reason,
    pub detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    Finished,
    Interrupted,
    Error,
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
}
