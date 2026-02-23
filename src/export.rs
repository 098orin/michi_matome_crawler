use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;
use serde::Serialize;
use std::fs::File;
use std::io::Write;

use crate::db;

#[derive(Serialize)]
struct ExportItem {
    id: String,
    r#type: String,
    title: String,
    url: String,
    description: Option<String>,
    thumbnail: Option<String>,
    published_at: Option<String>,
    score: i32,
}

// Entry point
pub fn export_json(conn: &Connection, path: &str) -> Result<()> {
    let items = db::fetch_all(conn)?;

    let mut exported = Vec::new();

    for item in items {
        let score = calculate_score(&item);

        exported.push(ExportItem {
            id: item.id,
            r#type: item.content_type,
            title: item.title,
            url: item.url,
            description: item.description,
            thumbnail: item.thumbnail,
            published_at: item.published_at,
            score,
        });
    }

    // Sort by score descending
    exported.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    let json = serde_json::to_string_pretty(&exported)?;

    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;

    Ok(())
}

fn calculate_score(item: &db::Content) -> i32 {
    let mut score = 0;
    if Regex::new(r"[一-龠ぁ-んァ-ン]+道\d+号").unwrap().is_match(
        format!(
            "{}, {}",
            &item.title,
            item.description.clone().unwrap_or("".into())
        )
        .as_str(),
    ) {
        score += 5
    }

    if Regex::new(r"[一-龠ぁ-んァ-ン]+跡").unwrap().is_match(
        format!(
            "{}, {}",
            &item.title,
            item.description.clone().unwrap_or("".into())
        )
        .as_str(),
    ) {
        score += 3
    }

    if Regex::new(r"[一-龠ぁ-んァ-ン]+道").unwrap().is_match(
        format!(
            "{}, {}",
            &item.title,
            item.description.clone().unwrap_or("".into())
        )
        .as_str(),
    ) {
        score += 1
    }

    if item.title.contains("404 Not Found") {
        score -= 3
    }

    score
}
