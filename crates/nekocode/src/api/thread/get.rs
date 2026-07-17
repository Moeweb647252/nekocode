use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetThread {
    pub id: u64,
    pub turns_limit: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResponse {
    pub id: u64,
    pub title: Option<String>,
    pub working_directory: String,
    pub model: String,
    pub updated_at: jiff::Timestamp,
    pub created_at: jiff::Timestamp,
    pub active: bool,
    pub generating: bool,
    pub turns: Vec<TurnResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResponse {
    pub id: u64,
    pub thread_id: u64,
    pub turn_index: u64,
    pub usage: nekocode_types::generate::Usage,
    pub finished: bool,
    pub updated_at: jiff::Timestamp,
    pub created_at: jiff::Timestamp,
    pub messages: Vec<Message>,
}

pub async fn get_thread(
    State(state): State<AppState>,
    Json(payload): Json<GetThread>,
) -> ApiResult {
    let mut db = state.db();
    let thread = toasty::query!(Thread FILTER .id == #(payload.id))
        .first()
        .exec(&mut db)
        .await?;
    if let Some(thread) = thread {
        let turns = if let Some(limit) = payload.turns_limit {
            // Select the newest page, then restore chronological order for the
            // client transcript. `ASC LIMIT` would permanently hide recent
            // messages once a thread grew past the requested page size.
            let mut turns = toasty::query!(Turn FILTER .thread_id == #(payload.id) ORDER BY .id DESC LIMIT #limit)
                .exec(&mut db)
                .await?;
            turns.reverse();
            turns
        } else {
            // No limit requested: return only the latest turn (DESC + LIMIT 1).
            // Previously this was `ASC LIMIT 1`, which returned the *oldest* turn.
            toasty::query!(Turn FILTER .thread_id == #(payload.id) ORDER BY .id DESC LIMIT 1)
                .exec(&mut db)
                .await?
        };
        let turns = materialize_turns(&mut db, turns).await?;
        ApiResponse::ok(ThreadResponse {
            id: thread.id,
            title: thread.title,
            working_directory: thread.working_directory,
            model: thread.model,
            updated_at: thread.updated_at,
            created_at: thread.created_at,
            active: state.runtime().is_active(thread.id),
            generating: state.runtime().is_generating(thread.id),
            turns,
        })
    } else {
        Err(ApiError::ItemNotFound(format!(
            "Thread not found: {}",
            payload.id
        )))
    }
}

async fn materialize_turns(
    db: &mut toasty::Db,
    turns: Vec<Turn>,
) -> Result<Vec<TurnResponse>, ApiError> {
    let mut out = Vec::with_capacity(turns.len());
    for turn in turns {
        let messages =
            toasty::query!(Message FILTER .turn_id == #(turn.id) ORDER BY .message_index ASC)
                .exec(db)
                .await?;
        out.push(TurnResponse {
            id: turn.id,
            thread_id: turn.thread_id,
            turn_index: turn.turn_index,
            usage: turn.usage.0,
            finished: turn.finished,
            updated_at: turn.updated_at,
            created_at: turn.created_at,
            messages,
        });
    }
    Ok(out)
}
