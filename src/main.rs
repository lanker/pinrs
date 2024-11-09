use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router, ServiceExt,
};
use hyper::header::{self};
use log::error;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::sync::Arc;
use tower::Layer;
use tower_http::cors::CorsLayer;

use tower_http::normalize_path::NormalizePathLayer;

pub mod api;

type UserID = i64;
type PostID = UserID;
type TagID = PostID;

#[derive(sqlx::FromRow, Deserialize, Serialize)]
struct User {
    id: UserID,
    username: String,
    token: String,
}

pub struct AppState {
    pool: SqlitePool,
}

async fn auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let sql = "SELECT * FROM users WHERE token = $1";

    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_value| {
            auth_value
                .strip_prefix("Token ")
                .map(|stripped| stripped.to_owned())
        });

    match sqlx::query_as::<_, User>(sql)
        .bind(token.as_ref())
        .fetch_all(&state.pool)
        .await
    {
        Ok(users) => match users.len() {
            1 => {
                req.extensions_mut().insert(users[0].id);
                Ok(next.run(req).await)
            }
            _ => Err(StatusCode::UNAUTHORIZED),
        },
        Err(err) => {
            error!("Failed to authenticate: {}", err);
            Err(StatusCode::UNAUTHORIZED)
        }
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
                "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, username INTEGER NOT NULL UNIQUE, token TEXT NOT NULL);",
            )
            .execute(&pool)
            .await;

    let _ = sqlx::query(
        "CREATE TABLE IF NOT EXISTS posts ( id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, url TEXT NOT NULL UNIQUE, title TEXT NOT NULL, description TEXT, notes TEXT, unread BOOLEAN, shared TEXT, toread TEXT, hash TEXT, meta TEXT);"
            )
            .execute(&pool)
            .await;

    let _ = sqlx::query(
        "CREATE TABLE IF NOT EXISTS post_tag ( post_id INTEGER NOT NULL, tag_id INTEGER NOT NULL, UNIQUE(post_id, tag_id));"
            )
            .execute(&pool)
            .await;

    let _ = sqlx::query("CREATE TABLE IF NOT EXISTS tags ( id INTEGER PRIMARY KEY, user_id INTEGER, name INTEGER NOT NULL, UNIQUE(user_id, name));")
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

    fn get_random_string(len: usize) -> String {
        let chars = "abcdefghijklmnopqrstuvwxyz";
        random_string::generate(len, chars)
    }

    #[tokio::test]
    async fn auth() {
        let username = get_random_string(5);
        let token = get_random_string(5);
        let pool = setup_db(true).await;
        let _ = sqlx::query(&format!(
            "INSERT INTO users (username, token) VALUES ('{}', '{}')",
            username, token
        ))
        .execute(&pool)
        .await;

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
                    .header(header::AUTHORIZATION, format!("Token {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
