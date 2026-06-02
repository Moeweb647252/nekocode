use crate::api::{
    generate::{Reason, StopReason, ThreadId, WebSocketEvent},
    prelude::*,
};
use axum::{
    extract::{
        Path, WebSocketUpgrade,
        ws::{self, WebSocket},
    },
    response::Response,
};
use tracing::error;

pub async fn watch_stream(
    State(state): State<AppState>,
    Path(thread_id): Path<ThreadId>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |mut ws| async move {
        match handle_websocket(&mut ws, state, thread_id).await {
            Ok(_) => (),
            Err(e) => {
                error!("error handling watch stream: {e}");
                ws.send(ws::Message::Text(
                    serde_json::to_string(&WebSocketEvent::Stop(StopReason {
                        reason: Reason::Error,
                        detail: e.to_string().into(),
                    }))
                    .unwrap()
                    .try_into()
                    .unwrap(),
                ))
                .await
                .ok();
            }
        }
    })
}

pub async fn handle_websocket(
    ws: &mut WebSocket,
    state: AppState,
    thread_id: u64,
) -> anyhow::Result<()> {
    Ok(())
}
