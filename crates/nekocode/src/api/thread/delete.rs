use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteThread {
    pub id: u64,
}

pub async fn delete_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<DeleteThread>,
) -> ApiResult {
    // Refuse to delete a thread that is mid-generation.
    if state.generate_states.contains_key(&payload.id) {
        return Err(ApiError::ThreadGenerating);
    }
    let turns = toasty::query!(Turn FILTER .thread_id == #(payload.id)).exec(&mut state.db).await?;
    let mut transaction = state.db.transaction().await?;
    for turn in turns {
        toasty::query!(Message FILTER .turn_id == #(turn.id))
            .delete()
            .exec(&mut transaction)
            .await?;
    }
    toasty::query!(Turn FILTER .thread_id == #(payload.id))
        .delete()
        .exec(&mut transaction)
        .await?;
    // Middleware rows reference the thread and would otherwise be orphaned.
    toasty::query!(Middleware FILTER .thread_id == #(payload.id))
        .delete()
        .exec(&mut transaction)
        .await?;
    toasty::query!(Thread FILTER .id == #(payload.id))
        .delete()
        .exec(&mut transaction)
        .await?;
    transaction.commit().await?;
    // Drop any in-memory activated agent for this thread so its shells /
    // extensions don't linger as a "ghost".
    state.active_threads.remove(&payload.id);
    ApiResponse::ok(())
}
