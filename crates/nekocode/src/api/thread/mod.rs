use axum::routing::{get, post};

use crate::AppState;

pub mod activate;
pub mod create;
pub mod delete;
pub mod get;
pub mod list;
pub mod update;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/create", post(create::create_thread))
        .route("/list", get(list::list_threads))
        .route("/delete", post(delete::delete_thread))
        .route("/activate", post(activate::activate_thread))
        .route("/update", post(update::update_thread))
        .route("/get", post(get::get_thread))
}
