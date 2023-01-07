use axum::{
    extract::{Extension, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Json,
    response::Response,
    routing::get,
    Router,
};
use log::{error, info, debug};
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

#[derive(Debug, sqlx::FromRow, Deserialize, Serialize)]
struct Tag {
    id: u32,
    user_id: u32,
    name: String,
}

#[derive(sqlx::FromRow, Deserialize, Serialize)]
struct Post {
    url: String,
    description: String,
}

#[derive(sqlx::FromRow, Deserialize, Serialize)]
struct PostRequest {
    url: String,
    description: String,
    tags: Option<String>,
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

async fn add_tag_to_post(state: &AppState, post_id: u32, tag_id: u32) -> Result<(), StatusCode> {
    match sqlx::query("INSERT INTO post_tag (post_id, tag_id) VALUES ($1, $2)")
        .bind(post_id)
        .bind(tag_id)
        .execute(&state.pool)
        .await
    {
        Ok(_) => {
            info!("inserted tag for post: {}, {}", post_id, tag_id);
            Ok(())
        }
        Err(err) => {
            // probably the tag was already added to the post
            error!(
                "Failed to add tag to post: {} {} ({})",
                post_id, tag_id, err
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn posts_add(
    Query(params): Query<PostRequest>,
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
) -> Result<(), StatusCode> {
    // add post
    let post =
        match sqlx::query("INSERT INTO posts (user_id, url, description) VALUES ($1, $2, $3)")
            .bind(user_id)
            .bind(params.url)
            .bind(params.description)
            .execute(&state.pool)
            .await
        {
            Ok(post) => Ok(post),
            Err(err) => {
                error!("Failed to add post: {}", err);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        };

    let post_id = post.unwrap().last_insert_rowid() as u32;

    // check/add tags
    let tags: Vec<String> = match params.tags {
        Some(tags) => tags.split(' ').map(|s| s.to_owned()).collect(),
        None => vec![],
    };
    for tag in tags {
        let _ =
            match sqlx::query_as::<_, Tag>("SELECT * FROM tags WHERE user_id = $1 AND name = $2")
                .bind(user_id)
                .bind(&tag)
                .fetch_all(&state.pool)
                .await
            {
                Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
                Ok(tags_found) => match tags_found.len() {
                    0 => {
                        match sqlx::query("INSERT INTO tags (user_id, name) VALUES ($1, $2)")
                            .bind(user_id)
                            .bind(tag)
                            .execute(&state.pool)
                            .await
                        {
                            Ok(tag) => {
                                debug!("inserted tag: {}", tag.last_insert_rowid());
                                let _ = add_tag_to_post(
                                    &state.clone(),
                                    post_id,
                                    tag.last_insert_rowid() as u32,
                                )
                                .await;
                                Ok(())
                            }
                            Err(err) => {
                                error!("Failed to add tag: {}", err);
                                Err(StatusCode::INTERNAL_SERVER_ERROR)
                            }
                        }
                    }
                    1 => {
                        let tag_id = tags_found[0].id;
                        debug!("tags_found: {:?}", tags_found);
                        let _ = add_tag_to_post(&state.clone(), post_id, tag_id).await;
                        Ok(())
                    }
                    _ => Err(StatusCode::INTERNAL_SERVER_ERROR),
                },
            };
    }

    Ok(())
}

async fn posts_all(
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
        .route("/v1/posts/add", get(posts_add))
        .route("/v1/posts/all", get(posts_all))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
