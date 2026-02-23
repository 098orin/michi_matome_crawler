use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

// Struct used for export
#[derive(Debug)]
pub struct Content {
    pub id: String,
    pub content_type: String,
    pub title: String,
    pub url: String,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub published_at: Option<String>,
}

// Initialize database and table
pub fn init(conn: &Connection) -> Result<()> {
    // Enable WAL mode (safer for SD cards)
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS contents (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL,            -- youtube or blog
            title TEXT NOT NULL,
            url TEXT NOT NULL,
            description TEXT,
            thumbnail TEXT,
            published_at TEXT,
            fetched_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_published_at
        ON contents (published_at);
        ",
    )?;

    init_error_table(conn)?;

    Ok(())
}

pub fn init_error_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS error_sites (
            site TEXT PRIMARY KEY,
            last_error_at TEXT NOT NULL,
            retry_after TEXT NOT NULL,
            error_message TEXT
        );
        ",
    )?;
    Ok(())
}

// Returns true if inserted, false if already existed
pub fn insert(
    conn: &Connection,
    id: &str,
    content_type: &str,
    title: &str,
    url: &str,
    description: Option<&str>,
    thumbnail: Option<&str>,
    published_at: Option<&str>,
    fetched_at: &str,
) -> Result<bool> {
    let affected = conn.execute(
        "
        INSERT OR IGNORE INTO contents
        (id, type, title, url, description, thumbnail, published_at, fetched_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ",
        params![
            id,
            content_type,
            title,
            url,
            description,
            thumbnail,
            published_at,
            fetched_at
        ],
    )?;

    Ok(affected > 0)
}

pub fn register_error(conn: &Connection, site: &str, message: &str, retry_days: i64) -> Result<()> {
    let now = Utc::now();
    let retry_after = now + Duration::days(retry_days);

    conn.execute(
        "
        INSERT INTO error_sites (site, last_error_at, retry_after, error_message)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(site) DO UPDATE SET
            last_error_at = excluded.last_error_at,
            retry_after = excluded.retry_after,
            error_message = excluded.error_message
        ",
        rusqlite::params![site, now.to_rfc3339(), retry_after.to_rfc3339(), message],
    )?;

    Ok(())
}

// Fetch all contents for JSON export
pub fn fetch_all(conn: &Connection) -> Result<Vec<Content>> {
    let mut stmt = conn.prepare(
        "
        SELECT id, type, title, url, description, thumbnail, published_at
        FROM contents
        ORDER BY published_at DESC
        ",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(Content {
            id: row.get(0)?,
            content_type: row.get(1)?,
            title: row.get(2)?,
            url: row.get(3)?,
            description: row.get(4)?,
            thumbnail: row.get(5)?,
            published_at: row.get(6)?,
        })
    })?;

    let mut results = Vec::new();
    for item in rows {
        results.push(item?);
    }

    Ok(results)
}

pub fn should_skip(conn: &Connection, site: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT retry_after FROM error_sites WHERE site = ?1")?;

    let mut rows = stmt.query([site])?;

    if let Some(row) = rows.next()? {
        let retry_after: String = row.get(0)?;
        let retry_time = DateTime::parse_from_rfc3339(&retry_after)?.with_timezone(&Utc);

        if Utc::now() < retry_time {
            return Ok(true);
        }
    }

    Ok(false)
}
