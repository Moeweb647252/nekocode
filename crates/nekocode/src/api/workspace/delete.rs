use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteWorkspace {
    pub id: u64,
}

/// Delete a workspace and cascade through every thread it owns (Messages →
/// Turns → Middlewares → Thread), then the workspace row. Mirrors the cascade
/// in `thread/delete.rs` but applied to all threads in the workspace.
pub async fn delete_workspace(
    State(mut state): State<AppState>,
    Json(payload): Json<DeleteWorkspace>,
) -> ApiResult {
    let threads = toasty::query!(Thread FILTER .workspace_id == #(payload.id))
        .exec(&mut state.db)
        .await?;
    // Refuse if any thread in the workspace is mid-generation.
    for t in &threads {
        if state.generate_states.contains_key(&t.id) {
            return Err(ApiError::ThreadGenerating);
        }
    }
    let mut transaction = state.db.transaction().await?;
    for t in &threads {
        let turns = toasty::query!(Turn FILTER .thread_id == #(t.id))
            .exec(&mut transaction)
            .await?;
        for turn in turns {
            toasty::query!(Message FILTER .turn_id == #(turn.id))
                .delete()
                .exec(&mut transaction)
                .await?;
        }
        toasty::query!(Turn FILTER .thread_id == #(t.id))
            .delete()
            .exec(&mut transaction)
            .await?;
        toasty::query!(Middleware FILTER .thread_id == #(t.id))
            .delete()
            .exec(&mut transaction)
            .await?;
        toasty::query!(Thread FILTER .id == #(t.id))
            .delete()
            .exec(&mut transaction)
            .await?;
    }
    toasty::query!(Workspace FILTER .id == #(payload.id))
        .delete()
        .exec(&mut transaction)
        .await?;
    transaction.commit().await?;
    // Drop any in-memory activated agents for the removed threads.
    for t in &threads {
        state.active_threads.remove(&t.id);
    }
    ApiResponse::ok(())
}
