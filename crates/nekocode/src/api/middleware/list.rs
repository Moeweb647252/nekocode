use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMiddlewares {
    pub thread_id: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiddlewareResponse {
    pub id: u64,
    pub name: String,
    pub config: serde_json::Value,
    pub enabled: bool,
}

pub async fn list_middlewares(
    State(mut state): State<AppState>,
    Json(payload): Json<ListMiddlewares>,
) -> ApiResult {
    let rows = toasty::query!(Middleware FILTER .thread_id == #(payload.thread_id))
        .exec(&mut state.db)
        .await?;
    let middlewares: Vec<MiddlewareResponse> = rows
        .into_iter()
        .map(|m| MiddlewareResponse {
            id: m.id,
            name: m.name,
            config: m.config.0,
            enabled: m.enabled,
        })
        .collect();
    ApiResponse::ok(middlewares)
}
