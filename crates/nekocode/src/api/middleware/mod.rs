pub mod list;
pub mod shell;
pub mod update;

use axum::routing::post;

use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/list", post(list::list_middlewares))
        .route("/update", post(update::update_middleware))
        .nest("/shell", shell::router())
}
