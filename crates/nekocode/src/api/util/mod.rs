use axum::routing::get;

use crate::AppState;

mod fs;
mod models;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .nest("/fs", fs::router())
        .route("/models", get(models::list_models))
}
