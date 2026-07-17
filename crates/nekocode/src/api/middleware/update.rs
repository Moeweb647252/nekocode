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
    State(state): State<AppState>,
    Json(payload): Json<UpdateMiddleware>,
) -> ApiResult {
    let runtime = state.runtime();
    let lifecycle = runtime.lifecycle_guard().await;
    let mut db = state.db();
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

    let mut update = toasty::query!(Middleware FILTER .id == #(payload.id)).update();
    if let Some(config) = payload.config {
        update.set_config(toasty::Json(config));
    }
    if let Some(enabled) = payload.enabled {
        update.set_enabled(enabled);
    }
    update.exec(&mut db).await?;

    lifecycle.remove_and_shutdown(mw.thread_id).await;
    ApiResponse::ok(())
}
