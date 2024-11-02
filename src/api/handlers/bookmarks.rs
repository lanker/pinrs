use crate::{AppState, PostID, TagID, UserID};
use axum::extract::{Path, Query, State};
use axum::routing::post;
use axum::{routing::get, Extension};
use axum::{Json, Router};
use hyper::StatusCode;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, sqlx::FromRow, Deserialize, Serialize)]
pub(crate) struct Tag {
    pub id: TagID,
    pub user_id: UserID,
    pub name: String,
}

#[derive(sqlx::FromRow, Deserialize, Serialize, Debug)]
pub(crate) struct BookmarkDb {
    id: PostID,
    url: String,
    title: String,
    description: Option<String>,
    notes: Option<String>,
    unread: Option<bool>,
    tag_names: Option<String>,
}

#[derive(sqlx::FromRow, Deserialize, Serialize)]
pub(crate) struct BookmarkRequest {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub unread: Option<bool>,
    pub tag_names: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub(crate) struct BookmarkResponse {
    pub(crate) id: PostID,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) notes: Option<String>,
    pub(crate) unread: bool,
    pub(crate) tag_names: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct BookmarksResponse {
    count: usize,
    pub(crate) results: Vec<BookmarkResponse>,
}

impl Into<BookmarkResponse> for BookmarkDb {
    fn into(self) -> BookmarkResponse {
        let mut tags = vec![];
        if self.tag_names.is_some() {
            tags = self
                .tag_names
                .unwrap()
                .split(",")
                .map(String::from)
                .collect();
        }
        BookmarkResponse {
            id: self.id,
            url: self.url,
            title: self.title,
            description: self.description,
            notes: self.notes,
            unread: self.unread.unwrap_or_default(),
            tag_names: tags,
        }
    }
}

pub fn configure(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handle_get_bookmarks))
        .route("/", post(handle_post_bookmarks))
        .route("/:id", get(handle_get_bookmark))
        .route("/check", get(handle_check_bookmark))
        .with_state(state.clone())
}

async fn get_bookmark(
    state: Arc<AppState>,
    user_id: UserID,
    id: PostID,
) -> Option<BookmarkResponse> {
    let sql = "SELECT posts.*,GROUP_CONCAT(tags.name) AS tag_names FROM posts LEFT OUTER JOIN post_tag ON (posts.id = post_tag.post_id) LEFT OUTER JOIN tags ON (tags.id = post_tag.tag_id) WHERE posts.user_id = $1 AND posts.id = $2 GROUP BY posts.id";

    match sqlx::query_as::<_, BookmarkDb>(sql)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
    {
        Ok(row) => match row {
            Some(row) => {
                let post: BookmarkResponse = row.into();
                Some(post)
            }
            None => None,
        },

        Err(err) => {
            error!("Failed to get posts: {}", err);
            None
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Default)]
struct ResponseCheck {
    bookmark: BookmarkResponse,
    metadata: Option<String>,
    auto_tags: Vec<String>,
}
#[derive(Deserialize)]
struct Url {
    url: String,
}
async fn handle_check_bookmark(
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
    Query(url): Query<Url>,
) -> Result<Json<ResponseCheck>, StatusCode> {
    let sql = "SELECT posts.*,GROUP_CONCAT(tags.name) AS tag_names FROM posts LEFT OUTER JOIN post_tag ON (posts.id = post_tag.post_id) LEFT OUTER JOIN tags ON (tags.id = post_tag.tag_id) WHERE posts.user_id = $1 AND posts.url = $2 GROUP BY posts.id";

    match sqlx::query_as::<_, BookmarkDb>(sql)
        .bind(user_id)
        .bind(url.url)
        .fetch_optional(&state.pool)
        .await
    {
        Ok(row) => match row {
            Some(row) => {
                let post: BookmarkResponse = row.into();
                let response = ResponseCheck {
                    bookmark: post,
                    metadata: None,
                    auto_tags: vec![],
                };
                Ok(Json(response))
            }
            None => Err(StatusCode::NOT_FOUND),
        },

        Err(_err) => Err(StatusCode::NOT_FOUND),
    }
}

async fn handle_get_bookmarks(
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<BookmarksResponse>, StatusCode> {
    //let sql = "SELECT * FROM posts WHERE user_id = $1";
    let sql = "SELECT posts.*,GROUP_CONCAT(tags.name) AS tag_names FROM posts LEFT OUTER JOIN post_tag ON (posts.id = post_tag.post_id) LEFT OUTER JOIN tags ON (tags.id = post_tag.tag_id) WHERE posts.user_id = $1 GROUP BY posts.id";

    match sqlx::query_as::<_, BookmarkDb>(sql)
        .bind(user_id)
        .fetch_all(&state.pool)
        .await
    {
        Ok(rows) => {
            let mut posts = vec![];
            for row in rows {
                let post: BookmarkResponse = row.into();
                posts.push(post);
            }
            Ok(Json(BookmarksResponse {
                count: posts.len(),
                results: posts,
            }))
        }

        Err(err) => {
            error!("Failed to get posts: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn handle_get_bookmark(
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<PostID>,
) -> Result<Json<BookmarkResponse>, StatusCode> {
    match get_bookmark(state, user_id, id).await {
        Some(post) => Ok(Json(post)),
        None => Err(StatusCode::NOT_FOUND),
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

//async fn get_tags_for_post(state: &AppState, post_id: PostID) -> Vec<String> {
//    let sql = "SELECT tags.name FROM tags,post_tag WHERE post_tag.post_id = $1";
//
//    match sqlx::query(sql)
//        .bind(post_id.to_owned())
//        .fetch_all(&state.pool)
//        .await
//    {
//        Ok(rows) => {
//            let mut v: Vec<String> = vec![];
//            for row in rows {
//                v.push(row.try_get(0).unwrap());
//            }
//            v
//        }
//        Err(_err) => vec![],
//    }
//}

async fn handle_post_bookmarks(
    Extension(user_id): Extension<UserID>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BookmarkRequest>,
) -> Result<Json<BookmarkResponse>, StatusCode> {
    // add post
    let post = match sqlx::query(
        "INSERT INTO posts (user_id, url, title, unread) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(payload.url)
    .bind(payload.title)
    .bind(payload.unread)
    .execute(&state.pool)
    .await
    {
        Ok(post) => Ok(post),
        Err(err) => {
            error!("Failed to add bookmark: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    };

    let post_id = post.unwrap().last_insert_rowid() as PostID;

    for tag in payload.tag_names.unwrap_or_default() {
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

    match get_bookmark(state, user_id, post_id).await {
        Some(post) => Ok(Json(post)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api::handlers::bookmarks::BookmarkRequest, app, setup_db};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::Response,
    };
    use hyper::header;
    use sqlx::{Pool, Sqlite};
    use tower::ServiceExt; // for `oneshot` and `ready`

    fn get_random_string(len: usize) -> String {
        let chars = "abcdefghijklmnopqrstuvwxyz";
        random_string::generate(len, chars)
    }

    struct CreatedBookmark {
        bookmark: BookmarkRequest,
        response: Response,
    }

    async fn add_post(app: Router, pool: &Pool<Sqlite>, token: String) -> CreatedBookmark {
        let username = get_random_string(5);
        let _ = sqlx::query(&format!(
            "INSERT INTO users (username, token) VALUES ('{}', '{}')",
            username, token
        ))
        .execute(pool)
        .await;

        let url = get_random_string(5);
        let title = get_random_string(5);
        let tag1 = get_random_string(5);
        let tag2 = get_random_string(5);

        // serde_json::from_value(BookmarkRequest{url, title, description: None, notes: None, unread: Some(false), tag_names: None }).unwrap())
        let bookmark_req = BookmarkRequest {
            url: url.to_owned(),
            title: title.to_owned(),
            description: None,
            notes: None,
            unread: Some(false),
            tag_names: Some(vec![tag1.clone(), tag2.clone()]),
        };
        let bookmark = serde_json::to_string(&bookmark_req).unwrap();
        //let bookmark = Json(&BookmarkRequest{url: url.to_owned(), title: title.to_owned(), description: None, notes: None, unread: Some(false), tag_names: None });
        // insert a post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {token}"))
                    .header(header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    //.body(Json(BookmarkRequest{url, title, description: None, notes: None, unread: Some(false), tag_names: None }))
                    .body(Body::from(bookmark))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        CreatedBookmark {
            bookmark: bookmark_req,
            response,
        }
    }

    #[tokio::test]
    async fn test_add_post() {
        let pool = setup_db(true).await;
        let app = app(pool.clone()).await;
        let token = get_random_string(5);

        let CreatedBookmark {
            bookmark,
            response: _response,
        } = add_post(app.clone(), &pool, token.clone()).await;

        // get posts
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    //.uri(format!("/api/bookmarks"))
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        let posts: BookmarksResponse = serde_json::from_str(body_str.as_str()).unwrap();

        let post = posts
            .results
            .iter()
            .find(|post| post.url == bookmark.url && post.title == bookmark.title);

        assert!(post.is_some());

        let post = post.unwrap();

        // get post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/{}", post.id))
                    .header(header::AUTHORIZATION, format!("Token {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status() == StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        let res: BookmarkResponse = serde_json::from_str(body_str.as_str()).unwrap();

        assert!(post.url == res.url && post.title == res.title);

        //// get tags
        //let response = app
        //    .clone()
        //    .oneshot(
        //        Request::builder()
        //            .uri(format!("/api/tags"))
        //            .header(header::AUTHORIZATION, format!("Token {token}"))
        //            .body(Body::empty())
        //            .unwrap(),
        //    )
        //    .await
        //    .unwrap();
        //
        //let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        //    .await
        //    .unwrap();
        //let body_str = String::from_utf8(body.to_vec()).unwrap();
        //let tags: Vec<Tag> = serde_json::from_str(&body_str.as_str()).unwrap();
        //
        //assert!(tags.iter().any(|tag| tag.name == tag1));
        //assert!(tags.iter().any(|tag| tag.name == tag2));
    }

    #[tokio::test]
    async fn test_check_post() {
        let pool = setup_db(true).await;
        let app = app(pool.clone()).await;
        let token = get_random_string(5);

        let CreatedBookmark {
            bookmark,
            response: _response,
        } = add_post(app.clone(), &pool, token.clone()).await;

        // get existing post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/check?url={}", bookmark.url))
                    .header(header::AUTHORIZATION, format!("Token {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status() == StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        let res: ResponseCheck = serde_json::from_str(body_str.as_str()).unwrap();

        assert!(bookmark.url == res.bookmark.url && bookmark.title == res.bookmark.title);


        // get non-existing post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/check?url={}", get_random_string(5)))
                    .header(header::AUTHORIZATION, format!("Token {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status() == StatusCode::NOT_FOUND);
    }
}
