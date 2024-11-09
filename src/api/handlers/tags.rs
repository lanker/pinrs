use crate::{AppState, PostID, TagID };
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use hyper::StatusCode;
use log::error;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, sqlx::FromRow, Deserialize, Serialize)]
pub(crate) struct TagDb {
    pub id: TagID,
    pub name: String,
}

#[derive(sqlx::FromRow, Deserialize, Serialize, Debug, Default)]
pub(crate) struct TagResponse {
    pub(crate) id: PostID,
    pub(crate) name: String,
    //date_added: String, // date
}

#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct TagsResponse {
    count: usize,
    pub(crate) results: Vec<TagResponse>,
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

    match sqlx::query_as::<_, TagResponse>(sql)
        .fetch_all(&state.pool)
        .await
    {
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
