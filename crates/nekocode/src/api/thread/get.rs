use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct GetThread {
    pub id: u64,
}

#[derive(Serialize)]
pub struct GetThreadResponse {
    pub id: u64,
    pub title: Option<String>,
    pub working_directory: String,
    pub updated_at: jiff::Timestamp,
    pub created_at: jiff::Timestamp,
    pub active: bool,
    pub generating: bool,
    pub messages: Vec<Message>,
}

pub async fn get_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<GetThread>,
) -> ApiResult {
    let thread = toasty::query!(Thread FILTER .id == #(payload.id))
        .first()
        .exec(&mut state.db)
        .await?;
    if let Some(thread) = thread {
        let messages = toasty::query!(Message FILTER .thread_id == #(payload.id) ORDER BY .created_at DESC LIMIT 50)
            .exec(&mut state.db)
            .await?;
        ApiResponse::ok(GetThreadResponse {
            id: thread.id,
            title: thread.title,
            working_directory: thread.working_directory,
            updated_at: thread.updated_at,
            created_at: thread.created_at,
            active: state.active_threads.contains_key(&thread.id),
            generating: state.generate_states.contains_key(&thread.id),
            messages,
        })
    } else {
        Err(ApiError::ItemNotFound)
    }
}
