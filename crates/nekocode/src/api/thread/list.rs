use crate::api::prelude::*;

pub async fn list_threads(State(mut state): State<AppState>) -> ApiResult {
    let threads = toasty::query!(Thread).exec(&mut state.db).await?;
    ApiResponse::ok(threads)
}
