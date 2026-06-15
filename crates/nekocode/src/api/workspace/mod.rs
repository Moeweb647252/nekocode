use axum::routing::{get, post};

use crate::AppState;

pub mod create;
pub mod delete;
pub mod get;
pub mod list;
pub mod update;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/create", post(create::create_workspace))
        .route("/list", get(list::list_workspaces))
        .route("/get", post(get::get_workspace))
        .route("/update", post(update::update_workspace))
        .route("/delete", post(delete::delete_workspace))
}
