mod blog;
mod config;
mod db;
mod export;

use anyhow::Result;
use rusqlite::Connection;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: crawler <config.json>");
        std::process::exit(1);
    }

    let config_path = &args[1];

    let config = config::load(config_path)?;

    println!("Crawler started");

    // Open SQLite database
    let conn = Connection::open("crawler.db")?;

    // Initialize tables
    db::init(&conn)?;

    // === Blogs ===
    for blog_cfg in config.blogs {
        if let Err(e) = blog::fetch_and_store(&conn, &blog_cfg.url).await {
            eprintln!("Blog error: {e}");
        }
    }

    // === Export JSON ===
    export::export_json(&conn, "index.json")?;

    println!("Crawler finished");

    Ok(())
}
