use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteMiddleware {
    pub id: u64,
}

/// Delete a middleware by id. Refuses while the owning thread is generating.
/// After deletion, the cached agent is evicted so the next activation reflects
/// the updated middleware list.
pub async fn delete_middleware(
    State(mut state): State<AppState>,
    Json(payload): Json<DeleteMiddleware>,
) -> ApiResult {
    let _lifecycle = state.thread_lifecycle.lock().await;
    // Look up the middleware's thread_id for the generating check + evict.
    let mw = toasty::query!(Middleware FILTER .id == #(payload.id))
        .first()
        .exec(&mut state.db)
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Middleware not found: {}",
            payload.id
        )))?;

    if state.generate_states.contains_key(&mw.thread_id) {
        return Err(ApiError::ThreadGenerating);
    }

    toasty::query!(Middleware FILTER .id == #(payload.id))
        .delete()
        .exec(&mut state.db)
        .await?;

    // Evict the cached agent so the next activation picks up the change.
    crate::api::thread::shutdown_and_remove_agent(&state.active_threads, mw.thread_id).await;

    ApiResponse::ok(())
}
