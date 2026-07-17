use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteThread {
    pub id: u64,
}

pub async fn delete_thread(
    State(state): State<AppState>,
    Json(payload): Json<DeleteThread>,
) -> ApiResult {
    state
        .runtime()
        .delete_threads_cascade(payload.id)
        .await
        .map_err(ApiError::from)?;
    ApiResponse::ok(())
}
