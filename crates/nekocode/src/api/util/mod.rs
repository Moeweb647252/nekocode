use crate::AppState;

mod fs;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().nest("/fs", fs::router())
}
