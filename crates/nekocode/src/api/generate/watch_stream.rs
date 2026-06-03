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
    let generate_state_ref = state
        .generate_states
        .get(&thread_id)
        .ok_or_else(|| anyhow::anyhow!("no active generation for thread {}", thread_id))?;
    let generate_state = generate_state_ref.clone();
    drop(generate_state_ref);

    // Subscribe first so no events are missed between replay and live listening.
    let mut rx = generate_state.boardcast.resubscribe();

    // Replay historical deltas so late joiners catch up.
    // Track the index one past the last replayed event for dedup.
    let mut watermark = 0;
    for (_, delta) in generate_state.deltas.iter() {
        watermark = delta.index.max(watermark);
        ws.send(ws::Message::Text(
            serde_json::to_string(&WebSocketEvent::Delta(delta.clone()))?.try_into()?,
        ))
        .await?;
    }

    let cancellation = generate_state.cancallation_token.clone();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        // Skip events we already replayed.
                        if event.index <= watermark {
                            continue;
                        }
                        ws.send(ws::Message::Text(
                            serde_json::to_string(&WebSocketEvent::Delta(event))?.try_into()?,
                        ))
                        .await?;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        ws.send(ws::Message::Text(
                            serde_json::to_string(&WebSocketEvent::Stop(StopReason {
                                reason: Reason::Finished,
                                detail: serde_json::Value::Null,
                            }))?
                            .try_into()?,
                        ))
                        .await?;
                        return Ok(());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                }
            }
            _ = cancellation.cancelled() => {
                ws.send(ws::Message::Text(
                    serde_json::to_string(&WebSocketEvent::Stop(StopReason {
                        reason: Reason::Interrupted,
                        detail: serde_json::Value::Null,
                    }))?
                    .try_into()?,
                ))
                .await?;
                return Ok(());
            }
        }
    }
}
