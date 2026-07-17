use crate::api::{ApiError, ApiResponse, ApiResult, prelude::*};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelGeneration {
    pub thread_id: u64,
}

/// Explicitly cancel an active generation. Socket lifecycle is intentionally
/// separate from run lifecycle so a client can reconnect through `/watch`.
pub async fn cancel_generation(
    State(state): State<AppState>,
    Json(payload): Json<CancelGeneration>,
) -> ApiResult {
    state
        .runtime()
        .cancel_generation(payload.thread_id)
        .map_err(ApiError::from)?;
    ApiResponse::ok(())
}
