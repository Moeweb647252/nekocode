use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMiddleware {
    pub id: u64,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

/// Update a middleware's config and/or enabled flag. Refuses while the owning
/// thread is generating and evicts the cached agent so the next activation
/// reflects the change.
pub async fn update_middleware(
    State(mut state): State<AppState>,
    Json(payload): Json<UpdateMiddleware>,
) -> ApiResult {
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

    let mut update = toasty::query!(Middleware FILTER .id == #(payload.id)).update();
    if let Some(config) = payload.config {
        update.set_config(toasty::Json(config));
    }
    if let Some(enabled) = payload.enabled {
        update.set_enabled(enabled);
    }
    update.exec(&mut state.db).await?;

    state.active_threads.remove(&mw.thread_id);
    ApiResponse::ok(())
}
