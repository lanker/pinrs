use crate::AppState;
use axum::Router;
use std::sync::Arc;
pub mod bookmarks;

pub fn configure(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/bookmarks", bookmarks::configure(state))
}
