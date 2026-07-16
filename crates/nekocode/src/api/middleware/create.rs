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
    State(mut state): State<AppState>,
    Json(payload): Json<CreateMiddleware>,
) -> ApiResult {
    let _lifecycle = state.thread_lifecycle.lock().await;
    let _thread = toasty::query!(Thread FILTER .id == #(payload.thread_id))
        .first()
        .exec(&mut state.db)
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Thread not found: {}",
            payload.thread_id
        )))?;

    if state.generate_states.contains_key(&payload.thread_id) {
        return Err(ApiError::ThreadGenerating);
    }

    let created = toasty::create!(Middleware {
        thread_id: payload.thread_id,
        name: payload.name,
        config: toasty::Json(payload.config),
    })
    .exec(&mut state.db)
    .await?;

    // Evict the cached agent so the next activation picks up the new middleware.
    crate::api::thread::shutdown_and_remove_agent(&state.active_threads, payload.thread_id).await;

    ApiResponse::ok(MiddlewareResponse {
        id: created.id,
        name: created.name,
        config: created.config.0,
        enabled: created.enabled,
    })
}
