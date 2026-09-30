use anyhow::Result;
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;

use crate::cli::Cli;

/// Scan clip offsets for playable MP4 assets.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `id` - VOD/broadcast ID.
/// * `start` - Start offset in seconds.
/// * `end` - End offset in seconds.
/// * `flags` - Global CLI flags controlling concurrency and progress output.
///
/// # Errors
///
/// Returns an error when the range is reversed.
pub async fn execute(client: &Client, id: i64, start: i64, end: i64, flags: &Cli) -> Result<()> {
    if start > end {
        anyhow::bail!("Start offset must be before end offset");
    }

    let progress = if flags.progressbar {
        let bar = ProgressBar::new((end - start) as u64);
        let style = ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("##-");
        bar.set_style(style);
        bar.set_message("Scanning for clips...");
        Some(bar)
    } else {
        None
    };

    let found = stream::iter(start..end)
        .map(|offset| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let url = format!("https://clips-media-assets2.twitch.tv/{id}-offset-{offset}.mp4");
                let available = client
                    .head(&url)
                    .send()
                    .await
                    .map(|response| response.status().is_success())
                    .unwrap_or(false);
                if let Some(bar) = progress {
                    bar.inc(1);
                }
                available.then_some(url)
            }
        })
        .buffer_unordered(flags.threads)
        .filter_map(|item| async move { item })
        .collect::<Vec<_>>()
        .await;

    if let Some(bar) = progress {
        bar.finish_with_message("Scan complete");
    }

    if found.is_empty() {
        println!("Could not find any clips in the specified range.");
    } else {
        println!("Found {} clips:", found.len());
        for url in found {
            println!("{url}");
        }
    }
    Ok(())
}
