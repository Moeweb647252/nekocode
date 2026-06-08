use crate::AppState;

pub mod get_dirs;
pub mod list_dir;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/list_dir", axum::routing::post(list_dir::list_dir))
        .route("/dirs", axum::routing::get(get_dirs::get_dirs))
}
