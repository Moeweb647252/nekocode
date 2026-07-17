use crate::api::prelude::*;

pub async fn list_threads(State(state): State<AppState>) -> ApiResult {
    let mut db = state.db();
    let threads = toasty::query!(Thread).exec(&mut db).await?;
    ApiResponse::ok(threads)
}
