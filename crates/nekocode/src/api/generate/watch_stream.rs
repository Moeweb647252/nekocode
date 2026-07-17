use crate::api::{
    generate::{Reason, ThreadId},
    prelude::*,
};
use axum::{
    extract::{Path, WebSocketUpgrade, ws::WebSocket},
    response::Response,
};
use tracing::error;

pub async fn watch_stream(
    State(state): State<AppState>,
    Path(thread_id): Path<ThreadId>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        if let Err(error) = handle_websocket(&mut socket, state, thread_id).await {
            error!("error handling watch stream: {error}");
            super::send_stop(&mut socket, Reason::Error, error.to_string().into()).await;
        }
    })
}

pub async fn handle_websocket(
    socket: &mut WebSocket,
    state: AppState,
    thread_id: u64,
) -> anyhow::Result<()> {
    let mut subscription = state
        .runtime()
        .subscribe_generation(thread_id)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    super::forward_subscription(socket, &mut subscription).await
}
