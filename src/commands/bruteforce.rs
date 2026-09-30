use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;

use crate::cli::Cli;
use crate::twitch::cdns::DEFAULT_CDNS;
use crate::twitch::check;
use crate::util::parse_timestamp;

/// Inputs for a bruteforce scan.
pub struct BruteforceTarget<'a> {
    /// Streamer login name.
    pub username: &'a str,
    /// VOD/broadcast ID.
    pub id: i64,
    /// Range start timestamp string.
    pub from: &'a str,
    /// Range end timestamp string.
    pub to: &'a str,
}

/// Scan a timestamp range for playable VOD playlists.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `target` - Username, VOD ID, and range bounds.
/// * `flags` - Global CLI flags controlling concurrency and progress output.
///
/// # Errors
///
/// Returns an error when timestamps are invalid or the range is reversed.
pub async fn execute(client: &Client, target: BruteforceTarget<'_>, flags: &Cli) -> Result<()> {
    let start = parse_timestamp(target.from).context("Invalid 'from' timestamp")?;
    let end = parse_timestamp(target.to).context("Invalid 'to' timestamp")?;

    if start > end {
        anyhow::bail!("Start timestamp must be before end timestamp");
    }

    let progress = if flags.progressbar {
        let total =
            (end - start + 1) as u64 * DEFAULT_CDNS.len() as u64 * check::QUALITIES.len() as u64;
        let bar = ProgressBar::new(total);
        let style = ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("##-");
        bar.set_style(style);
        bar.set_message("Searching for VOD playlist...");
        Some(bar)
    } else {
        None
    };

    let candidates = (start..=end).flat_map(|timestamp| {
        DEFAULT_CDNS.iter().flat_map(move |cdn| {
            check::QUALITIES.iter().map(move |quality| {
                let url = check::playlist_url(cdn, target.username, target.id, timestamp, quality);
                (timestamp, url, (*quality).to_string())
            })
        })
    });

    let found = stream::iter(candidates)
        .map(|(timestamp, url, quality)| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let available = client
                    .head(&url)
                    .send()
                    .await
                    .map(|response| response.status().is_success())
                    .unwrap_or(false);
                if let Some(bar) = progress {
                    if available {
                        bar.set_message(format!("Found {quality} at timestamp {timestamp}"));
                    } else {
                        bar.inc(1);
                    }
                }
                available.then_some((timestamp, url, quality))
            }
        })
        .buffer_unordered(flags.threads)
        .filter_map(|item| async move { item })
        .collect::<Vec<_>>()
        .await;

    if let Some(bar) = progress {
        bar.finish_with_message("Search complete");
    }

    if found.is_empty() {
        println!("Could not find any available VODs in the specified range.");
    } else {
        println!("Found {} potential playlists:", found.len());
        for (timestamp, url, quality) in found {
            println!("[{quality}] Timestamp {timestamp}: {url}");
        }
    }

    Ok(())
}
