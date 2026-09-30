use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::Cli;
use crate::progress::{emit, scanning_progress};
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
/// Hits print as they arrive. Failed probes print as a warning; when every
/// probe fails an error is returned instead of a misleading "not found".
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `target` - Username, VOD ID, and range bounds.
/// * `flags` - Global CLI flags controlling concurrency and progress output.
///
/// # Errors
///
/// Returns an error when timestamps are invalid, the range is reversed, or
/// all probes failed.
pub async fn execute(client: &Client, target: BruteforceTarget<'_>, flags: &Cli) -> Result<()> {
    let username = target.username.to_lowercase();
    let start = parse_timestamp(target.from).context("Invalid 'from' timestamp")?;
    let end = parse_timestamp(target.to).context("Invalid 'to' timestamp")?;

    if start > end {
        anyhow::bail!("Start timestamp must be before end timestamp");
    }

    let total =
        (end - start + 1) as u64 * DEFAULT_CDNS.len() as u64 * check::QUALITIES.len() as u64;
    let progress = scanning_progress(
        total,
        "Searching for VOD playlist...",
        flags.simple,
        flags.progressbar,
    );

    let candidates = (start..=end).flat_map(|timestamp| {
        let stem = check::url_stem(&username, target.id, timestamp);
        DEFAULT_CDNS.iter().flat_map(move |cdn| {
            let stem = stem.clone();
            check::QUALITIES.iter().map(move |quality| {
                let url = check::playlist_url(cdn, &stem, quality);
                (timestamp, url, (*quality).to_string())
            })
        })
    });

    let (found, failed) = stream::iter(candidates)
        .map(|(timestamp, url, quality)| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let probe = check::probe_head(&client, &url).await;
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                (probe, timestamp, url, quality)
            }
        })
        .buffer_unordered(usize::from(flags.threads))
        .fold(
            (0_u64, 0_u64),
            |(found, failed), (probe, timestamp, url, quality)| {
                let progress = progress.clone();
                async move {
                    match probe {
                        check::Probe::Hit => {
                            emit(
                                progress.as_ref(),
                                format!("[{quality}] Timestamp {timestamp}: {url}"),
                            );
                            (found + 1, failed)
                        }
                        check::Probe::Miss => (found, failed),
                        check::Probe::Failed => (found, failed + 1),
                    }
                }
            },
        )
        .await;

    if let Some(bar) = progress {
        bar.finish_with_message("Search complete");
    }

    if failed > 0 {
        println!("Warning: {failed} of {total} probes failed; results may be incomplete.");
    }
    if found == 0 {
        if failed == total {
            anyhow::bail!("All {total} probes failed; check your connection and try again.");
        }
        println!("Could not find any available VODs in the specified range.");
    } else {
        println!("Found {found} potential playlists.");
    }

    Ok(())
}
