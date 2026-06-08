mod stream_generate;
mod watch_stream;

use nekocode_core::agent::AgentEvent;
use serde::Serialize;

use crate::AppState;

pub type ThreadId = u64;

pub struct GenerateState {
    pub thread_id: ThreadId,
    pub deltas: boxcar::Vec<AgentEvent>,
    pub boardcast: tokio::sync::broadcast::Receiver<AgentEvent>,
    pub cancallation_token: tokio_util::sync::CancellationToken,
}

#[derive(Serialize)]
pub enum WebSocketEvent {
    Delta(AgentEvent),
    Stop(StopReason),
}

#[derive(Debug, Serialize)]
pub struct StopReason {
    pub reason: Reason,
    pub detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub enum Reason {
    Finished,
    Interrupted,
    Error,
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/stream",
            axum::routing::get(stream_generate::stream_generate),
        )
        .route(
            "/watch/{thread_id}",
            axum::routing::get(watch_stream::watch_stream),
        )
}
