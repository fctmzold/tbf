use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

use crate::cli::Cli;
use crate::commands::exact;
use crate::util::parse_timestamp;

/// Extract timestamps from a tracker URL and run an exact lookup.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `url` - TwitchTracker or StreamsCharts URL.
/// * `flags` - Global CLI flags.
///
/// # Errors
///
/// Returns an error when fetching, HTML parsing, or the follow-up lookup fails.
pub async fn execute(client: &Client, url: &str, flags: &Cli) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to fetch URL")?;
    let html = response.text().await.context("Failed to read HTML")?;
    let document = Html::parse_document(&html);

    let mut timestamps = Vec::new();
    let data_selector = Selector::parse("div[data-requests]").expect("static selector is valid");

    for element in document.select(&data_selector) {
        if let Some(data) = element.value().attr("data-requests") {
            if let Ok(clips) = serde_json::from_str::<Vec<serde_json::Value>>(data) {
                if let Some(first) = clips.first() {
                    if let Some(started) = first.get("started_at").and_then(|value| value.as_str())
                    {
                        timestamps.push(started.to_string());
                    }
                }
                if let Some(last) = clips.last() {
                    if let Some(ended) = last.get("ended_at").and_then(|value| value.as_str()) {
                        timestamps.push(ended.to_string());
                    }
                }
            }
        }
    }

    if timestamps.is_empty() {
        let time_selector = Selector::parse("time").expect("static selector is valid");
        for element in document.select(&time_selector) {
            if let Some(datetime) = element.value().attr("datetime") {
                timestamps.push(datetime.to_string());
                break;
            }
        }
    }

    if timestamps.is_empty() {
        println!("Could not extract timestamps from the provided URL.");
        return Ok(());
    }

    println!("Extracted timestamps: {timestamps:?}");

    if url.contains("twitchtracker.com") {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() >= 5 {
            let username = parts[3];
            let id = parts[4].parse::<i64>().unwrap_or(0);
            if id > 0 && !username.is_empty() {
                println!("Detected Username: {username}, ID: {id}");
                if let Ok(start) = parse_timestamp(&timestamps[0]) {
                    let stamp = start.to_string();
                    return exact::execute(client, username, id, &stamp, flags).await;
                }
            }
        }
    }
    Ok(())
}
