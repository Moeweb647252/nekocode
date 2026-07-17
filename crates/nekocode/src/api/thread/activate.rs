use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct ActivateThread {
    pub id: u64,
}

pub async fn activate_thread(
    State(state): State<AppState>,
    Json(payload): Json<ActivateThread>,
) -> ApiResult {
    state
        .runtime()
        .activate_new(payload.id)
        .await
        .map_err(ApiError::from)?;
    ApiResponse::ok(())
}
