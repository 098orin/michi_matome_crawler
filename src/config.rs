use anyhow::Result;
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub youtube: Vec<YouTubeConfig>,
    pub blogs: Vec<BlogConfig>,
}

#[derive(Debug, Deserialize)]
pub struct YouTubeConfig {
    pub channel_id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct BlogConfig {
    pub name: String,
    pub url: String,
}

pub fn load(path: &str) -> Result<Config> {
    let text = fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&text)?;
    Ok(config)
}
