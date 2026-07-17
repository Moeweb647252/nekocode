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
    State(state): State<AppState>,
    Json(payload): Json<DeleteMiddleware>,
) -> ApiResult {
    let runtime = state.runtime();
    let lifecycle = runtime.lifecycle_guard().await;
    let mut db = state.db();
    // Look up the middleware's thread_id for the generating check + evict.
    let mw = toasty::query!(Middleware FILTER .id == #(payload.id))
        .first()
        .exec(&mut db)
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Middleware not found: {}",
            payload.id
        )))?;

    lifecycle
        .ensure_idle(mw.thread_id)
        .map_err(ApiError::from)?;

    toasty::query!(Middleware FILTER .id == #(payload.id))
        .delete()
        .exec(&mut db)
        .await?;

    // Evict the cached agent so the next activation picks up the change.
    lifecycle.remove_and_shutdown(mw.thread_id).await;

    ApiResponse::ok(())
}
