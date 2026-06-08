use crate::api::prelude::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirsResponse {
    home_dir: String,
}

pub async fn get_dirs() -> ApiResult {
    ApiResponse::ok(DirsResponse {
        home_dir: dirs::home_dir()
            .ok_or_else(|| ApiError::Internal("Failed to get home directory".to_string()))?
            .to_string_lossy()
            .to_string(),
    })
}
