use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct UpdateWorkspace {
    pub id: u64,
    pub name: Option<String>,
}

pub async fn update_workspace(
    State(state): State<AppState>,
    Json(payload): Json<UpdateWorkspace>,
) -> ApiResult {
    let mut db = state.db();
    let mut update = toasty::query!(Workspace FILTER .id == #(payload.id)).update();
    if let Some(name) = payload.name {
        update.set_name(name);
    }
    update.exec(&mut db).await?;
    ApiResponse::ok(())
}
