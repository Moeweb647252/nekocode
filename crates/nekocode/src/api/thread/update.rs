use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct UpdateThread {
    pub id: u64,
    pub title: Option<String>,
    pub model: Option<String>,
}

pub async fn update_thread(
    State(state): State<AppState>,
    Json(payload): Json<UpdateThread>,
) -> ApiResult {
    // A model change affects the provider, which is built at activation. Refuse
    // to swap it mid-generation and evict the cached agent so the next
    // activation rebuilds it with the new provider.
    let changes_model = payload.model.is_some();
    let runtime = state.runtime();
    let lifecycle = runtime.lifecycle_guard().await;
    if changes_model {
        lifecycle.ensure_idle(payload.id).map_err(ApiError::from)?;
    }
    let mut db = state.db();
    let mut update = toasty::query!(Thread FILTER .id == #(payload.id)).update();
    if let Some(title) = payload.title {
        update.set_title(title);
    }
    if let Some(model) = payload.model {
        update.set_model(model);
    }
    update.exec(&mut db).await?;
    if changes_model {
        lifecycle.remove_and_shutdown(payload.id).await;
    }
    ApiResponse::ok(())
}
