use std::sync::Arc;

use anyhow::{anyhow, bail};
use axum::{extract::ws, response::Response};
use nekocode_core::agent::RunLoopSummary;
use tracing::error;

use crate::api::{
    generate::{GenerateState, Reason, StopReason, WebSocketEvent},
    prelude::*,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamGenerate {
    pub user_input: String,
    pub thread_id: u64,
}

pub async fn stream_generate(State(state): State<AppState>, ws: ws::WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut ws| async move {
        match handle_websocket(&mut ws, state).await {
            Ok(_) => (),
            Err(e) => {
                error!("error handling stream generate: {e}");
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

pub async fn handle_websocket(socket: &mut ws::WebSocket, state: AppState) -> anyhow::Result<()> {
    let payload: StreamGenerate = loop {
        match socket.recv().await {
            Some(Ok(ws::Message::Text(bytes))) => {
                break serde_json::from_str(&bytes.to_string())?;
            }
            Some(Ok(_)) => return Err(anyhow::anyhow!("unexpected message type")),
            Some(Err(e)) => return Err(anyhow::anyhow!("error receiving message: {e}")),
            None => return Err(anyhow::anyhow!("socket closed before receiving message")),
        }
    };
    if state.generate_states.contains_key(&payload.thread_id) {
        bail!("thread generating");
    }
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel(100);
    let cancellation_token = tokio_util::sync::CancellationToken::new();
    let generate_state = Arc::new(GenerateState {
        thread_id: payload.thread_id,
        deltas: boxcar::Vec::new(),
        broadcast: broadcast_rx,
        cancellation_token: cancellation_token.clone(),
    });
    match state.generate_states.entry(payload.thread_id) {
        dashmap::Entry::Occupied(_) => {
            bail!("thread generating");
        }
        dashmap::Entry::Vacant(entry) => {
            entry.insert(generate_state.clone());
        }
    }
    let guard_thread_id = payload.thread_id;
    scopeguard::defer! {
        state.generate_states.remove(&guard_thread_id);
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let agent = if let Some(agent) = state.active_threads.get(&payload.thread_id) {
        agent.read().await.clone()
    } else {
        bail!("thread not activated");
    };
    let handle = tokio::spawn(async move { agent.run_loop(payload.user_input, tx).await });
    let task = async {
        while let Some(event) = rx.recv().await {
            generate_state.deltas.push(event.clone());
            broadcast_tx.send(event.clone()).map_err(|_| anyhow!(""))?;
            socket
                .send(ws::Message::Text(
                    serde_json::to_string(&WebSocketEvent::Delta(event))?.try_into()?,
                ))
                .await?;
        }
        anyhow::Ok(())
    };
    let _ = tokio::select! {
        res = task => res?,
        _ = cancellation_token.cancelled() => {
            handle.abort();
            socket.send(ws::Message::Text(
                serde_json::to_string(&WebSocketEvent::Stop(StopReason {
                    reason: Reason::Interrupted,
                    detail: serde_json::Value::Null,
                }))?
                .try_into()?,
            )).await?;
        }
    };
    let summary = handle.await??;
    socket
        .send(ws::Message::Text(
            serde_json::to_string(&WebSocketEvent::Stop(StopReason {
                reason: Reason::Finished,
                detail: serde_json::to_value(summary)?,
            }))?
            .try_into()?,
        ))
        .await?;
    Ok(())
}
