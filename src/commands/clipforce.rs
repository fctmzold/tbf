use anyhow::Result;
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::Cli;
use crate::progress::scanning_progress;
use crate::twitch::check;

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
/// Returns an error when the range is reversed or all probes failed.
pub async fn execute(client: &Client, id: i64, start: i64, end: i64, flags: &Cli) -> Result<()> {
    if start > end {
        anyhow::bail!("Start offset must be before end offset");
    }

    let total = (end - start) as u64;
    let progress = scanning_progress(
        total,
        "Scanning for clips...",
        flags.simple,
        flags.progressbar,
    );

    let (found, failed) = stream::iter(start..end)
        .map(|offset| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let url = format!("https://clips-media-assets2.twitch.tv/{id}-offset-{offset}.mp4");
                let probe = check::probe_head(&client, &url).await;
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                (probe, url)
            }
        })
        .buffer_unordered(usize::from(flags.threads))
        .fold(
            (Vec::new(), 0_u64),
            |(mut found, failed), (probe, url)| async move {
                match probe {
                    check::Probe::Hit => {
                        found.push(url);
                        (found, failed)
                    }
                    check::Probe::Miss => (found, failed),
                    check::Probe::Failed => (found, failed + 1),
                }
            },
        )
        .await;

    if let Some(bar) = progress {
        bar.finish_with_message("Scan complete");
    }

    if failed > 0 {
        println!("Warning: {failed} of {total} probes failed; results may be incomplete.");
    }
    if found.is_empty() {
        if total > 0 && failed == total {
            anyhow::bail!("All {total} probes failed; check your connection and try again.");
        }
        println!("Could not find any clips in the specified range.");
    } else {
        println!("Found {} clips:", found.len());
        for url in found {
            println!("{url}");
        }
    }
    Ok(())
}
