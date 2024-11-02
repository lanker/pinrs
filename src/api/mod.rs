use crate::AppState;
use axum::Router;
use std::sync::Arc;

pub mod handlers;

pub fn configure(state: Arc<AppState>) -> Router {
    Router::new().nest("/api/", handlers::configure(state))
}
