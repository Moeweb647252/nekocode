pub mod list;

use axum::routing::post;

pub fn router() -> axum::Router<crate::AppState> {
    axum::Router::new().route("/list", post(list::list_shells))
}
