use anyhow::Result;
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::Cli;
use crate::progress::scanning_progress;
use crate::report::{emit_hit, hint, suggest_player};
use crate::twitch::check::{self, ScanOutcome};
use crate::util::format_utc;

/// Request estimate above which confirmation is required.
const CONFIRM_THRESHOLD: u64 = 20_000;

/// Inputs for a bruteforce scan.
pub struct BruteforceTarget<'a> {
    /// Streamer login name.
    pub username: &'a str,
    /// VOD/broadcast ID.
    pub id: i64,
    /// Range start as Unix epoch seconds.
    pub from: i64,
    /// Range end as Unix epoch seconds.
    pub to: i64,
    /// Keep scanning after the first hit.
    pub all: bool,
    /// Skip the large-range confirmation.
    pub yes: bool,
}

/// Scan a timestamp range for playable VOD playlists.
///
/// Probes `chunked` first and expands to all qualities only on hits.
/// Stops at the first hit unless `all` is set.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `target` - Username, VOD ID, range bounds, and scan options.
/// * `flags` - Global CLI flags controlling concurrency and progress output.
///
/// # Errors
///
/// Returns an error when the range is reversed, confirmation is missing
/// for a huge range, or all probes failed.
pub async fn execute(client: &Client, target: BruteforceTarget<'_>, flags: &Cli) -> Result<()> {
    let username = target.username.to_lowercase();
    if target.from > target.to {
        anyhow::bail!("Start timestamp must be before end timestamp");
    }

    let timestamps: Vec<i64> = (target.from..=target.to).collect();
    let estimate = timestamps.len() as u64 * check::DEFAULT_CDN_COUNT as u64;
    if !target.yes && estimate > CONFIRM_THRESHOLD {
        anyhow::bail!("This scan implies about {estimate} requests. Rerun with --yes to proceed.");
    }

    eprintln!(
        "Scanning {} timestamps ({} to {}, chunked first)...",
        timestamps.len(),
        format_utc(target.from),
        format_utc(target.to)
    );
    let progress = scanning_progress(
        timestamps.len() as u64,
        "Scanning timestamps...",
        flags.simple,
        flags.progressbar,
    );
    let threads = usize::from(flags.threads);

    if target.all {
        scan_all(
            client, &username, target.id, timestamps, threads, progress, flags,
        )
        .await
    } else {
        scan_first(
            client, &username, target.id, timestamps, threads, progress, flags,
        )
        .await
    }
}

/// Scan in order, stopping at the first timestamp with a hit.
async fn scan_first(
    client: &Client,
    username: &str,
    id: i64,
    timestamps: Vec<i64>,
    threads: usize,
    progress: Option<indicatif::ProgressBar>,
    flags: &Cli,
) -> Result<()> {
    let mut failed_total = 0_u64;
    for timestamp in timestamps {
        let outcome =
            check::check_qualities(client, username, id, timestamp, &["chunked"], threads).await;
        failed_total += outcome.failed;
        if let Some(bar) = &progress {
            bar.inc(1);
        }
        if outcome.hits.is_empty() {
            continue;
        }
        let full = check::check_availability(client, username, id, timestamp, threads).await;
        failed_total += full.failed;
        if let Some(bar) = &progress {
            bar.finish_with_message("Search complete");
        }
        eprintln!(
            "Found VOD at timestamp {timestamp} ({})",
            format_utc(timestamp)
        );
        for info in &full.hits {
            emit_hit(info, flags.simple);
        }
        if !full.hits.is_empty() {
            suggest_player(&full.hits[0].playlist_url, flags.simple);
        }
        if failed_total > 0 {
            eprintln!("Warning: {failed_total} probes failed; results may be incomplete.");
        }
        return Ok(());
    }

    if let Some(bar) = progress {
        bar.finish_with_message("Search complete");
    }
    if failed_total > 0 {
        eprintln!("Warning: {failed_total} probes failed; results may be incomplete.");
    }
    eprintln!("Could not find any available VODs in the specified range.");
    hint("check the channel name and timestamps, or try `live` for a live stream");
    Ok(())
}

/// Scan concurrently, reporting every hit.
async fn scan_all(
    client: &Client,
    username: &str,
    id: i64,
    timestamps: Vec<i64>,
    threads: usize,
    progress: Option<indicatif::ProgressBar>,
    flags: &Cli,
) -> Result<()> {
    let mut probed: Vec<(i64, ScanOutcome)> = stream::iter(timestamps)
        .map(|timestamp| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let chunked =
                    check::check_qualities(&client, username, id, timestamp, &["chunked"], threads)
                        .await;
                let outcome = if chunked.hits.is_empty() {
                    chunked
                } else {
                    let full =
                        check::check_availability(&client, username, id, timestamp, threads).await;
                    ScanOutcome {
                        failed: chunked.failed + full.failed,
                        hits: full.hits,
                    }
                };
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                (timestamp, outcome)
            }
        })
        .buffer_unordered(threads)
        .collect()
        .await;

    probed.sort_by_key(|(timestamp, _)| *timestamp);
    if let Some(bar) = &progress {
        bar.finish_with_message("Search complete");
    }

    let mut found = 0_usize;
    let mut failed_total = 0_u64;
    let mut first_url = None;
    for (timestamp, outcome) in &probed {
        failed_total += outcome.failed;
        if outcome.hits.is_empty() {
            continue;
        }
        found += 1;
        eprintln!("Timestamp {timestamp} ({}):", format_utc(*timestamp));
        for info in &outcome.hits {
            emit_hit(info, flags.simple);
            if first_url.is_none() {
                first_url = Some(info.playlist_url.clone());
            }
        }
    }

    if failed_total > 0 {
        eprintln!("Warning: {failed_total} probes failed; results may be incomplete.");
    }
    if found == 0 {
        eprintln!("Could not find any available VODs in the specified range.");
        hint("check the channel name and timestamps, or try `live` for a live stream");
    } else {
        eprintln!("Found {found} timestamps with playable playlists.");
        if let Some(url) = first_url {
            suggest_player(&url, flags.simple);
        }
    }
    Ok(())
}
