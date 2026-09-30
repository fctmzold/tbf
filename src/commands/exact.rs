use anyhow::Result;
use reqwest::Client;

use crate::cli::Cli;
use crate::report::{emit_hit, hint, suggest_player};
use crate::twitch::check;
use crate::util::format_utc;

/// Check offsets around a timestamp for a playable VOD.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `id` - VOD/broadcast ID.
/// * `timestamp` - Base Unix epoch seconds.
/// * `flags` - Global CLI flags controlling output.
///
/// # Errors
///
/// Returns an error when every probe failed, leaving no reliable answer.
pub async fn execute(
    client: &Client,
    username: &str,
    id: i64,
    timestamp: i64,
    flags: &Cli,
) -> Result<()> {
    let username = username.to_lowercase();
    let offsets = std::iter::once(0).chain((1..=10).flat_map(|delta| [delta, -delta]));

    eprintln!(
        "Checking offsets from -10 to +10 around {}...",
        format_utc(timestamp)
    );

    let mut failed_total = 0_u64;
    let mut probe_total = 0_u64;
    for offset in offsets {
        let current = timestamp + offset;
        let outcome =
            check::check_availability(client, &username, id, current, usize::from(flags.threads))
                .await;

        probe_total += check::probe_total() as u64;
        failed_total += outcome.failed;
        if !outcome.hits.is_empty() {
            eprintln!(
                "Found VOD at offset {offset} (Timestamp: {current} = {})",
                format_utc(current)
            );
            for info in &outcome.hits {
                emit_hit(info, flags.simple);
            }
            suggest_player(&outcome.hits[0].playlist_url, flags.simple);
            return Ok(());
        }
        if outcome.failed > 0 {
            eprintln!(
                "Warning: {} of {} probes failed at offset {offset}; results may be incomplete.",
                outcome.failed,
                check::probe_total()
            );
        }
    }

    if probe_total > 0 && failed_total == probe_total {
        anyhow::bail!("All {probe_total} probes failed; check your connection and try again.");
    }
    eprintln!("Could not find available VOD in the specified range.");
    hint("confirm the timestamp with `link`, or widen the search with `bruteforce`");
    Ok(())
}
