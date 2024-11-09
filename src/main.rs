use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router, ServiceExt,
};
use hyper::header::{self};
use log::error;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::sync::Arc;
use tower::Layer;
use tower_http::cors::CorsLayer;

use tower_http::normalize_path::NormalizePathLayer;

pub mod api;

type PostID = i64;
type TagID = PostID;

pub struct AppState {
    pool: SqlitePool,
}

// TODO: get from command line args
pub const TOKEN: &str = "abc";

async fn auth(
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_value| {
            auth_value
                .strip_prefix("Token ")
                .map(|stripped| stripped.to_owned())
        });

    if token == Some(TOKEN.to_owned()) {
        Ok(next.run(req).await)
    } else {
        error!(
            "Failed to authenticate with token: {}",
            token.unwrap_or_default()
        );
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub(crate) async fn setup_db(memory: bool) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect(if memory {
            "sqlite::memory:"
        } else {
            "sqlite://pinrs.db?mode=rwc"
        })
        .await
        .unwrap();

    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS posts (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                description TEXT,
                notes TEXT,
                unread BOOLEAN,
                date_added INTEGER,
                date_modified INTEGER
            );"#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE
             );"#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS post_tag (
                post_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                UNIQUE(post_id, tag_id),
                FOREIGN KEY(post_id) REFERENCES posts(id) ON DELETE CASCADE,
                FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
            );"#,
    )
    .execute(&pool)
    .await;

    pool
}

pub(crate) async fn app(pool: SqlitePool) -> Router {
    let state = Arc::new(AppState { pool });

    let router = crate::api::configure(state.clone());

    router
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(CorsLayer::permissive())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    env_logger::init();
    let pool = setup_db(false).await;
    let app = app(pool).await;

    let app = NormalizePathLayer::trim_trailing_slash().layer(app);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .await
        .unwrap();
    //axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
    //    .serve(app.await.into_make_service())
    //    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use hyper::header;
    use tower::ServiceExt; // for `oneshot` and `ready`

    #[tokio::test]
    async fn auth() {
        let pool = setup_db(true).await;
        let app = app(pool.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token 123"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks"))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
