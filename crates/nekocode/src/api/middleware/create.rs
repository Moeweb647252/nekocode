use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMiddleware {
    pub thread_id: u64,
    pub name: String,
    pub config: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiddlewareResponse {
    pub id: u64,
    pub name: String,
    pub config: serde_json::Value,
    pub enabled: bool,
}

/// Create a new middleware attached to a thread. Defaults to enabled. Thread
/// creation seeds Shell/Tool; this endpoint is used to add Mcp instances.
pub async fn create_middleware(
    State(state): State<AppState>,
    Json(payload): Json<CreateMiddleware>,
) -> ApiResult {
    let runtime = state.runtime();
    let lifecycle = runtime.lifecycle_guard().await;
    lifecycle
        .ensure_idle(payload.thread_id)
        .map_err(ApiError::from)?;
    let mut db = state.db();
    let _thread = toasty::query!(Thread FILTER .id == #(payload.thread_id))
        .first()
        .exec(&mut db)
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Thread not found: {}",
            payload.thread_id
        )))?;

    let next_order_index = toasty::query!(Middleware FILTER .thread_id == #(payload.thread_id) ORDER BY .order_index DESC LIMIT 1)
        .first()
        .exec(&mut db)
        .await?
        .map(|middleware| middleware.order_index.saturating_add(100))
        .unwrap_or(100);

    let created = toasty::create!(Middleware {
        thread_id: payload.thread_id,
        order_index: next_order_index,
        name: payload.name,
        config: toasty::Json(payload.config),
    })
    .exec(&mut db)
    .await?;

    // Evict the cached agent so the next activation picks up the new middleware.
    lifecycle.remove_and_shutdown(payload.thread_id).await;

    ApiResponse::ok(MiddlewareResponse {
        id: created.id,
        name: created.name,
        config: created.config.0,
        enabled: created.enabled,
    })
}
