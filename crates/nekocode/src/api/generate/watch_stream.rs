use crate::api::{
    generate::{Reason, ThreadId, WebSocketEvent},
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
                super::send_stop(&mut ws, Reason::Error, e.to_string().into()).await;
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
    let mut rx = generate_state.broadcast.subscribe();
    let mut terminal = generate_state.terminal();

    // Replay historical deltas so late joiners catch up. We track a contiguous
    // watermark: the index one past the last replayed event. Using a strict
    // `max`-based watermark would be unsafe if `boxcar::Vec::iter()` ever
    // observed indices out of order, so we only advance when we see the exact
    // next index we expect.
    let mut watermark = 0usize;
    for (i, delta) in generate_state.deltas.iter() {
        if i == watermark {
            watermark = i + 1;
            ws.send(ws::Message::Text(
                serde_json::to_string(&WebSocketEvent::Delta(delta.clone()))?.into(),
            ))
            .await?;
        } else if i > watermark {
            // Iteration skipped ahead (shouldn't happen with boxcar's monotonic
            // push indices, but guard regardless); advance to cover the gap.
            watermark = i + 1;
            ws.send(ws::Message::Text(
                serde_json::to_string(&WebSocketEvent::Delta(delta.clone()))?.into(),
            ))
            .await?;
        }
    }

    let cancellation = generate_state.cancellation_token.clone();

    let initial_stop = { terminal.borrow().clone() };
    if let Some(stop) = initial_stop {
        super::send_terminal(ws, stop).await;
        return Ok(());
    }

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        // Skip events we already replayed.
                        if event.index < watermark {
                            continue;
                        }
                        // Keep the watermark moving forward so future replays
                        // (after a lag) stay in sync with what we've sent.
                        watermark = watermark.max(event.index + 1);
                        ws.send(ws::Message::Text(
                            serde_json::to_string(&WebSocketEvent::Delta(event))?.into(),
                        ))
                        .await?;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        let terminal_stop = { terminal.borrow().clone() };
                        let stop = terminal_stop.unwrap_or_else(|| super::StopReason {
                            reason: Reason::Error,
                            detail: "generation event channel closed without a terminal result".into(),
                        });
                        super::send_terminal(ws, stop).await;
                        return Ok(());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // The live receiver skipped events. Recover them from
                        // the durable boxcar buffer by replaying everything at
                        // or beyond the watermark, so a slow watcher doesn't
                        // silently lose content.
                        for (i, delta) in generate_state.deltas.iter() {
                            if i < watermark {
                                continue;
                            }
                            watermark = i + 1;
                            ws.send(ws::Message::Text(
                                serde_json::to_string(&WebSocketEvent::Delta(delta.clone()))?.into(),
                            ))
                            .await?;
                        }
                    }
                }
            }
            changed = terminal.changed() => {
                if changed.is_err() {
                    super::send_stop(
                        ws,
                        Reason::Error,
                        "generation ended without a terminal result".into(),
                    ).await;
                    return Ok(());
                }
                let terminal_stop = { terminal.borrow().clone() };
                if let Some(stop) = terminal_stop {
                    super::send_terminal(ws, stop).await;
                    return Ok(());
                }
            }
            _ = cancellation.cancelled() => {
                super::send_stop(ws, Reason::Interrupted, serde_json::Value::Null).await;
                return Ok(());
            }
        }
    }
}
