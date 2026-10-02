use anyhow::Result;
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::GlobalOpts;
use crate::commands::Outcome;
use crate::progress::scanning_progress;
use crate::report::{emit_hit, hint, note, suggest_player};
use crate::twitch::check::{self, Prober, ScanOutcome};
use crate::util::{format_utc, range_len};

/// Request estimate above which confirmation is required.
pub(crate) const CONFIRM_THRESHOLD: u64 = 20_000;

/// Inputs for a bruteforce scan.
pub struct BruteforceTarget<'a> {
    /// Streamer login name.
    pub username: &'a str,
    /// VOD/broadcast ID.
    pub id: u64,
    /// Range start as Unix epoch seconds.
    pub from: i64,
    /// Range end as Unix epoch seconds.
    pub to: i64,
    /// Keep scanning after the first hit.
    pub all: bool,
    /// Skip the large-range confirmation.
    pub yes: bool,
}

/// Shared inputs for the first-hit and full-range scans.
struct ScanCtx<'a> {
    /// Prober bounding concurrent requests.
    prober: &'a Prober,
    /// Streamer login name.
    username: &'a str,
    /// VOD/broadcast ID.
    id: u64,
    /// Range start as Unix epoch seconds.
    from: i64,
    /// Range end as Unix epoch seconds.
    to: i64,
    /// Progress bar, if output is interactive.
    progress: Option<indicatif::ProgressBar>,
    /// Global CLI options controlling output and concurrency.
    opts: &'a GlobalOpts,
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
/// * `opts` - Global CLI options controlling concurrency and output.
///
/// # Errors
///
/// Returns an error when the range is reversed, confirmation is missing
/// for a huge range, or all probes failed.
pub async fn execute(
    client: &Client,
    target: BruteforceTarget<'_>,
    opts: &GlobalOpts,
) -> Result<Outcome> {
    let username = target.username.to_lowercase();
    if target.from > target.to {
        anyhow::bail!("Start timestamp must be before end timestamp");
    }

    // Count before iterating: collecting the range first would allocate
    // gigabytes for a typo'd bound before the guard below can refuse it.
    let prober = Prober::with_hosts(client.clone(), usize::from(opts.threads), opts.cdn_hosts());
    let count = range_len(target.from, target.to);
    let estimate = count.saturating_mul(prober.host_count() as u64);
    if !target.yes && estimate > CONFIRM_THRESHOLD {
        anyhow::bail!("This scan implies about {estimate} requests. Rerun with --yes to proceed.");
    }

    eprintln!(
        "Scanning {count} timestamps ({} to {}, chunked first)...",
        format_utc(target.from),
        format_utc(target.to)
    );
    let progress = scanning_progress(
        count,
        "Scanning timestamps...",
        opts.simple,
        opts.progressbar,
    );
    let ctx = ScanCtx {
        prober: &prober,
        username: &username,
        id: target.id,
        from: target.from,
        to: target.to,
        progress,
        opts,
    };

    if target.all {
        scan_all(ctx).await
    } else {
        scan_first(ctx).await
    }
}

/// Probe one timestamp, expanding to all qualities on a `chunked` hit.
///
/// The expansion skips `chunked` (already probed) and prepends those hits,
/// so no playlist is fetched twice.
///
/// # Arguments
///
/// * `ctx` - Shared scan inputs.
/// * `timestamp` - Unix epoch seconds to probe.
///
/// # Returns
///
/// Timestamp with its scan outcome and the number of probes it took.
async fn probe_timestamp(ctx: &ScanCtx<'_>, timestamp: i64) -> (i64, ScanOutcome, u64) {
    let hosts = ctx.prober.host_count() as u64;
    let chunked =
        check::check_qualities(ctx.prober, ctx.username, ctx.id, timestamp, &["chunked"]).await;
    if chunked.hits.is_empty() {
        if let Some(bar) = &ctx.progress {
            bar.inc(1);
        }
        return (timestamp, chunked, hosts);
    }
    let rest = check::check_qualities(
        ctx.prober,
        ctx.username,
        ctx.id,
        timestamp,
        &check::QUALITIES[1..],
    )
    .await;
    let probes = hosts + hosts * check::QUALITIES.len().saturating_sub(1) as u64;
    if let Some(bar) = &ctx.progress {
        bar.inc(1);
    }
    let mut hits = chunked.hits;
    let failed = chunked.failed + rest.failed;
    hits.extend(rest.hits);
    (timestamp, ScanOutcome { hits, failed }, probes)
}

/// Scan in order, stopping at the first timestamp with a hit.
///
/// Timestamps stream through an ordered `buffered` window, so `--threads`
/// bounds real concurrency (via the shared prober) and dropping the stream
/// on the first hit cancels in-flight work.
async fn scan_first(ctx: ScanCtx<'_>) -> Result<Outcome> {
    let window = usize::from(ctx.opts.threads).max(1);
    let mut results = stream::iter(ctx.from..=ctx.to)
        .map(|timestamp| probe_timestamp(&ctx, timestamp))
        .buffered(window);

    let mut failed_total = 0_u64;
    let mut probe_total = 0_u64;
    while let Some((timestamp, outcome, probes)) = results.next().await {
        probe_total += probes;
        failed_total += outcome.failed;
        if outcome.hits.is_empty() {
            continue;
        }
        if let Some(bar) = &ctx.progress {
            bar.finish_with_message("Search complete");
        }
        note(
            ctx.progress.as_ref(),
            format!(
                "Found VOD at timestamp {timestamp} ({})",
                format_utc(timestamp)
            ),
        );
        for info in &outcome.hits {
            emit_hit(
                ctx.progress.as_ref(),
                info,
                ctx.opts.simple,
                ctx.opts.json,
                Some(timestamp),
            );
        }
        suggest_player(&outcome.hits[0].playlist_url, ctx.opts.simple);
        if failed_total > 0 {
            note(
                ctx.progress.as_ref(),
                format!("Warning: {failed_total} probes failed; results may be incomplete."),
            );
        }
        return Ok(Outcome::Found);
    }
    drop(results);

    if let Some(bar) = ctx.progress {
        bar.finish_with_message("Search complete");
    }
    if probe_total > 0 && failed_total == probe_total {
        anyhow::bail!("All {probe_total} probes failed; check your connection and try again.");
    }
    if failed_total > 0 {
        eprintln!("Warning: {failed_total} probes failed; results may be incomplete.");
    }
    eprintln!("Could not find any available VODs in the specified range.");
    hint("check the channel name and timestamps, or try `live` for a live stream");
    Ok(Outcome::NotFound)
}

/// Scan concurrently, reporting every hit.
///
/// Results stream through an ordered `buffered` window, so hits print as
/// they arrive in timestamp order instead of buffering every outcome. The
/// outer window only bounds task count; the prober's semaphore bounds
/// actual requests, so `--threads` is honored however scans nest.
async fn scan_all(ctx: ScanCtx<'_>) -> Result<Outcome> {
    let window = usize::from(ctx.opts.threads).max(1);
    let mut results = stream::iter(ctx.from..=ctx.to)
        .map(|timestamp| probe_timestamp(&ctx, timestamp))
        .buffered(window);

    let mut found = 0_usize;
    let mut failed_total = 0_u64;
    let mut probe_total = 0_u64;
    let mut first_url = None;
    while let Some((timestamp, outcome, probes)) = results.next().await {
        probe_total += probes;
        failed_total += outcome.failed;
        if outcome.hits.is_empty() {
            continue;
        }
        found += 1;
        note(
            ctx.progress.as_ref(),
            format!("Timestamp {timestamp} ({}):", format_utc(timestamp)),
        );
        for info in &outcome.hits {
            emit_hit(
                ctx.progress.as_ref(),
                info,
                ctx.opts.simple,
                ctx.opts.json,
                Some(timestamp),
            );
            if first_url.is_none() {
                first_url = Some(info.playlist_url.clone());
            }
        }
    }

    if let Some(bar) = &ctx.progress {
        bar.finish_with_message("Search complete");
    }

    if probe_total > 0 && failed_total == probe_total {
        anyhow::bail!("All {probe_total} probes failed; check your connection and try again.");
    }
    if failed_total > 0 {
        eprintln!("Warning: {failed_total} probes failed; results may be incomplete.");
    }
    if found == 0 {
        eprintln!("Could not find any available VODs in the specified range.");
        hint("check the channel name and timestamps, or try `live` for a live stream");
        Ok(Outcome::NotFound)
    } else {
        eprintln!("Found {found} timestamps with playable playlists.");
        if let Some(url) = first_url {
            suggest_player(&url, ctx.opts.simple);
        }
        Ok(Outcome::Found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::serve_gated;

    fn scan_ctx<'a>(
        prober: &'a Prober,
        username: &'a str,
        opts: &'a GlobalOpts,
        from: i64,
        to: i64,
    ) -> ScanCtx<'a> {
        ScanCtx {
            prober,
            username,
            id: 1,
            from,
            to,
            progress: None,
            opts,
        }
    }

    #[tokio::test]
    async fn scan_first_stops_at_hit() {
        let target = 1_790_752_835_i64;
        let base = serve_gated(target);
        let prober = Prober::with_hosts(Client::new(), 8, [base]);
        let username = "arquel".to_string();
        let opts = GlobalOpts::default();
        let ctx = scan_ctx(&prober, &username, &opts, target - 2, target + 2);
        assert_eq!(scan_first(ctx).await.expect("scan runs"), Outcome::Found);
    }

    #[tokio::test]
    async fn scan_first_misses_cleanly() {
        let base = serve_gated(9_999_999_999);
        let prober = Prober::with_hosts(Client::new(), 8, [base]);
        let username = "arquel".to_string();
        let opts = GlobalOpts::default();
        let ctx = scan_ctx(&prober, &username, &opts, 1_790_752_800, 1_790_752_810);
        assert_eq!(scan_first(ctx).await.expect("scan runs"), Outcome::NotFound);
    }

    #[tokio::test]
    async fn scan_all_reports_hits() {
        let target = 1_790_752_835_i64;
        let base = serve_gated(target);
        let prober = Prober::with_hosts(Client::new(), 8, [base]);
        let username = "arquel".to_string();
        let opts = GlobalOpts::default();
        let ctx = scan_ctx(&prober, &username, &opts, target - 2, target + 2);
        assert_eq!(scan_all(ctx).await.expect("scan runs"), Outcome::Found);
    }

    #[tokio::test]
    async fn scan_all_misses_cleanly() {
        let base = serve_gated(9_999_999_999);
        let prober = Prober::with_hosts(Client::new(), 8, [base]);
        let username = "arquel".to_string();
        let opts = GlobalOpts::default();
        let ctx = scan_ctx(&prober, &username, &opts, 1_790_752_800, 1_790_752_810);
        assert_eq!(scan_all(ctx).await.expect("scan runs"), Outcome::NotFound);
    }
}
