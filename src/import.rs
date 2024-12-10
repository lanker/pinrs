use anyhow::Result;
use chrono::DateTime;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::fs::File;
use std::io::BufReader;
use tracing::error;

use crate::api::handlers::bookmarks::{BookmarkQuery, BookmarkRequest};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct LinkDing {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) notes: Option<String>,
    pub(crate) unread: bool,
    pub(crate) tag_names: Option<Vec<String>>,
    pub(crate) date_added: String,
    pub(crate) date_modified: String,
}

impl From<LinkDing> for BookmarkRequest {
    fn from(val: LinkDing) -> Self {
        let added = DateTime::parse_from_rfc3339(val.date_added.as_ref()).ok();
        let modified = DateTime::parse_from_rfc3339(val.date_modified.as_ref()).ok();

        BookmarkRequest {
            url: val.url,
            title: val.title,
            description: val.description,
            notes: val.notes,
            unread: Some(val.unread),
            tag_names: val.tag_names,
            date_added: added.map(|a| a.timestamp()),
            date_modified: modified.map(|a| a.timestamp()),
        }
    }
}

pub(crate) async fn import(path: String, pool: &SqlitePool) -> Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let bookmarks: Vec<LinkDing> = serde_json::from_reader(reader)?;

    let mut success = 0;
    let mut failed = vec![];
    for bookmark in bookmarks {
        match crate::api::handlers::bookmarks::add_bookmark(pool, bookmark.clone().into()).await {
            Ok(_id) => success += 1,
            Err(_err) => {
                failed.push(bookmark.url);
            }
        };
    }

    println!("Imported {} entries", success);

    if !failed.is_empty() {
        error!("Failed to import:\n{}", failed.join("\n"));
    }

    Ok(())
}

pub(crate) async fn export_html(pool: &SqlitePool) -> Result<()> {
    let query = BookmarkQuery {
        limit: Some(0),
        ..Default::default()
    };

    let bookmarks = crate::api::handlers::bookmarks::get_bookmarks(pool, query).await;

    let mut result = vec![
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>".to_owned(),
        "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">".to_owned(),
        "<TITLE>Bookmarks</TITLE>".to_owned(),
        "<H1>Bookmarks</H1>".to_owned(),
        "<DL><p>".to_owned(),
    ];

    for bookmark in bookmarks {
        let added = DateTime::parse_from_rfc3339(bookmark.date_added.as_ref())
            .map(|a| a.timestamp())
            .unwrap_or_default();
        let modified = DateTime::parse_from_rfc3339(bookmark.date_modified.as_ref())
            .map(|a| a.timestamp())
            .unwrap_or_default();
        let entry = format!(
            r#"<DT><A HREF="{}" ADD_DATE="{}" LAST_MODIFIED="{}" TOREAD="{}" TAGS="{}">{}</A>"#,
            &bookmark.url,
            added,
            modified,
            if bookmark.unread { "1" } else { "0" },
            &bookmark.tag_names.join(","),
            &bookmark.title
        );
        result.push(entry);

        let mut text = vec![];
        let description = bookmark.description.unwrap_or_default();
        if !description.is_empty() {
            text.push(description);
        }
        let notes = bookmark.notes.unwrap_or_default();
        if !notes.is_empty() {
            text.push(notes);
        }

        if !text.is_empty() {
            result.push("<DD>".to_owned());
            result.push(text.join("---\n"));
        }
    }
    result.push("</DL><p>".to_owned());

    println!("{}", result.join("\n"));
    Ok(())
}
