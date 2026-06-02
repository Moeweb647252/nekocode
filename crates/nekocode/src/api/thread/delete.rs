use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteThread {
    pub id: u64,
}

pub async fn delete_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<DeleteThread>,
) -> ApiResult {
    toasty::query!(Thread FILTER .id == #(payload.id))
        .delete()
        .exec(&mut state.db)
        .await?;
    toasty::query!(Message FILTER .thread_id == #(payload.id))
        .delete()
        .exec(&mut state.db)
        .await?;
    ApiResponse::ok(())
}
