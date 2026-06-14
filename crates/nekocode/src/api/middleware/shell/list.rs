use std::sync::{Arc, atomic};

use nekocode_shell::ShellTaskState;
use serde_json::json;

use crate::api::prelude::*;

#[derive(serde::Deserialize)]
pub struct ListShells {
    pub thread_id: u64,
}

pub async fn list_shells(
    State(mut state): State<AppState>,
    Json(ListShells { thread_id }): Json<ListShells>,
) -> ApiResult {
    let thread_state = {
        state
            .active_threads
            .get(&thread_id)
            .ok_or(ApiError::ThreadNotActivated)?
            .clone()
    };
    let shell_states = thread_state
        .read()
        .await
        .extensions
        .get("shell")
        .ok_or_else(|| ApiError::ThreadNotActivated)?
        .downcast_ref::<Arc<dashmap::DashMap<u32, ShellTaskState>>>()
        .ok_or_else(|| ApiError::ItemNotFound(String::from("shell middleware ext")))?
        .clone();
    let shell_states: Vec<serde_json::Value> = shell_states
        .iter()
        .map(|entry| entry.value().clone())
        .map(|state| {
            json!(
                {
                    "command": state.command,
                    "is_running": state.is_running.load(atomic::Ordering::SeqCst),
                }
            )
        })
        .collect::<Vec<_>>();
    ApiResponse::ok(json!(shell_states))
}
