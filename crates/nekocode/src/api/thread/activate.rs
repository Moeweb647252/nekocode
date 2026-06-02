use std::sync::Arc;

use dashmap::Entry::{Occupied, Vacant};
use nekocode_core::agent::Agent;
use tokio::sync::RwLock;

use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct ActivateThread {
    pub id: u64,
}

pub async fn activate_thread(
    State(state): State<AppState>,
    Json(payload): Json<ActivateThread>,
) -> ApiResult {
    let thread_id = payload.id;
    if state.active_threads.contains_key(&thread_id) {
        return Err(ApiError::ThreadAlreadyActivated);
    }

    match state.active_threads.entry(thread_id) {
        Occupied(_) => {
            // This should never happen due to the check above, but we handle it just in case.
            return Err(ApiError::ThreadAlreadyActivated);
        }
        Vacant(entry) => {
            entry.insert(Arc::new(RwLock::new(Agent::new())));
        }
    }

    ApiResponse::ok(())
}
