use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThread {
    pub working_directory: String,
}

#[axum::debug_handler]
pub async fn create_thread(
    State(state): State<AppState>,
    Json(payload): Json<CreateThread>,
) -> ApiResult {
    let thread = state
        .runtime()
        .create_root(payload.working_directory)
        .await
        .map_err(ApiError::from)?;
    ApiResponse::ok(thread)
}
