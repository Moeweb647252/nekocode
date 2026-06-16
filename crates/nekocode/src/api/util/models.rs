use crate::api::prelude::*;

/// Available model names from the server config — drives the model dropdown in
/// the per-thread settings dialog.
pub async fn list_models(State(state): State<AppState>) -> ApiResult {
    let config = state.config.read().await;
    let names: Vec<String> = config.models.iter().map(|m| m.name.clone()).collect();
    ApiResponse::ok(names)
}
