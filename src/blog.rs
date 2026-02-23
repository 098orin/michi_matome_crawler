use anyhow::Result;
use chrono::Utc;
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Client;
use reqwest::StatusCode;
use rusqlite::Connection;
use scraper::{Html, Selector};
use thiserror::Error;
use url::Url;

use crate::db;

const MAX_NEW_PER_SITE: usize = 5;

#[derive(Debug, Error)]
pub enum CrawlError {
    #[error("HTTP status error: {status} {url}")]
    HttpStatus { status: StatusCode, url: String },
}

pub async fn fetch_and_store(conn: &Connection, base_url: &str) -> Result<()> {
    println!("Crawl blog; base_url: {}", base_url);

    let client = Client::new();

    // Try sitemap first
    if let Ok(urls) = fetch_sitemap(&client, base_url).await {
        println!("Crawl sitemap");
        let mut counter = 0;
        let now = Utc::now().to_rfc3339();

        for url in urls {
            let inserted = crawl_article(conn, &client, &url, &now, false)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Blog warn: {}", e);
                    false
                });

            if inserted {
                counter += 1;
            }

            if counter >= MAX_NEW_PER_SITE {
                println!("Reached limit, stopping this site.");
                break;
            }
        }

        return Ok(());
    }

    // Fallback to HTML link scraping
    println!("Crawl via HTML link scraping");
    crawl_html(conn, base_url, MAX_NEW_PER_SITE).await
}

async fn fetch_sitemap(client: &Client, base_url: &str) -> Result<Vec<String>> {
    let sitemap_url = format!("{}/sitemap.xml", base_url.trim_end_matches('/'));

    let body = client.get(&sitemap_url).send().await?.text().await?;

    let mut reader = Reader::from_str(&body);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut urls = Vec::new();
    let mut inside_loc = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.name().as_ref() == b"loc" => {
                inside_loc = true;
            }
            Ok(Event::Text(e)) if inside_loc => {
                let text = String::from_utf8_lossy(e.as_ref()).to_string();
                urls.push(text);
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"loc" => {
                inside_loc = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    if urls.is_empty() {
        anyhow::bail!("No URLs in sitemap");
    }

    Ok(urls)
}

pub async fn crawl_html(conn: &Connection, base_url: &str, max_new: usize) -> Result<()> {
    let client = Client::new();

    let now = Utc::now().to_rfc3339();

    // Insert root if not exists
    db::enqueue(conn, base_url, None)?;

    let mut new_count = 0;

    loop {
        // Stop if limit reached
        if new_count >= max_new {
            break;
        }

        let targets = db::next_pending(conn, 10)?;
        if targets.is_empty() {
            break;
        }

        for url in targets {
            if new_count >= max_new {
                break;
            }

            match crawl_page(conn, &client, &url).await {
                Ok(added) => {
                    let inserted = crawl_article(conn, &client, &url, &now, false)
                        .await
                        .unwrap_or_else(|e| {
                            eprintln!("Blog warn: {}", e);
                            false
                        });

                    if inserted {
                        new_count += added
                    }

                    db::mark_done(conn, &url)?;
                }
                Err(e) => {
                    eprintln!("Crawl html warn: {}, {}", e, url);
                }
            }
        }
    }

    Ok(())
}

async fn crawl_page(conn: &Connection, client: &Client, url: &str) -> Result<usize> {
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Status error {}", response.status());
    }

    // HTML only
    if let Some(ct) = response.headers().get(reqwest::header::CONTENT_TYPE) {
        if !ct.to_str()?.contains("text/html") {
            return Ok(0);
        }
    } else if !is_article_link(&url) {
        return Ok(0);
    }

    let body = response.text().await?;
    let document = Html::parse_document(&body);
    let selector = Selector::parse("a").unwrap();

    let mut added = 0;

    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            let next_url = normalize_url(url, href);

            if !same_domain(url, &next_url) {
                continue;
            }

            if db::enqueue(conn, &next_url, Some(url))? {
                added += 1;
            }
        }
    }

    Ok(added)
}

async fn crawl_article(
    conn: &Connection,
    client: &Client,
    url: &str,
    fetched_at: &str,
    ignore_skip: bool,
) -> Result<bool> {
    if db::should_skip(conn, url)? && !ignore_skip {
        println!("Skipping {} due to recent error", url);
        return Ok(false);
    }

    let fetch_result = fetch_html(client, url).await;

    if let Err(ref e) = fetch_result {
        if let Some(crawl_err) = e.downcast_ref::<CrawlError>() {
            match crawl_err {
                CrawlError::HttpStatus { status, url } => {
                    println!("Status error: {} {}", status, url);

                    if *status == StatusCode::NOT_FOUND {
                        db::register_error(conn, url, "404", 7)?;
                    }
                }
            }
        }
    }

    let body = fetch_result?;

    let document = Html::parse_document(&body);

    let title_selector = Selector::parse("title").unwrap();
    let meta_selector = Selector::parse("meta[name=description]").unwrap();

    let title = document
        .select(&title_selector)
        .next()
        .map(|t| t.text().collect::<String>())
        .unwrap_or_else(|| "No Title".to_string());

    let description = document
        .select(&meta_selector)
        .next()
        .and_then(|m| m.value().attr("content"))
        .map(|s| s.to_string());

    let result = db::insert(
        conn,
        url, // URL as unique ID
        "blog",
        &title,
        url,
        description.as_deref(),
        None,
        None,
        fetched_at,
    );

    if let Ok(bool) = result {
        if bool {
            println!("Crawl and insert article: {}", url);
        }
    }

    result
}

fn is_article_link(href: &str) -> bool {
    // Simple heuristic:
    // contains year/month or ends with html
    href.contains("/20") || href.ends_with(".html")
}

// Check if two URLs share the same domain
fn same_domain(base: &str, target: &str) -> bool {
    let base_url = Url::parse(base).ok();
    let target_url = Url::parse(target).ok();

    match (base_url, target_url) {
        (Some(b), Some(t)) => b.domain() == t.domain(),
        _ => false,
    }
}

fn normalize_url(base: &str, href: &str) -> String {
    // Parse base URL
    let base_url = match Url::parse(base) {
        Ok(u) => u,
        Err(_) => return href.to_string(),
    };

    // Resolve relative URL correctly
    match base_url.join(href) {
        Ok(joined) => joined.to_string(),
        Err(_) => href.to_string(),
    }
}

use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use regex::Regex;
use reqwest::header::CONTENT_TYPE;

async fn fetch_html(client: &Client, url: &str) -> Result<String> {
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(CrawlError::HttpStatus {
            status: response.status(),
            url: url.to_string(),
        }
        .into());
    }

    let headers = response.headers().clone();
    let bytes = response.bytes().await?;

    // 1. Try charset from header
    if let Some(content_type) = headers.get(CONTENT_TYPE) {
        if let Ok(content_type_str) = content_type.to_str() {
            if let Some(charset) = content_type_str.split("charset=").nth(1) {
                if let Some(encoding) = Encoding::for_label(charset.trim().as_bytes()) {
                    let (text, _, _) = encoding.decode(&bytes);
                    return Ok(text.into_owned());
                }
            }
        }
    }

    // 2. Try to detect charset from meta tag (ASCII-safe)
    let ascii_head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);

    let re = Regex::new(r#"charset\s*=\s*["']?([A-Za-z0-9_\-]+)"#)?;

    if let Some(cap) = re.captures(&ascii_head) {
        let charset = cap.get(1).unwrap().as_str();

        if let Some(encoding) = Encoding::for_label(charset.as_bytes()) {
            let (text, _, _) = encoding.decode(&bytes);
            return Ok(text.into_owned());
        }
    }

    // 3. Fallback: Detect encoding automatically
    let mut detector = EncodingDetector::new();
    detector.feed(&bytes, true);

    let encoding = detector.guess(None, true);

    let (text, _, _) = encoding.decode(&bytes);

    Ok(text.into_owned())
}
