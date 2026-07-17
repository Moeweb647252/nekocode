use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct GetWorkspace {
    pub id: u64,
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Json(payload): Json<GetWorkspace>,
) -> ApiResult {
    let mut db = state.db();
    let ws = toasty::query!(Workspace FILTER .id == #(payload.id))
        .include(Workspace::fields().threads())
        .first()
        .exec(&mut db)
        .await?
        .ok_or(ApiError::ItemNotFound(format!(
            "Workspace not found: {}",
            payload.id
        )))?;
    let threads = ws.threads.get().to_owned();
    ApiResponse::ok(crate::api::workspace::list::WorkspaceResponse {
        id: ws.id,
        working_directory: ws.working_directory,
        name: ws.name,
        updated_at: ws.updated_at,
        created_at: ws.created_at,
        threads,
    })
}
