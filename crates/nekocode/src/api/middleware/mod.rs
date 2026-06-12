pub mod shell;

pub fn router() -> axum::Router<crate::AppState> {
    axum::Router::new().nest("/shell", shell::router())
}
