use axum::{
    extract::{Extension, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Json,
    response::Response,
    routing::get,
    Router,
};
use log::error;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

type UserID = u32;

#[derive(sqlx::FromRow, Deserialize, Serialize)]
struct User {
    id: UserID,
    username: String,
    token: String,
}

#[derive(sqlx::FromRow, Deserialize, Serialize)]
struct Post {
    url: String,
    description: String,
}

struct AppState {
    pool: SqlitePool,
}

#[derive(Clone)]
struct UserId(&'static str);

async fn auth<B>(
    State(state): State<Arc<AppState>>,
    mut req: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    let sql = "SELECT * FROM users WHERE token = $1";

    let params: HashMap<String, String> = req
        .uri()
        .query()
        .map(|v| {
            url::form_urlencoded::parse(v.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_else(HashMap::new);

    let token = params
        .get("token")
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("".to_owned()));

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
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn add_entry(
    Query(params): Query<Post>,
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
) -> Result<(), StatusCode> {
    let sql = "INSERT INTO posts (user_id, url, description) VALUES ($1, $2, $3)".to_string();

    match sqlx::query(&sql)
        .bind(user_id)
        .bind(params.url)
        .bind(params.description)
        .execute(&state.pool)
        .await
    {
        Ok(_) => Ok(()),
        Err(err) => {
            error!("Failed to add post: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_entries(
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Post>>, StatusCode> {
    let sql = "SELECT * FROM posts WHERE user_id = $1";

    match sqlx::query_as::<_, Post>(sql)
        .bind(user_id)
        .fetch_all(&state.pool)
        .await
    {
        Ok(posts) => Ok(Json(posts)),
        Err(err) => {
            error!("Failed to get posts: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    env_logger::init();

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite://pinrs.db")
        .await?;

    let state = Arc::new(AppState { pool });

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/v1/posts/add", get(add_entry))
        .route("/v1/posts/all", get(get_entries))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
