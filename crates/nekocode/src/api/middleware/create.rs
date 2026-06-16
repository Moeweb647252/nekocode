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
pub struct CreatedMiddleware {
    pub id: u64,
    pub name: String,
    pub config: serde_json::Value,
}

/// Create a new middleware attached to a thread. The config is stored as JSON
/// and can be edited later via `/middleware/update`. Thread creation does not
/// automatically add middlewares; this endpoint lets the UI add McpMiddleware
/// (or any other future middleware) dynamically.
pub async fn create_middleware(
    State(mut state): State<AppState>,
    Json(payload): Json<CreateMiddleware>,
) -> ApiResult {
    // Verify the thread exists.
    let _thread = toasty::query!(Thread FILTER .id == #(payload.thread_id))
        .first()
        .exec(&mut state.db)
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Thread not found: {}",
            payload.thread_id
        )))?;

    // Insert a new Middleware row linked to the thread.
    let created = toasty::create!(Middleware {
        thread_id: payload.thread_id,
        name: payload.name,
        config: toasty::Json(payload.config),
    })
    .exec(&mut state.db)
    .await?;

    ApiResponse::ok(CreatedMiddleware {
        id: created.id,
        name: created.name,
        config: created.config.0,
    })
}