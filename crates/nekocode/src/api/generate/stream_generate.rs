use std::sync::Arc;

use anyhow::{Context as _, bail};
use axum::{extract::ws, response::Response};
use nekocode_types::generate::MessageContent;
use tracing::error;

use crate::api::{
    generate::{GenerateState, Reason, StopReason, WebSocketEvent, turn_io},
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
        if let Err(e) = handle_websocket(&mut ws, state).await {
            error!("error handling stream generate: {e}");
            super::send_stop(&mut ws, Reason::Error, e.to_string().into()).await;
        }
    })
}

pub async fn handle_websocket(socket: &mut ws::WebSocket, state: AppState) -> anyhow::Result<()> {
    let payload: StreamGenerate = match socket.recv().await {
        Some(Ok(ws::Message::Text(bytes))) => serde_json::from_str(&bytes.to_string())?,
        Some(Ok(_)) => bail!("unexpected message type"),
        Some(Err(e)) => bail!("error receiving message: {e}"),
        None => bail!("socket closed before receiving message"),
    };

    let generate_state = GenerateState::new();
    let agent_entry = {
        // Reservation and destructive/configuration operations share this lock.
        // Once the state is inserted, delete/update paths must reject until it
        // is released below.
        let _lifecycle = state.thread_lifecycle.lock().await;
        if state.generate_states.contains_key(&payload.thread_id) {
            bail!("thread generating");
        }
        let agent = state
            .active_threads
            .get(&payload.thread_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("thread not activated"))?;
        state
            .generate_states
            .insert(payload.thread_id, generate_state.clone());
        agent
    };

    let agent = agent_entry.read().await.clone();
    let thread_id = payload.thread_id;
    let stop =
        run_registered_generation(socket, &state, generate_state.clone(), agent, payload).await;

    generate_state.finish(stop.clone());
    release_generation(&state, thread_id, &generate_state);
    super::send_terminal(socket, stop).await;
    Ok(())
}

// Keep release keyed explicitly without allowing an old handler to remove a
// newer run that reused the same thread id.
fn release_generation(state: &AppState, thread_id: u64, expected: &Arc<GenerateState>) {
    let should_remove = state
        .generate_states
        .get(&thread_id)
        .map(|current| Arc::ptr_eq(current.value(), expected))
        .unwrap_or(false);
    if should_remove {
        state.generate_states.remove(&thread_id);
    }
}

async fn run_registered_generation(
    socket: &mut ws::WebSocket,
    state: &AppState,
    generate_state: Arc<GenerateState>,
    agent: nekocode_core::agent::Agent,
    payload: StreamGenerate,
) -> StopReason {
    let old_turns = match turn_io::load_turn_context(&state.db, payload.thread_id).await {
        Ok(turns) => turns,
        Err(e) => return error_stop(format!("error loading turn context: {e}")),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let cancellation = generate_state.cancellation_token.clone();
    let agent_cancellation = cancellation.clone();
    let thread_id = payload.thread_id;
    let handle = tokio::spawn(async move {
        agent
            .run_loop_with_cancellation(
                vec![MessageContent::Text {
                    content: payload.user_input,
                }],
                old_turns,
                nekocode_core::agent::AgentEventSink::new(tx),
                agent_cancellation,
            )
            .await
    });

    let mut cancelled = false;
    // The initiating socket is only one subscriber. A closed browser tab or a
    // transient network failure must not abort work that a `/watch` client can
    // resume observing, so detach it while the agent keeps running.
    let mut socket_connected = true;
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => {
                cancelled = true;
                break;
            }
            incoming = socket.recv(), if socket_connected => {
                match incoming {
                    None | Some(Ok(ws::Message::Close(_))) | Some(Err(_)) => {
                        socket_connected = false;
                    }
                    Some(Ok(_)) => {}
                }
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                generate_state.publish(event.clone());
                let payload = match serde_json::to_string(&WebSocketEvent::Delta(event)) {
                    Ok(payload) => payload,
                    Err(e) => {
                        tracing::error!("error serializing stream event: {e}");
                        socket_connected = false;
                        continue;
                    }
                };
                if socket_connected
                    && socket.send(ws::Message::Text(payload.into())).await.is_err()
                {
                    socket_connected = false;
                }
            }
        }
    }

    let run_result = match handle.await.context("error joining agent task") {
        Ok(result) => result,
        Err(e) => return error_stop(e.to_string()),
    };

    match run_result {
        Ok(turn) => {
            let usage = turn.usage.clone();
            if let Err(e) = turn_io::persist_turn(&state.db, thread_id, turn).await {
                return error_stop(format!("error persisting turn {thread_id}: {e}"));
            }
            if cancelled {
                StopReason {
                    reason: Reason::Interrupted,
                    detail: serde_json::Value::Null,
                }
            } else {
                StopReason {
                    reason: Reason::Finished,
                    detail: serde_json::to_value(usage).unwrap_or(serde_json::Value::Null),
                }
            }
        }
        Err(partial) => {
            if let Err(e) = turn_io::persist_turn(&state.db, thread_id, partial).await {
                return error_stop(format!("error persisting partial turn {thread_id}: {e}"));
            }
            if cancelled {
                StopReason {
                    reason: Reason::Interrupted,
                    detail: serde_json::Value::Null,
                }
            } else {
                error_stop("agent run failed")
            }
        }
    }
}

fn error_stop(detail: impl Into<String>) -> StopReason {
    StopReason {
        reason: Reason::Error,
        detail: detail.into().into(),
    }
}
