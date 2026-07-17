use anyhow::bail;
use axum::{extract::ws, response::Response};
use tracing::error;

use crate::api::{generate::Reason, prelude::*};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamGenerate {
    pub user_input: String,
    pub thread_id: u64,
}

pub async fn stream_generate(State(state): State<AppState>, ws: ws::WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut socket| async move {
        if let Err(error) = handle_websocket(&mut socket, state).await {
            error!("error handling stream generate: {error}");
            super::send_stop(&mut socket, Reason::Error, error.to_string().into()).await;
        }
    })
}

pub async fn handle_websocket(socket: &mut ws::WebSocket, state: AppState) -> anyhow::Result<()> {
    let payload: StreamGenerate = match socket.recv().await {
        Some(Ok(ws::Message::Text(bytes))) => serde_json::from_str(&bytes.to_string())?,
        Some(Ok(_)) => bail!("unexpected message type"),
        Some(Err(error)) => bail!("error receiving message: {error}"),
        None => bail!("socket closed before receiving message"),
    };
    let mut subscription = state
        .runtime()
        .start_generation(payload.thread_id, payload.user_input)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    super::forward_subscription(socket, &mut subscription).await
}
