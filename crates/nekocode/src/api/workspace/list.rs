use crate::api::prelude::*;

/// A workspace with its threads materialized, returned by `/workspace/list` and
/// `/workspace/get`. We build a DTO (rather than serializing `Workspace`
/// directly) because a toasty `Deferred` relation serializes as `null` unless
/// explicitly resolved with `.get()`; the DTO calls `.get()` to flatten the
/// threads into the response the sidebar renders.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    pub id: u64,
    pub working_directory: String,
    pub name: Option<String>,
    pub updated_at: jiff::Timestamp,
    pub created_at: jiff::Timestamp,
    pub threads: Vec<Thread>,
}

pub async fn list_workspaces(State(mut state): State<AppState>) -> ApiResult {
    let workspaces = toasty::query!(Workspace)
        .include(Workspace::fields().threads())
        .exec(&mut state.db)
        .await?;
    let items = workspaces
        .into_iter()
        .map(|ws| WorkspaceResponse {
            threads: ws.threads.get().to_owned(),
            id: ws.id,
            working_directory: ws.working_directory,
            name: ws.name,
            updated_at: ws.updated_at,
            created_at: ws.created_at,
        })
        .collect::<Vec<_>>();
    ApiResponse::ok(items)
}
