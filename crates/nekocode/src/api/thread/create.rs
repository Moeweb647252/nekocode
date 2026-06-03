use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct CreateThread {
    pub working_directory: String,
}

#[axum::debug_handler]
pub async fn create_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<CreateThread>,
) -> ApiResult {
    let model = {
        let config = state.config.read().await;
        config.default_model.clone()
    };
    let thread = toasty::create!(Thread {
        working_directory: payload.working_directory,
        model: model,
    })
    .exec(&mut state.db)
    .await?;
    ApiResponse::ok(thread)
}
