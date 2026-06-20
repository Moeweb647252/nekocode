use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThread {
    pub working_directory: String,
}

#[axum::debug_handler]
pub async fn create_thread(
    State(mut state): State<AppState>,
    Json(payload): Json<CreateThread>,
) -> ApiResult {
    let model = {
        let config = state.config.read().await;
        config.default_model.clone()
    };
    // Ensure a workspace owns this working directory before creating the thread,
    // then link the thread to it. `working_directory` is denormalized onto the
    // thread too so the agent run-loop / get_thread need no join.
    let workspace =
        nekocode_entities::workspace::find_or_create(&mut state.db, &payload.working_directory)
            .await?;
    let thread = toasty::create!(Thread {
        working_directory: payload.working_directory.clone(),
        model: model,
        workspace_id: Some(workspace.id),
    })
    .exec(&mut state.db)
    .await?;
    // Seed both middlewares with the thread's working directory so shell
    // commands and file tools are scoped to the project root. Config values
    // are built from the typed config structs via `to_value()` so serialization
    // stays locked to the config definition (no hand-written `json!({...})`
    // literals that could drift from the struct).
    let shell_cfg = nekocode_shell::config::ShellConfig {
        working_directory: Some(payload.working_directory.clone()),
        ..Default::default()
    }
    .to_value();
    toasty::create!(Middleware {
        thread_id: thread.id,
        name: "shell".to_string(),
        config: toasty::Json(shell_cfg),
    })
    .exec(&mut state.db)
    .await?;
    let tool_cfg = nekocode_file::config::FileConfig {
        working_directory: Some(payload.working_directory),
    }
    .to_value();
    toasty::create!(Middleware {
        thread_id: thread.id,
        name: "tool".to_string(),
        config: toasty::Json(tool_cfg),
    })
    .exec(&mut state.db)
    .await?;
    ApiResponse::ok(thread)
}
