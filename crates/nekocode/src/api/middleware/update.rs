use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMiddleware {
    pub id: u64,
    pub config: serde_json::Value,
}

pub async fn update_middleware(
    State(mut state): State<AppState>,
    Json(payload): Json<UpdateMiddleware>,
) -> ApiResult {
    let mut update = toasty::query!(Middleware FILTER .id == #(payload.id)).update();
    update.set_config(toasty::Json(payload.config));
    update.exec(&mut state.db).await?;
    ApiResponse::ok(())
}
