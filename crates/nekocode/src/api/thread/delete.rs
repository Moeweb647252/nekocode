use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteThread {
    pub id: u64,
}

pub async fn delete_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<DeleteThread>,
) -> ApiResult {
    let turns = toasty::query!(Turn FILTER .thread_id == #(payload.id))
        .exec(&mut state.db)
        .await?;
    let mut transction = state.db.transaction().await?;
    for turn in turns {
        toasty::query!(Message FILTER .turn_id == #(turn.id))
            .delete()
            .exec(&mut transction);
    }
    toasty::query!(Turn FILTER .thread_id == #(payload.id))
        .delete()
        .exec(&mut transction);
    toasty::query!(Thread FILTER .id == #(payload.id))
        .delete()
        .exec(&mut transction);
    transction.commit().await?;
    ApiResponse::ok(())
}
