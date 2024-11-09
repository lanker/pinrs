use crate::{AppState, PostID, TagID};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use hyper::StatusCode;
use log::error;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, sqlx::FromRow, Deserialize, Serialize)]
pub(crate) struct TagDb {
    pub(crate) id: TagID,
    pub(crate) name: String,
    pub(crate) date_added: i64,
}

#[derive(sqlx::FromRow, Deserialize, Serialize, Debug, Default)]
pub(crate) struct TagResponse {
    pub(crate) id: PostID,
    pub(crate) name: String,
    pub(crate) date_added: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct TagsResponse {
    count: usize,
    pub(crate) results: Vec<TagResponse>,
}

impl From<TagDb> for TagResponse {
    fn from(val: TagDb) -> Self {
        let added = Utc.timestamp_opt(val.date_added, 0).unwrap();

        TagResponse {
            id: val.id,
            name: val.name,
            date_added: added.to_rfc3339(),
        }
    }
}
pub fn configure(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handle_get_tags))
        .with_state(state.clone())
}

async fn handle_get_tags(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TagsResponse>, StatusCode> {
    let sql = "SELECT * FROM tags";

    match sqlx::query_as::<_, TagDb>(sql).fetch_all(&state.pool).await {
        Ok(rows) => {
            let mut tags = vec![];
            for row in rows {
                let tag: TagResponse = row.into();
                tags.push(tag);
            }
            Ok(Json(TagsResponse {
                count: tags.len(),
                results: tags,
            }))
        }

        Err(err) => {
            error!("Failed to get tags: {}", err);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
