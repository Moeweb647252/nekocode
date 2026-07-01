use std::sync::Arc;

use anyhow::{anyhow, bail};
use axum::{extract::ws, response::Response};
use nekocode_types::generate::MessageContent;
use tracing::error;

use crate::api::{
    generate::{GenerateState, Reason, WebSocketEvent, turn_io},
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
                super::send_stop(&mut ws, Reason::Error, e.to_string().into()).await;
            }
        }
    })
}

pub async fn handle_websocket(socket: &mut ws::WebSocket, state: AppState) -> anyhow::Result<()> {
    // Read the single initial payload message.
    let payload: StreamGenerate = match socket.recv().await {
        Some(Ok(ws::Message::Text(bytes))) => serde_json::from_str(&bytes.to_string())?,
        Some(Ok(_)) => bail!("unexpected message type"),
        Some(Err(e)) => bail!("error receiving message: {e}"),
        None => bail!("socket closed before receiving message"),
    };
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel(100);
    let cancellation_token = tokio_util::sync::CancellationToken::new();
    let generate_state = Arc::new(GenerateState {
        deltas: boxcar::Vec::new(),
        broadcast: broadcast_rx,
        cancellation_token: cancellation_token.clone(),
    });
    match state.generate_states.entry(payload.thread_id) {
        dashmap::Entry::Occupied(_) => bail!("thread generating"),
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
    // Load the thread's history here (not inside the agent) so a load failure
    // is reported to the client as an Error frame rather than a socket drop.
    let old_turns = turn_io::load_turn_context(&state.db, payload.thread_id)
        .await
        .map_err(|e| anyhow!("error loading turn context: {e}"))?;
    let thread_id = payload.thread_id;
    let user_input = payload.user_input;
    let handle = tokio::spawn(async move {
        agent
            .run_loop(
                vec![MessageContent::Text { content: user_input }],
                old_turns,
                nekocode_core::agent::AgentEventSink::new(tx),
            )
            .await
    });

    // Forward agent events to the client socket until the stream closes,
    // the client disconnects, or the generation is interrupted.
    let task = async {
        while let Some(event) = rx.recv().await {
            generate_state.deltas.push(event.clone());
            // Broadcast failures (no live watchers) are not fatal.
            let _ = broadcast_tx.send(event.clone());
            socket
                .send(ws::Message::Text(
                    serde_json::to_string(&WebSocketEvent::Delta(event))?.try_into()?,
                ))
                .await?;
        }
        anyhow::Ok(())
    };

    let result = tokio::select! {
        res = task => res,
        _ = cancellation_token.cancelled() => {
            // Interrupted: abort the agent task, send Interrupted, and return
            // without emitting a second (Error) Stop frame.
            handle.abort();
            super::send_stop(socket, Reason::Interrupted, serde_json::Value::Null).await;
            return Ok(());
        }
    };

    match result {
        Ok(()) => {
            let run_result = handle
                .await
                .map_err(|e| anyhow!("error joining agent task: {e}"))?;
            match run_result {
                // Turn completed: persist it, then send a Finished frame
                // carrying the aggregate usage directly as the detail payload.
                Ok(turn) => {
                    if let Err(e) = turn_io::persist_turn(&state.db, thread_id, turn.clone()).await {
                        error!("error persisting turn {thread_id}: {e}");
                    }
                    super::send_stop(
                        socket,
                        Reason::Finished,
                        serde_json::to_value(turn.usage)?,
                    )
                    .await;
                }
                // Run errored: the agent already emitted the error as a
                // MessageEnd(Error) stream event (forwarded above). Discard the
                // partial turn (today nothing consumes unfinished turns) and
                // close the socket with an Error frame.
                Err(_partial) => {
                    super::send_stop(socket, Reason::Error, serde_json::Value::Null).await;
                }
            }
            Ok(())
        }
        // The forwarder errored (socket send failed, etc.). Abort the agent so
        // it doesn't keep running detached, then propagate the error so the
        // upgrade closure sends an Error frame once.
        Err(e) => {
            handle.abort();
            Err(e)
        }
    }
}
