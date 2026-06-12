use crate::api::prelude::*;

#[derive(serde::Deserialize)]
pub struct ListShells {
    pub thread_id: u64,
}

pub async fn list_shells(
    State(mut state): State<AppState>,
    Json(ListShells { thread_id }): Json<ListShells>,
) -> ApiResult {
    let thread_state = {
        state
            .active_threads
            .get(&thread_id)
            .ok_or(ApiError::ThreadNotActivated)?
            .clone()
    };
    let shell_states = thread_state
        .read()
        .await
        .extensions
        .get("shell")
        .ok_or_else(|| ApiError::ThreadNotActivated)?
        .downcast::<Arc<dashmap::DashMap<u32, ShellTaskState>>>()
        .unwrap_or_else(|| ApiError::ItemNotFound(String::from("shell middleware ext")));

    todo!()
}
