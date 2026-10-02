use anyhow::Result;
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::GlobalOpts;
use crate::commands::Outcome;
use crate::commands::bruteforce::CONFIRM_THRESHOLD;
use crate::progress::scanning_progress;
use crate::twitch::check::{Probe, Prober};
use crate::util::range_len;

/// Inputs for a clipforce scan.
pub struct ClipforceTarget {
    /// VOD/broadcast ID.
    pub id: u64,
    /// Start offset in seconds.
    pub start: i64,
    /// End offset in seconds.
    pub end: i64,
    /// Skip the large-range confirmation.
    pub yes: bool,
}

/// Scan clip offsets for playable MP4 assets.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `target` - VOD ID, offset bounds, and scan options.
/// * `opts` - Global CLI options controlling concurrency and output.
///
/// # Errors
///
/// Returns an error when the range is reversed, confirmation is missing
/// for a huge range, or all probes failed.
pub async fn execute(
    client: &Client,
    target: ClipforceTarget,
    opts: &GlobalOpts,
) -> Result<Outcome> {
    let ClipforceTarget {
        id,
        start,
        end,
        yes,
    } = target;
    if start > end {
        anyhow::bail!("Start offset must be before end offset");
    }

    // Count before iterating: `end - start + 1` overflows `i64` for wide
    // ranges, and one request runs per offset, so the count is the estimate.
    let total = range_len(start, end);
    if !yes && total > CONFIRM_THRESHOLD {
        anyhow::bail!("This scan implies about {total} requests. Rerun with --yes to proceed.");
    }

    let progress = scanning_progress(
        total,
        "Scanning for clips...",
        opts.simple,
        opts.progressbar,
    );
    // Clip hosts stay default: these URLs target the clips host, so custom
    // CDN lists do not apply here.
    let prober = Prober::new(client.clone(), usize::from(opts.threads));
    let window = usize::from(opts.threads).max(1);

    let (mut found, failed) = stream::iter(start..=end)
        .map(|offset| {
            let prober = prober.clone();
            let progress = progress.clone();
            async move {
                let url = format!("https://clips-media-assets2.twitch.tv/{id}-offset-{offset}.mp4");
                let probe = prober.probe_head(&url).await;
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                (probe, offset, url)
            }
        })
        .buffer_unordered(window)
        .fold(
            (Vec::new(), 0_u64),
            |(mut found, failed), (probe, offset, url)| async move {
                match probe {
                    Probe::Hit => {
                        found.push((offset, url));
                        (found, failed)
                    }
                    Probe::Miss => (found, failed),
                    Probe::Failed => (found, failed + 1),
                }
            },
        )
        .await;

    if let Some(bar) = &progress {
        bar.finish_with_message("Scan complete");
    }

    if failed > 0 {
        crate::report::note(
            progress.as_ref(),
            format!("Warning: {failed} of {total} probes failed; results may be incomplete."),
        );
    }
    if found.is_empty() {
        if failed == total {
            anyhow::bail!("All {total} probes failed; check your connection and try again.");
        }
        eprintln!("Could not find any clips in the specified range.");
        crate::report::hint("widen the offset range");
        Ok(Outcome::NotFound)
    } else {
        // `buffer_unordered` finishes in random order; string sorting would
        // put offset 10 before offset 9, so sort by the number.
        found.sort_by_key(|(offset, _)| *offset);
        crate::report::note(progress.as_ref(), format!("Found {} clips:", found.len()));
        for (offset, url) in found {
            if opts.json {
                let entry = serde_json::json!({"clip_url": url, "offset": offset});
                crate::report::stdout_line(progress.as_ref(), &entry.to_string());
            } else {
                crate::report::stdout_line(progress.as_ref(), &url);
            }
        }
        Ok(Outcome::Found)
    }
}
