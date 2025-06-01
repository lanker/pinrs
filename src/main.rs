// SPDX-FileCopyrightText: 2025 Fredrik Lanker <fredrik@lanker.se>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router, ServiceExt,
};
use clap::Parser;
use directories::ProjectDirs;
use hyper::header::{self};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::ConnectOptions;
use std::fs;
use std::str::FromStr;
use std::sync::Arc;
use std::{env, path::Path};
use tower::Layer;
use tower_http::cors::CorsLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::trace::TraceLayer;
use tracing::error;

pub mod api;
mod import;

type PostID = i64;
type TagID = PostID;

pub struct AppState {
    pool: SqlitePool,
    token: String,
}

#[derive(Parser)]
pub struct Arguments {
    #[arg(long)]
    import: Option<String>,
    #[arg(long = "export-html")]
    export_html: bool,
}

async fn auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let mut token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_value| {
            auth_value
                .strip_prefix("Token ")
                .map(|stripped| stripped.to_owned())
        });

    if token.is_none() {
        token = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|auth_header| auth_header.to_str().ok())
            .and_then(|auth_value| {
                auth_value
                    .strip_prefix("Bearer ")
                    .map(|stripped| stripped.to_owned())
            });
    }

    if token.is_none() {
        error!("No token");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = token.unwrap();

    if token == state.token {
        Ok(next.run(req).await)
    } else {
        error!("Failed to authenticate with token: {}", token);
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub(crate) async fn setup_db(memory: bool) -> SqlitePool {
    let db_path = if memory {
        "sqlite::memory:".to_owned()
    } else if let Ok(env_db) = env::var("PINRS_DB") {
        let path = Path::new(&env_db);
        let dir = path.parent().expect("Couldn't get directory of database");
        match fs::create_dir_all(dir) {
            Ok(_) => format!("sqlite://{}?mode=rwc", env_db),
            Err(err) => panic!("Failed to create database: {}", err),
        }
    } else {
        match ProjectDirs::from("se", "lanker", "pinrs") {
            Some(base_dirs) => {
                let dir = base_dirs.data_dir();
                match fs::create_dir_all(dir) {
                    Ok(_) => format!(
                        "sqlite://{}/pinrs.db?mode=rwc",
                        dir.to_string_lossy().into_owned()
                    ),
                    Err(_) => "sqlite://pinrs.db?mode=rwc".to_owned(),
                }
            }
            None => "sqlite://pinrs.db?mode=rwc".to_owned(),
        }
    };

    println!("Using database: {}", db_path);

    let options = SqliteConnectOptions::from_str(&db_path)
        .expect("Failed to parse database string")
        .create_if_missing(true)
        .log_statements(tracing::log::LevelFilter::Debug);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .expect("Failed to connect to database");

    let _ = sqlx::query(
        r#"
            CREATE TABLE IF NOT EXISTS posts (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                description TEXT,
                notes TEXT,
                unread BOOLEAN,
                date_added INTEGER,
                date_modified INTEGER
            );
        "#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"
            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                date_added INTEGER
             );
        "#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"
            CREATE TABLE IF NOT EXISTS post_tag (
                post_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                UNIQUE(post_id, tag_id),
                FOREIGN KEY(post_id) REFERENCES posts(id) ON DELETE CASCADE,
                FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
            );
        "#,
    )
    .execute(&pool)
    .await;

    // ---------------------- FTS
    let _ = sqlx::query(
        r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS posts_fts USING fts5(
                url,
                title,
                description,
                notes,
                unread UNINDEXED,
                date_added UNINDEXED,
                date_modified UNINDEXED,
                content='posts',
                content_rowid='id'
            );
        "#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"
            CREATE TRIGGER IF NOT EXISTS posts_ai AFTER INSERT ON posts
                BEGIN
                    INSERT INTO posts_fts (rowid, url, title, description, notes)
                    VALUES (new.id, new.url, new.title, new.description, new.notes);
                END;
    "#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"
            CREATE TRIGGER IF NOT EXISTS posts_ad AFTER DELETE ON posts
                BEGIN
                    INSERT INTO posts_fts (posts_fts, rowid, url, title, description, notes)
                    VALUES ('delete', old.id, old.url, old.title, old.description, old.notes);
                END;
    "#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"
            CREATE TRIGGER IF NOT EXISTS posts_au AFTER UPDATE ON posts
                BEGIN
                    INSERT INTO posts_fts (posts_fts, rowid, url, title, description, notes)
                    VALUES ('delete', old.id, old.url, old.title, old.description, old.notes);
                    INSERT INTO posts_fts (rowid, url, title, description, notes)
                    VALUES (new.id, new.url, new.title, new.description, new.notes);
                END;
    "#,
    )
    .execute(&pool)
    .await;

    pool
}

pub(crate) async fn app(pool: SqlitePool, token: String) -> Router {
    let state = Arc::new(AppState { pool, token });

    let router = crate::api::configure(state.clone());

    router
        .route_layer(middleware::from_fn_with_state(state, auth))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    env_logger::init();

    let pool = setup_db(false).await;

    let args = Arguments::parse();
    if args.import.is_some() {
        import::import(args.import.unwrap(), &pool).await?;
        return Ok(());
    } else if args.export_html {
        import::export_html(&pool).await?;
        return Ok(());
    }

    let token = env::var("PINRS_TOKEN").expect("Need to set environment variable PINRS_TOKEN");
    let port = env::var("PINRS_PORT").unwrap_or("3000".to_owned());

    let app = app(pool, token).await;

    let app = NormalizePathLayer::trim_trailing_slash().layer(app);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("Failed to bind to port");
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .await
        .expect("Failed to create server");

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
    use tower::ServiceExt;

    #[tokio::test]
    async fn auth_token() {
        let pool = setup_db(true).await;
        let app = app(pool, "abc".to_owned()).await;

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
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks"))
                    .header(header::AUTHORIZATION, "Token abc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_bearer() {
        let pool = setup_db(true).await;
        let app = app(pool, "abc".to_owned()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Bearer 123"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks"))
                    .header(header::AUTHORIZATION, "Token abc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
