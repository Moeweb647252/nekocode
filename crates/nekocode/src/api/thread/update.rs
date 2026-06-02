use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct UpdateThread {
    pub id: u64,
    pub title: Option<String>,
}

pub async fn update_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<UpdateThread>,
) -> ApiResult {
    let mut update = toasty::query!(Thread FILTER .id == #(payload.id)).update();
    if let Some(title) = payload.title {
        update.set_title(title);
    }
    update.exec(&mut state.db).await?;
    ApiResponse::ok(())
}
