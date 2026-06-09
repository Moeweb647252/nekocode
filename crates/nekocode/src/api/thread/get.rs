use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetThread {
    pub id: u64,
    pub turns_limit: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetThreadResponse {
    pub id: u64,
    pub title: Option<String>,
    pub working_directory: String,
    pub updated_at: jiff::Timestamp,
    pub created_at: jiff::Timestamp,
    pub active: bool,
    pub generating: bool,
    pub turns: Vec<Turn>,
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
        let turns = if let Some(limit) = payload.turns_limit {
            toasty::query!(Turn FILTER .thread_id == #(payload.id) ORDER BY .id ASC LIMIT #limit)
                .include(Turn::fields().messages())
                .exec(&mut state.db)
                .await?
        } else {
            toasty::query!(Turn FILTER .thread_id == #(payload.id) ORDER BY .id ASC LIMIT 1)
                .include(Turn::fields().messages())
                .exec(&mut state.db)
                .await?
        };
        ApiResponse::ok(GetThreadResponse {
            id: thread.id,
            title: thread.title,
            working_directory: thread.working_directory,
            updated_at: thread.updated_at,
            created_at: thread.created_at,
            active: state.active_threads.contains_key(&thread.id),
            generating: state.generate_states.contains_key(&thread.id),
            turns,
        })
    } else {
        Err(ApiError::ItemNotFound(format!(
            "Thread not found: {}",
            payload.id
        )))
    }
}
