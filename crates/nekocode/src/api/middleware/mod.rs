pub mod create;
pub mod delete;
pub mod list;
pub mod shell;
pub mod update;

use axum::routing::post;

use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/create", post(create::create_middleware))
        .route("/list", post(list::list_middlewares))
        .route("/update", post(update::update_middleware))
        .route("/delete", post(delete::delete_middleware))
        .nest("/shell", shell::router())
}
