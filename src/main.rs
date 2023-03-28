use axum::{
    extract::{Extension, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Json,
    response::Response,
    routing::get,
    Router,
};
use log::{debug, error, info};
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

#[derive(sqlx::FromRow, Debug, Deserialize, Serialize)]
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
            Err(StatusCode::UNAUTHORIZED)
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

async fn tags_get(
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Tag>>, StatusCode> {
    let sql = "SELECT * FROM tags WHERE user_id = $1";

    match sqlx::query_as::<_, Tag>(sql)
        .bind(user_id)
        .fetch_all(&state.pool)
        .await
    {
        // return number of times used
        Ok(tags) => Ok(Json(tags)),
        Err(err) => {
            error!("Failed to get tags: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn setup_db(memory: bool) -> SqlitePool {
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
        "CREATE TABLE IF NOT EXISTS posts ( id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, url TEXT NOT NULL UNIQUE, description TEXT NOT NULL, extended TEXT, time TEXT, shared TEXT, toread TEXT, hash TEXT, meta TEXT);"
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

async fn app(pool: SqlitePool) -> Router {
    let state = Arc::new(AppState { pool });

    Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/v1/posts/add", get(posts_add))
        .route("/v1/posts/all", get(posts_all))
        .route("/v1/tags/get", get(tags_get))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    env_logger::init();
    let pool = setup_db(false).await;
    let app = app(pool);
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.await.into_make_service())
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
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
            username,
            token
        ))
        .execute(&pool)
        .await;

        let app = app(pool.clone()).await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/tags/get?token=123")
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
                    .uri(format!("/v1/tags/get?token={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn add_post() {
        let username = get_random_string(5);
        let token = get_random_string(5);
        let pool = setup_db(true).await;
        let _ = sqlx::query(&format!(
            "INSERT INTO users (username, token) VALUES ('{}', '{}')",
            username,
            token
        ))
        .execute(&pool)
        .await;

        let url = get_random_string(5);
        let description = get_random_string(5);
        // insert a post
        let app = app(pool.clone()).await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/posts/add?token={token}&url={url}&description={description}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/posts/all?token={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();


        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let posts: Vec<Post> = serde_json::from_slice(&body).unwrap();

        let mut found = false;
        for post in posts {
            if post.url == url && post.description == description {
                found = true;
                break;
            }
        }
        assert!(found);
    }
}
