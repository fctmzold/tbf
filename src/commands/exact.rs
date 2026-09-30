use anyhow::{Context, Result};
use reqwest::Client;

use crate::cli::Cli;
use crate::twitch::check;
use crate::util::parse_timestamp;

/// Check offsets around a timestamp for a playable VOD.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `id` - VOD/broadcast ID.
/// * `stamp` - Base timestamp string.
/// * `flags` - Global CLI flags controlling output.
///
/// # Errors
///
/// Returns an error when the timestamp cannot be parsed.
pub async fn execute(
    client: &Client,
    username: &str,
    id: i64,
    stamp: &str,
    flags: &Cli,
) -> Result<()> {
    let username = username.to_lowercase();
    let base_timestamp = parse_timestamp(stamp).context("Failed to parse start timestamp")?;

    let offsets = std::iter::once(0).chain((1..=10).flat_map(|delta| [delta, -delta]));

    if !flags.simple {
        println!("Checking offsets from -10 to +10...");
    }

    let mut failed_total = 0_u64;
    let mut probe_total = 0_u64;
    for offset in offsets {
        let current = base_timestamp + offset;
        let outcome =
            check::check_availability(client, &username, id, current, usize::from(flags.threads))
                .await;

        probe_total += check::probe_total() as u64;
        failed_total += outcome.failed;
        if !outcome.hits.is_empty() {
            println!("Found VOD at offset {offset} (Timestamp: {current})");
            for info in outcome.hits {
                println!("[{}] {}", info.quality, info.playlist_url);
            }
            return Ok(());
        }
        if outcome.failed > 0 {
            println!(
                "Warning: {} of {} probes failed at offset {offset}; results may be incomplete.",
                outcome.failed,
                check::probe_total()
            );
        }
    }

    if probe_total > 0 && failed_total == probe_total {
        anyhow::bail!("All {probe_total} probes failed; check your connection and try again.");
    }
    println!("Could not find available VOD in the specified range.");
    Ok(())
}
