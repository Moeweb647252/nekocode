pub mod list;

pub fn router() -> axum::Router<crate::AppState> {
    axum::Router::new()
}
