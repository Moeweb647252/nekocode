use axum::routing::{get, post};

use crate::AppState;

mod fs;
mod mcp_probe;
mod models;
mod skills;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .nest("/fs", fs::router())
        .route("/models", get(models::list_models))
        .route("/mcp_probe", post(mcp_probe::probe_mcp))
        .route("/skills", get(skills::list_skills))
}
