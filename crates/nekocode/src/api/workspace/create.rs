use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspace {
    pub working_directory: String,
    pub name: Option<String>,
}

/// Find-or-create a workspace for the given working directory. A workspace is
/// unique per directory, so re-creating for the same path returns the existing
/// row rather than producing a duplicate.
pub async fn create_workspace(
    State(mut state): State<AppState>,
    Json(payload): Json<CreateWorkspace>,
) -> ApiResult {
    // The toasty query interpolation moves the value, so query against a clone
    // and reserve the original for the create below.
    let lookup = payload.working_directory.clone();
    if let Some(existing) =
        toasty::query!(Workspace FILTER .working_directory == #(lookup))
            .first()
            .exec(&mut state.db)
            .await?
    {
        return ApiResponse::ok(existing);
    }
    let workspace = toasty::create!(Workspace {
        working_directory: payload.working_directory,
        name: payload.name,
    })
    .exec(&mut state.db)
    .await?;
    ApiResponse::ok(workspace)
}
