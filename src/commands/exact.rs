use anyhow::Result;
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::GlobalOpts;
use crate::commands::Outcome;
use crate::progress::scanning_progress;
use crate::report::{emit_hit, hint, note, suggest_player};
use crate::twitch::check::{self, Prober};
use crate::util::format_utc;

/// Check offsets around a timestamp for a playable VOD.
///
/// Offsets stream through an ordered `buffered` window, so the
/// closest-to-center hit still wins and the stream drops on the first hit.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `id` - VOD/broadcast ID.
/// * `timestamp` - Base Unix epoch seconds.
/// * `opts` - Global CLI options controlling output and scan window.
///
/// # Errors
///
/// Returns an error when every probe failed, leaving no reliable answer.
pub async fn execute(
    client: &Client,
    username: &str,
    id: u64,
    timestamp: i64,
    opts: &GlobalOpts,
) -> Result<Outcome> {
    let username = username.to_lowercase();
    let window = opts.window;
    let offsets = std::iter::once(0).chain((1..=window).flat_map(|delta| {
        let delta = delta as i64;
        [delta, -delta]
    }));

    eprintln!(
        "Checking offsets from -{window} to +{window} around {}...",
        format_utc(timestamp)
    );
    let progress = scanning_progress(
        2 * window + 1,
        "Checking offsets...",
        opts.simple,
        opts.progressbar,
    );

    let prober = Prober::with_hosts(client.clone(), usize::from(opts.threads), opts.cdn_hosts());
    let probe_each = check::probe_total(&prober) as u64;
    let tasks = usize::from(opts.threads).max(1);
    let mut results = stream::iter(offsets)
        .map(|offset| {
            let prober = prober.clone();
            let username = username.clone();
            let progress = progress.clone();
            async move {
                let current = timestamp.saturating_add(offset);
                let outcome = check::check_availability(&prober, &username, id, current).await;
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                (offset, current, outcome)
            }
        })
        .buffered(tasks);

    let mut failed_total = 0_u64;
    let mut probe_total = 0_u64;
    while let Some((offset, current, outcome)) = results.next().await {
        probe_total += probe_each;
        failed_total += outcome.failed;
        if outcome.hits.is_empty() {
            if outcome.failed > 0 {
                note(
                    progress.as_ref(),
                    format!(
                        "Warning: {} of {probe_each} probes failed at offset {offset}; results may be incomplete.",
                        outcome.failed
                    ),
                );
            }
            continue;
        }
        if let Some(bar) = &progress {
            bar.finish_with_message("Search complete");
        }
        note(
            progress.as_ref(),
            format!(
                "Found VOD at offset {offset} (Timestamp: {current} = {})",
                format_utc(current)
            ),
        );
        for info in &outcome.hits {
            emit_hit(
                progress.as_ref(),
                info,
                opts.simple,
                opts.json,
                Some(current),
            );
        }
        suggest_player(&outcome.hits[0].playlist_url, opts.simple);
        return Ok(Outcome::Found);
    }
    drop(results);

    if let Some(bar) = progress {
        bar.finish_with_message("Search complete");
    }
    if probe_total > 0 && failed_total == probe_total {
        anyhow::bail!("All {probe_total} probes failed; check your connection and try again.");
    }
    eprintln!("Could not find available VOD in the specified range.");
    hint("confirm the timestamp with `link`, or widen the search with `bruteforce`");
    Ok(Outcome::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve 200 for playlist paths of `target`, 404 for everything else.
    fn serve_gated(target: i64) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("read local addr");
        let marker = format!("_{target}/");
        std::thread::spawn(move || {
            for _ in 0..200 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let marker = marker.clone();
                std::thread::spawn(move || {
                    let mut request = [0_u8; 2048];
                    let _ = stream.read(&mut request);
                    let head = String::from_utf8_lossy(&request);
                    let path = head.split_whitespace().nth(1).unwrap_or_default();
                    let (status, reason) = if path.contains(&marker) {
                        ("200", "OK")
                    } else {
                        ("404", "Not Found")
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                });
            }
        });
        format!("http://{addr}")
    }

    fn test_opts(base: String) -> GlobalOpts {
        GlobalOpts {
            window: 2,
            threads: 8,
            cdn: [base].into_iter().collect(),
            ..GlobalOpts::default()
        }
    }

    #[tokio::test]
    async fn finds_gated_timestamp() {
        let target = 1_790_752_835_i64;
        let base = serve_gated(target);
        let client = Client::new();
        let opts = test_opts(base);
        assert_eq!(
            super::execute(&client, "arquel", 1, target, &opts)
                .await
                .expect("lookup runs"),
            Outcome::Found
        );
    }

    #[tokio::test]
    async fn misses_range_without_target() {
        let base = serve_gated(9_999_999_999);
        let client = Client::new();
        let opts = test_opts(base);
        assert_eq!(
            super::execute(&client, "arquel", 1, 1_790_752_800, &opts)
                .await
                .expect("lookup runs"),
            Outcome::NotFound
        );
    }
}
