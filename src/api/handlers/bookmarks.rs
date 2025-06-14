// SPDX-FileCopyrightText: 2025 Fredrik Lanker <fredrik@lanker.se>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::{AppState, PostID, TagID};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::query_builder::QueryBuilder;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

use super::tags::TagDb;

#[derive(sqlx::FromRow, Deserialize, Serialize, Debug)]
struct BookmarkDb {
    id: PostID,
    url: String,
    title: String,
    description: Option<String>,
    notes: Option<String>,
    unread: Option<bool>,
    tag_names: Option<String>,
    date_added: i64,
    date_modified: i64,
}

#[derive(sqlx::FromRow, Debug, Deserialize, Serialize)]
pub(crate) struct BookmarkRequest {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) notes: Option<String>,
    pub(crate) unread: Option<bool>,
    pub(crate) tag_names: Option<Vec<String>>,
    #[serde(skip_deserializing)]
    pub(crate) date_added: Option<i64>,
    #[serde(skip_deserializing)]
    pub(crate) date_modified: Option<i64>,
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
    pub(crate) date_added: String,
    pub(crate) date_modified: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct BookmarksResponse {
    count: usize,
    results: Vec<BookmarkResponse>,
}

impl From<BookmarkDb> for BookmarkResponse {
    fn from(val: BookmarkDb) -> Self {
        let mut tags = vec![];
        if let Some(tag_names) = val.tag_names {
            tags = tag_names.split(',').map(String::from).collect();
        }

        let added = Utc.timestamp_opt(val.date_added, 0).unwrap();
        let modified = Utc.timestamp_opt(val.date_modified, 0).unwrap();

        BookmarkResponse {
            id: val.id,
            url: val.url,
            title: val.title,
            description: val.description,
            notes: val.notes,
            unread: val.unread.unwrap_or_default(),
            tag_names: tags,
            date_added: added.to_rfc3339(),
            date_modified: modified.to_rfc3339(),
        }
    }
}

pub fn configure(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handle_get_bookmarks))
        .route("/", post(handle_post_bookmark))
        .route("/{id}", get(handle_get_bookmark))
        .route("/{id}", put(handle_put_bookmark))
        .route("/{id}", delete(handle_delete_bookmark))
        .route("/check", get(handle_check_bookmark))
        .with_state(state)
}

struct LookupType<'a> {
    id: Option<PostID>,
    url: Option<&'a str>,
}

async fn get_bookmark(state: Arc<AppState>, from: LookupType<'_>) -> Option<BookmarkResponse> {
    let mut sql: QueryBuilder<'_, sqlx::Sqlite> = QueryBuilder::new(
        r"SELECT posts.*,GROUP_CONCAT(tags.name) AS tag_names
                    FROM posts
                    LEFT OUTER JOIN post_tag ON (posts.id = post_tag.post_id)
                    LEFT OUTER JOIN tags ON (tags.id = post_tag.tag_id)",
    );

    if let Some(id) = from.id {
        sql.push(" WHERE posts.id = ");
        sql.push_bind(id);
    } else if let Some(url) = from.url {
        sql.push(" WHERE posts.url = ");
        sql.push_bind(url);
    } else {
        error!("get_bookmark called with the wrong parameters");
        return None;
    }

    sql.push(" GROUP BY posts.id");

    match sql
        .build_query_as::<BookmarkDb>()
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
struct ResponseCheckMetadata {
    url: String,
}

#[derive(Deserialize, Serialize, Debug, Default)]
struct ResponseCheck {
    bookmark: Option<BookmarkResponse>,
    metadata: Option<ResponseCheckMetadata>,
    auto_tags: Vec<String>,
}
#[derive(Deserialize)]
struct Url {
    url: String,
}
async fn handle_check_bookmark(
    State(state): State<Arc<AppState>>,
    Query(url): Query<Url>,
) -> Result<Json<ResponseCheck>, StatusCode> {
    if let Some(post) = get_bookmark(
        state,
        LookupType {
            url: Some(&url.url),
            id: None,
        },
    )
    .await
    {
        let response = ResponseCheck {
            bookmark: Some(post),
            metadata: Some(ResponseCheckMetadata { url: url.url }),
            auto_tags: vec![],
        };
        Ok(Json(response))
    } else {
        let response = ResponseCheck {
            bookmark: None,
            metadata: Some(ResponseCheckMetadata { url: url.url }),
            auto_tags: vec![],
        };
        Ok(Json(response))
    }
}

#[derive(Default)]
struct SearchQuery {
    tag_names: Vec<String>,
    text: Vec<String>,
}

fn parse_search(query: &str) -> SearchQuery {
    let tokens = query.split_whitespace();

    let mut tags = vec![];
    let mut text = vec![];
    for token in tokens {
        if token.starts_with('#') {
            let mut chars = token.chars();
            chars.next();
            tags.push(chars.as_str().to_owned());
        } else {
            text.push(token.to_owned());
        }
    }

    SearchQuery {
        tag_names: tags,
        text,
    }
}

// bookmarks?q=#audio namen&unread=yes
#[derive(Deserialize, Default)]
pub(crate) struct BookmarkQuery {
    pub(crate) q: Option<String>,
    pub(crate) limit: Option<u32>,
    pub(crate) offset: Option<u32>,
    pub(crate) unread: Option<String>,
}

pub(crate) async fn get_bookmarks(
    pool: &SqlitePool,
    query: BookmarkQuery,
) -> Vec<BookmarkResponse> {
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);
    let unread = query.unread.unwrap_or("no".to_owned());

    let mut sql: QueryBuilder<'_, sqlx::Sqlite> = QueryBuilder::new(
        r"
            SELECT posts.*, group_concat(tags.name) as tag_names
                FROM posts
                LEFT OUTER JOIN post_tag ON (posts.id = post_tag.post_id)
                LEFT OUTER JOIN tags ON (tags.id = post_tag.tag_id)
            ",
    );

    let search_query: SearchQuery;
    let mut have_where_clause = false;
    if let Some(q) = query.q {
        search_query = parse_search(&q);

        if !search_query.tag_names.is_empty() {
            have_where_clause = true;
            sql.push("WHERE posts.id IN (");
            sql.push(
                r"
                    SELECT post_id
                        FROM post_tag
                        WHERE tag_id IN (
                            SELECT id
                            FROM tags
                            WHERE ",
            );
            let mut first = true;
            for tag in &search_query.tag_names {
                if !first {
                    sql.push(" OR ");
                }
                first = false;
                sql.push(" name = ");
                sql.push_bind(tag);
            }
            sql.push(")");
            if search_query.text.is_empty() {
                sql.push(")");
            }
        }

        if !search_query.text.is_empty() {
            have_where_clause = true;
            if search_query.tag_names.is_empty() {
                sql.push("WHERE posts.id IN (");
            } else {
                sql.push(" INTERSECT ");
            }
            sql.push(
                r"
                    SELECT rowid
                        FROM posts_fts
                        WHERE posts_fts
                            MATCH ",
            );
            sql.push_bind(search_query.text.join(" "));
            sql.push(")");
        }
    }

    if unread == "yes" {
        sql.push(format!(
            " {} posts.unread = 1",
            if have_where_clause { "AND" } else { "WHERE" }
        ));
    }

    sql.push(
        r"
                GROUP BY posts.id
                ORDER BY posts.date_added DESC, posts.id DESC
                ",
    );

    if limit > 0 {
        sql.push(" LIMIT ");
        sql.push_bind(limit);
    }
    if offset > 0 {
        sql.push(" OFFSET ");
        sql.push_bind(offset);
    }

    match sql.build_query_as::<BookmarkDb>().fetch_all(pool).await {
        Ok(rows) => {
            let mut posts = vec![];
            for row in rows {
                let post: BookmarkResponse = row.into();
                posts.push(post);
            }
            posts
        }
        Err(err) => {
            error!("Failed to get posts: {}", err);
            vec![]
        }
    }
}

async fn handle_get_bookmarks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BookmarkQuery>,
) -> Result<Json<BookmarksResponse>, StatusCode> {
    let bookmarks = get_bookmarks(&state.pool, query).await;
    Ok(Json(BookmarksResponse {
        count: bookmarks.len(),
        results: bookmarks,
    }))
}

async fn handle_get_bookmark(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PostID>,
) -> Result<Json<BookmarkResponse>, StatusCode> {
    match get_bookmark(
        state,
        LookupType {
            id: Some(id),
            url: None,
        },
    )
    .await
    {
        Some(post) => Ok(Json(post)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn handle_delete_bookmark(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PostID>,
) -> Result<(), StatusCode> {
    match sqlx::query("DELETE from posts WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(_) => {
            info!("deleted bookmark: {}", id);
            Ok(())
        }
        Err(err) => {
            // probably the tag was already added to the post
            error!("Failed to delete bookmark: {} ({})", id, err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn add_tag_to_post(
    pool: &SqlitePool,
    post_id: PostID,
    tag_id: TagID,
) -> Result<(), StatusCode> {
    match sqlx::query("INSERT INTO post_tag (post_id, tag_id) VALUES ($1, $2)")
        .bind(post_id)
        .bind(tag_id)
        .execute(pool)
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

async fn update_tags_for_post(state: &AppState, post_id: PostID, new_tags: Vec<String>) {
    let mut old_tag_ids = sqlx::query("SELECT tag_id FROM post_tag WHERE post_id = $1")
        .bind(post_id)
        .map(|row: SqliteRow| row.get::<TagID, _>("tag_id"))
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    for tag in new_tags {
        let new_tag_id: TagID =
            match sqlx::query_as::<_, TagDb>("SELECT * FROM tags WHERE name = $1")
                .bind(&tag)
                .fetch_all(&state.pool)
                .await
            {
                Err(_) => -1,
                Ok(tags_found) => match tags_found.len() {
                    0 => {
                        match sqlx::query(
                            "INSERT INTO tags (name, date_added) VALUES ($1, unixepoch())",
                        )
                        .bind(tag)
                        .execute(&state.pool)
                        .await
                        {
                            Ok(tag) => {
                                debug!("inserted tag: {}", tag.last_insert_rowid());
                                tag.last_insert_rowid()
                            }
                            Err(err) => {
                                error!("Failed to add tag: {}", err);
                                -1
                            }
                        }
                    }
                    1 => {
                        debug!("tags_found: {:?}", tags_found);
                        tags_found[0].id
                    }
                    _ => -1,
                },
            };

        // if new tag doesn't exist among the old tags, we need to add it to post
        if old_tag_ids.contains(&new_tag_id) {
            // remove the tag from old_tag_ids
            let index = old_tag_ids.iter().position(|x| *x == new_tag_id).unwrap();
            old_tag_ids.remove(index);
        } else {
            let _ = add_tag_to_post(&state.pool, post_id, new_tag_id).await;
        }
    }

    // this should now contain all tags that should be removed from the post, and potential be
    // removed altogether
    if !old_tag_ids.is_empty() {
        for tag in old_tag_ids {
            // delete tag from post
            let _ = sqlx::query("DELETE FROM post_tag WHERE tag_id = $1 AND post_id = $2")
                .bind(tag)
                .bind(post_id)
                .execute(&state.pool)
                .await;

            // check if any other posts are using the tag
            let row = sqlx::query_as::<_, TagDb>("SELECT * FROM post_tag WHERE tag_id = $1")
                .bind(tag)
                .fetch_one(&state.pool)
                .await;

            if row.is_err() {
                // if no post are using the tag, remove it from tags too
                let _ = sqlx::query_as::<_, TagDb>("DELETE FROM tags WHERE id = $1")
                    .bind(tag)
                    .fetch_one(&state.pool)
                    .await;
            }
        }
    }
}

async fn handle_put_bookmark(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PostID>,
    Json(payload): Json<BookmarkRequest>,
) -> Result<Json<BookmarkResponse>, StatusCode> {
    // add post
    let _post = match sqlx::query(
        r"
            UPDATE posts
                SET (url, title, unread, description, notes, date_modified) = ($1, $2, $3, $4, $5, unixepoch())
                WHERE posts.id = $6
        ",
    )
    .bind(payload.url)
    .bind(payload.title)
    .bind(payload.unread.unwrap_or_default())
    .bind(payload.description.unwrap_or_default())
    .bind(payload.notes.unwrap_or_default())
    .bind(id)
    .execute(&state.pool)
    .await
    {
        Ok(post) => Ok(post),
        Err(err) => {
            error!("Failed to add bookmark: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    };

    update_tags_for_post(&state, id, payload.tag_names.unwrap_or_default()).await;

    match get_bookmark(
        state,
        LookupType {
            id: Some(id),
            url: None,
        },
    )
    .await
    {
        Some(post) => Ok(Json(post)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub(crate) async fn add_bookmark(
    pool: &SqlitePool,
    bookmark: BookmarkRequest,
) -> Result<PostID, StatusCode> {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or_default();

    // add post
    let post = match sqlx::query("INSERT INTO posts (url, title, unread, description, notes, date_added, date_modified) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(bookmark.url)
        .bind(bookmark.title)
        .bind(bookmark.unread)
        .bind(bookmark.description)
        .bind(bookmark.notes)
        .bind(bookmark.date_added.unwrap_or(now))
        .bind(bookmark.date_modified.unwrap_or(now))
        .execute(pool)
        .await
    {
        Ok(post) => post,
        Err(err) => {
            error!("Failed to add bookmark: {}", err);
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    let post_id = post.last_insert_rowid() as PostID;

    for tag in bookmark.tag_names.unwrap_or_default() {
        let _ = match sqlx::query_as::<_, TagDb>("SELECT * FROM tags WHERE name = $1")
            .bind(&tag)
            .fetch_all(pool)
            .await
        {
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
            Ok(tags_found) => match tags_found.len() {
                0 => {
                    match sqlx::query(
                        "INSERT INTO tags (name, date_added) VALUES ($1, unixepoch())",
                    )
                    .bind(tag)
                    .execute(pool)
                    .await
                    {
                        Ok(tag) => {
                            debug!("inserted tag: {}", tag.last_insert_rowid());
                            let _ = add_tag_to_post(pool, post_id, tag.last_insert_rowid()).await;
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
                    let _ = add_tag_to_post(pool, post_id, tag_id).await;
                    Ok(())
                }
                _ => Err(StatusCode::INTERNAL_SERVER_ERROR),
            },
        };
    }

    Ok(post_id)
}

async fn handle_post_bookmark(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BookmarkRequest>,
) -> impl IntoResponse {
    let post_id = match add_bookmark(&state.pool, payload).await {
        Ok(post_id) => post_id,
        Err(status) => return (StatusCode::BAD_REQUEST, Err(format!("{status}"))),
    };

    match get_bookmark(
        state,
        LookupType {
            id: Some(post_id),
            url: None,
        },
    )
    .await
    {
        Some(post) => (StatusCode::CREATED, Ok(Json(post))),
        None => (
            StatusCode::NOT_FOUND,
            Err("Failed to get the added bookmark".to_string()),
        ),
    }
}

/*********************************************************************/
/******************************* TESTS *******************************/
/*********************************************************************/
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::handlers::{
            bookmarks::BookmarkRequest,
            tags::{TagResponse, TagsResponse},
        },
        app, setup_db,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::Response,
    };
    use hyper::header;
    use tower::ServiceExt; // for `oneshot` and `ready`

    const TOKEN: &str = "abc";

    fn get_random_string(len: usize) -> String {
        let chars = "abcdefghijklmnopqrstuvwxyz";
        random_string::generate(len, chars)
    }

    struct CreatedBookmark {
        bookmark: BookmarkRequest,
        response: Response,
    }

    async fn add_post(app: Router, tags: Option<Vec<String>>, unread: bool) -> CreatedBookmark {
        let url = get_random_string(5);
        let title = format!(
            "{} {} {}",
            get_random_string(5),
            get_random_string(5),
            get_random_string(3)
        );
        let description = format!(
            "{} - {} {}",
            get_random_string(5),
            get_random_string(6),
            get_random_string(5)
        );
        let notes = format!(
            "{} {} {}",
            get_random_string(5),
            get_random_string(5),
            get_random_string(6)
        );
        let tag_names = match tags {
            Some(tags) => tags,
            None => {
                let tag1 = get_random_string(5);
                let tag2 = get_random_string(5);
                vec![tag1, tag2]
            }
        };

        // serde_json::from_value(BookmarkRequest{url, title, description: None, notes: None, unread: Some(false), tag_names: None }).unwrap())
        let bookmark_req = BookmarkRequest {
            url: url.to_owned(),
            title: title.to_owned(),
            description: Some(description),
            notes: Some(notes),
            unread: Some(unread),
            tag_names: Some(tag_names),
            date_added: None,
            date_modified: None,
        };
        let bookmark = serde_json::to_string(&bookmark_req).unwrap();
        //let bookmark = Json(&BookmarkRequest{url: url.to_owned(), title: title.to_owned(), description: None, notes: None, unread: Some(false), tag_names: None });
        // insert a post
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
                    .header(header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    //.body(Json(BookmarkRequest{url, title, description: None, notes: None, unread: Some(false), tag_names: None }))
                    .body(Body::from(bookmark))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        CreatedBookmark {
            bookmark: bookmark_req,
            response,
        }
    }

    #[tokio::test]
    async fn test_add_post() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let CreatedBookmark {
            bookmark,
            response: _response,
        } = add_post(app.clone(), None, false).await;

        // get posts
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    //.uri(format!("/api/bookmarks"))
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/{}", post.id))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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
    }

    #[tokio::test]
    async fn test_check_post() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let CreatedBookmark {
            bookmark,
            response: _response,
        } = add_post(app.clone(), None, false).await;

        // get existing post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/check?url={}", bookmark.url))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        let res_bookmark = res.bookmark.unwrap();
        assert!(bookmark.url == res_bookmark.url);
        assert!(bookmark.title == res_bookmark.title);

        let expected_tag_names = bookmark.tag_names.unwrap();
        assert!(res_bookmark.tag_names.contains(&expected_tag_names[0]));
        assert!(res_bookmark.tag_names.contains(&expected_tag_names[1]));

        // get non-existing post
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/check?url={}", get_random_string(5)))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(res.bookmark.is_none());
    }

    #[tokio::test]
    async fn test_add_tags_to_post() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let CreatedBookmark {
            bookmark,
            response: _response,
        } = add_post(app.clone(), None, false).await;

        // get existing post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/check?url={}", bookmark.url))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        let expected_tag_names = bookmark.tag_names.unwrap();
        let res_bookmark = res.bookmark.unwrap();
        assert!(res_bookmark.tag_names.contains(&expected_tag_names[0]));
        assert!(res_bookmark.tag_names.contains(&expected_tag_names[1]));
        assert!(res_bookmark.date_added == res_bookmark.date_modified);

        // update tags for post
        let new_tag = get_random_string(5);
        let bookmark_req = BookmarkRequest {
            url: bookmark.url,
            title: bookmark.title,
            description: None,
            notes: None,
            unread: Some(false),
            tag_names: Some(vec![expected_tag_names[1].clone(), new_tag.clone()]),
            date_added: None,
            date_modified: None,
        };
        let bookmark_json = serde_json::to_string(&bookmark_req).unwrap();
        // update bookmark
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/bookmarks/{}", res_bookmark.id))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
                    .header(header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .body(Body::from(bookmark_json))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        let res: BookmarkResponse = serde_json::from_str(body_str.as_str()).unwrap();

        assert!(res.tag_names.contains(&expected_tag_names[1]));
        assert!(res.tag_names.contains(&new_tag));
        // Our time resolution is 1 sec, it takes less than that to run the test so these will most
        // often be the same. Could add a sleep before updating the post, but that's a bit
        // annoying.
        //assert!(res.date_added != res.date_modified);

        // check that GET /tags/ not returning the removed tag
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/tags")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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
        let res: TagsResponse = serde_json::from_str(body_str.as_str()).unwrap();
        assert!(!res
            .results
            .iter()
            .any(|tag: &TagResponse| tag.name == expected_tag_names[0]));
        assert!(res
            .results
            .iter()
            .any(|tag: &TagResponse| tag.name == expected_tag_names[1]));
        assert!(res
            .results
            .iter()
            .any(|tag: &TagResponse| tag.name == new_tag));
    }

    #[tokio::test]
    async fn test_get_post_limit() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        add_post(app.clone(), None, false).await;
        let post1 = add_post(app.clone(), None, false).await;

        // get posts
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    //.uri(format!("/api/bookmarks"))
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 2);

        // get posts with limit
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks?limit=1")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 1);
        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post1.bookmark.title));
    }

    #[tokio::test]
    async fn test_get_post_offset() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let post1 = add_post(app.clone(), None, false).await;
        let post2 = add_post(app.clone(), None, false).await;
        let post3 = add_post(app.clone(), None, false).await;
        add_post(app.clone(), None, false).await;
        add_post(app.clone(), None, false).await;

        // get posts
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 5);

        // get posts with offset
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks?offset=2")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 3);
        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post1.bookmark.title));
        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post2.bookmark.title));
        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post3.bookmark.title));
    }

    #[tokio::test]
    async fn test_get_post_limit_offset() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        add_post(app.clone(), None, false).await;
        let post1 = add_post(app.clone(), None, false).await;
        let post2 = add_post(app.clone(), None, false).await;
        add_post(app.clone(), None, false).await;
        add_post(app.clone(), None, false).await;

        // get posts
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 5);

        // get posts with offset
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks?offset=2&limit=2")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 2);
        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post1.bookmark.title));
        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post2.bookmark.title));
    }

    #[tokio::test]
    async fn test_get_bookmark_tag() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let tag1 = vec![get_random_string(5)];
        let post1 = add_post(app.clone(), Some(tag1.clone()), false).await;
        add_post(app.clone(), None, false).await;

        // get posts
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    //.uri(format!("/api/bookmarks"))
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 2);

        // get posts with query for tags
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks?q=%23{}", tag1[0]))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 1);

        assert!(posts.results[0].tag_names.contains(&tag1[0]));
        assert!(posts.results[0].url == post1.bookmark.url);
        assert!(posts.results[0].title == post1.bookmark.title);
    }

    #[tokio::test]
    async fn test_get_bookmark_tags() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let tag1 = vec![get_random_string(5)];
        let post1 = add_post(app.clone(), Some(tag1.clone()), false).await;
        let tag2 = vec![get_random_string(5)];
        let post2 = add_post(app.clone(), Some(tag2.clone()), false).await;
        add_post(app.clone(), None, false).await;

        // get posts with query for tags
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks?q=%23{}%20%23{}", tag1[0], tag2[0]))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 2);

        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post1.bookmark.title
                && post.url == post1.bookmark.url
                && post.tag_names.contains(&tag1[0])));

        assert!(posts
            .results
            .iter()
            .any(|post| post.title == post2.bookmark.title
                && post.url == post2.bookmark.url
                && post.tag_names.contains(&tag2[0])));
    }

    #[tokio::test]
    async fn test_get_bookmark_free_text() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let post1 = add_post(app.clone(), None, false).await;
        add_post(app.clone(), None, false).await;

        // get posts with query for free text
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/bookmarks?q={}",
                        post1
                            .bookmark
                            .description
                            .unwrap()
                            .split_whitespace()
                            .nth(0)
                            .unwrap()
                    ))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 1);

        assert!(posts.results[0].url == post1.bookmark.url);
        assert!(posts.results[0].title == post1.bookmark.title);
    }

    #[tokio::test]
    async fn test_get_bookmark_tag_and_free_text() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let post1 = add_post(app.clone(), None, false).await;
        let post2 = add_post(
            app.clone(),
            Some(vec![post1.bookmark.tag_names.clone().unwrap()[0].to_owned()]),
            false,
        )
        .await;
        add_post(app.clone(), None, false).await;

        // tag used for multiple posts but free text from post2 => should get post2 only
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/bookmarks?q=%23{}%20{}",
                        post1.bookmark.tag_names.unwrap()[0],
                        post2
                            .bookmark
                            .notes
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                    ))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 1);

        assert!(posts.results[0].url == post2.bookmark.url);
        assert!(posts.results[0].title == post2.bookmark.title);
    }

    #[tokio::test]
    async fn test_get_bookmark_unread() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let post1 = add_post(app.clone(), None, false).await;
        let post2 = add_post(
            app.clone(),
            Some(vec![post1.bookmark.tag_names.clone().unwrap()[0].to_owned()]),
            true,
        )
        .await;
        add_post(app.clone(), None, false).await;

        // tag used for multiple posts but free text from post2 => should get post2 only
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks?unread=yes",))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 1);

        assert!(posts.results[0].url == post2.bookmark.url);
        assert!(posts.results[0].title == post2.bookmark.title);
    }

    #[tokio::test]
    async fn test_get_bookmark_unread_tag() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        let post1 = add_post(app.clone(), None, true).await;
        add_post(app.clone(), None, true).await;
        add_post(app.clone(), None, true).await;

        // tag used for multiple posts but free text from post2 => should get post2 only
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/bookmarks?unread=yes&q=%23{}",
                        post1.bookmark.tag_names.unwrap()[0],
                    ))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 1);

        assert!(posts
            .results
            .iter()
            .any(|post| post.url == post1.bookmark.url));
    }

    #[tokio::test]
    async fn test_delete_bookmark() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        add_post(app.clone(), None, true).await;
        add_post(app.clone(), None, true).await;

        let CreatedBookmark {
            bookmark,
            response: _response,
        } = add_post(app.clone(), None, false).await;

        // get existing post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/bookmarks/check?url={}", bookmark.url))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        // delete post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/bookmarks/{}", res.bookmark.unwrap().id))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status() == StatusCode::OK);

        // get posts
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 2);

        assert!(posts.results.iter().any(|post| post.url != bookmark.url));
    }

    #[tokio::test]
    async fn test_delete_bookmark_non_existing() {
        let pool = setup_db(true).await;
        let app = app(pool, TOKEN.to_owned());

        add_post(app.clone(), None, true).await;
        add_post(app.clone(), None, true).await;

        // delete non-existing post
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/bookmarks/12345"))
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status() == StatusCode::OK);

        // get posts
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/bookmarks")
                    .header(header::AUTHORIZATION, format!("Token {TOKEN}"))
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

        assert!(posts.results.iter().count() == 2);
    }
}
