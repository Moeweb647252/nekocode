use std::sync::atomic;

use nekocode_shell::ShellTaskState;
use serde_json::json;

use crate::api::prelude::*;

#[derive(serde::Deserialize)]
pub struct ListShells {
    pub thread_id: u64,
}

pub async fn list_shells(
    State(state): State<AppState>,
    Json(ListShells { thread_id }): Json<ListShells>,
) -> ApiResult {
    let thread_state = state
        .runtime()
        .active_agent(thread_id)
        .map_err(ApiError::from)?;
    let shell_states = thread_state
        .extensions
        .get::<dashmap::DashMap<u32, ShellTaskState>>()
        .ok_or_else(|| {
            ApiError::ItemNotFound(String::from("shell middleware not configured for thread"))
        })?;
    let shell_states: Vec<serde_json::Value> = shell_states
        .iter()
        .map(|entry| entry.value().clone())
        .map(|state| {
            json!(
                {
                    "shellId": state.shell_id,
                    "pid": state.pid,
                    "command": state.command,
                    "isRunning": state.is_running.load(atomic::Ordering::SeqCst),
                }
            )
        })
        .collect::<Vec<_>>();
    ApiResponse::ok(json!(shell_states))
}
