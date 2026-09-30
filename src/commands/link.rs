use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

use crate::cli::Cli;
use crate::commands::exact;
use crate::twitch::retry::{check_http_status, classify_request, with_retry};
use crate::util::parse_timestamp;

/// Identify the streamer and VOD ID from a StreamsCharts stream URL.
///
/// TwitchTracker has no per-stream pages, so only StreamsCharts
/// `channels/<user>/streams/<id>` URLs are supported. The ID is the Twitch
/// broadcast ID, verified against live pages.
///
/// # Arguments
///
/// * `url` - Stream page URL.
///
/// # Returns
///
/// Lowercase username and broadcast ID.
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
        ("streamscharts.com", ["channels", username, "streams", id]) => (*username, *id),
        _ => anyhow::bail!("Unsupported tracker URL: expected a StreamsCharts stream page"),
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
/// * `url` - StreamsCharts stream page URL.
/// * `flags` - Global CLI flags.
///
/// # Errors
///
/// Returns an error when fetching, parsing, or the follow-up lookup fails.
pub async fn execute(client: &Client, url: &str, flags: &Cli) -> Result<()> {
    let target = parse_tracker_target(url)?;
    let (username, id) = target;
    println!("Detected Username: {username}, ID: {id}");

    let response = with_retry(
        || async {
            client
                .get(url)
                .send()
                .await
                .map_err(classify_request)
                .and_then(|response| {
                    check_http_status(response.status())?;
                    Ok(response)
                })
        },
        3,
    )
    .await
    .context("Failed to fetch tracker page")?;
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
    fn parses_streamscharts_url() {
        let (username, id) =
            parse_tracker_target("https://streamscharts.com/channels/xqc/streams/12345")
                .expect("valid tracker URL");
        assert_eq!(username, "xqc");
        assert_eq!(id, 12_345);
    }

    #[test]
    fn rejects_twitchtracker_shape() {
        assert!(
            parse_tracker_target("https://twitchtracker.com/arquel/streams/316969565142").is_err()
        );
    }

    #[test]
    fn rejects_host_in_query_string() {
        assert!(parse_tracker_target("https://example.com/?next=streamscharts.com").is_err());
    }

    #[test]
    fn rejects_non_numeric_id() {
        assert!(
            parse_tracker_target("https://streamscharts.com/channels/user/streams/latest").is_err()
        );
    }

    #[test]
    fn rejects_unknown_host() {
        assert!(parse_tracker_target("https://example.com/user/123").is_err());
    }
}
