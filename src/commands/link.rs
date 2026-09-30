use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

use crate::cli::Cli;
use crate::commands::exact;
use crate::util::parse_timestamp;

/// Identify the streamer and VOD ID from a tracker URL.
///
/// Supports `twitchtracker.com/<user>/streams/<id>` and
/// `streamscharts.com/channels/<user>/streams/<id>`.
///
/// # Arguments
///
/// * `url` - Tracker page URL.
///
/// # Returns
///
/// Lowercase username and VOD ID.
///
/// # Errors
///
/// Returns an error for unparsable URLs, unsupported hosts or paths, and
/// non-numeric IDs.
fn parse_tracker_target(page_url: &str) -> Result<(String, i64)> {
    let parsed = url::Url::parse(page_url).context("Invalid URL")?;
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.");
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect())
        .unwrap_or_default();

    let (username, id_text) = match (host, segments.as_slice()) {
        ("twitchtracker.com", [username, "streams", id]) => (*username, *id),
        ("streamscharts.com", ["channels", username, "streams", id]) => (*username, *id),
        _ => anyhow::bail!(
            "Unsupported tracker URL: expected a TwitchTracker or StreamsCharts stream page"
        ),
    };
    let id = id_text
        .parse::<i64>()
        .context("Invalid VOD ID in tracker URL")?;
    Ok((username.to_lowercase(), id))
}

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
/// Returns an error when fetching, parsing, or the follow-up lookup fails.
pub async fn execute(client: &Client, url: &str, flags: &Cli) -> Result<()> {
    let target = parse_tracker_target(url)?;
    let (username, id) = target;
    println!("Detected Username: {username}, ID: {id}");

    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to fetch URL")?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Tracker page fetch failed with status {}",
            response.status()
        );
    }
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
        anyhow::bail!("Could not extract timestamps from the provided URL.");
    }

    println!("Extracted timestamps: {timestamps:?}");
    println!("Running Exact search around the extracted timestamp...");
    let start = parse_timestamp(&timestamps[0]).context("Failed to parse extracted timestamp")?;
    exact::execute(client, &username, id, &start.to_string(), flags).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_twitchtracker_url() {
        let (username, id) =
            parse_tracker_target("https://twitchtracker.com/Arquel/streams/316969565142")
                .expect("valid tracker URL");
        assert_eq!(username, "arquel");
        assert_eq!(id, 316_969_565_142);
    }

    #[test]
    fn parses_streamscharts_url() {
        let (username, id) =
            parse_tracker_target("https://streamscharts.com/channels/xqc/streams/12345")
                .expect("valid tracker URL");
        assert_eq!(username, "xqc");
        assert_eq!(id, 12_345);
    }

    #[test]
    fn rejects_host_in_query_string() {
        assert!(parse_tracker_target("https://example.com/?next=twitchtracker.com").is_err());
    }

    #[test]
    fn rejects_non_numeric_id() {
        assert!(parse_tracker_target("https://twitchtracker.com/user/streams/latest").is_err());
    }

    #[test]
    fn rejects_unknown_host() {
        assert!(parse_tracker_target("https://example.com/user/123").is_err());
    }
}
