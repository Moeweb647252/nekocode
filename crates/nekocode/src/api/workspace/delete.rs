use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteWorkspace {
    pub id: u64,
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    Json(payload): Json<DeleteWorkspace>,
) -> ApiResult {
    state
        .runtime()
        .delete_workspace(payload.id)
        .await
        .map_err(ApiError::from)?;
    ApiResponse::ok(())
}
