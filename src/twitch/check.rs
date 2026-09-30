use futures::stream::{self, StreamExt};
use reqwest::Client;
use sha1::{Digest, Sha1};

use crate::twitch::cdns::DEFAULT_CDNS;
use crate::twitch::retry::{classify_request, with_retry, Failure};

/// Quality variants probed directly via their `index-dvr.m3u8` playlists.
pub const QUALITIES: &[&str] = &[
    "chunked",
    "720p60",
    "720p30",
    "480p30",
    "360p30",
    "160p30",
    "audio_only",
];

/// A playable VOD playlist discovered on a CDN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VodInfo {
    /// Direct URL to the `index-dvr.m3u8` playlist.
    pub playlist_url: String,
    /// Quality variant the playlist was found at.
    pub quality: String,
}

/// Outcome of a single HEAD probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The URL exists.
    Hit,
    /// The server answered but the URL is missing.
    Miss,
    /// No usable answer (timeout, DNS, or connection error).
    Failed,
}

/// Result of scanning one timestamp across CDNs and qualities.
#[derive(Debug)]
pub struct ScanOutcome {
    /// Playable playlists found, in CDN/quality order.
    pub hits: Vec<VodInfo>,
    /// Probes that failed without an answer.
    pub failed: u64,
}

/// Generate the 20-character hash prefix for a VOD timestamp.
///
/// # Arguments
///
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
///
/// # Returns
///
/// First 20 hex characters of `SHA1("{username}_{vod_id}_{timestamp}")`.
///
/// # Examples
///
/// ```
/// use tbf_new::twitch::check::generate_hash;
///
/// assert_eq!(generate_hash("destiny", 39700667438, 1605781794).len(), 20);
/// ```
pub fn generate_hash(username: &str, vod_id: i64, timestamp: i64) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("{username}_{vod_id}_{timestamp}"));
    let digest = hasher.finalize();
    format!("{digest:x}")[..20].to_string()
}

/// Build the shared `{hash}_{username}_{vod_id}_{timestamp}` URL stem.
///
/// Computing this once per timestamp avoids rehashing for every CDN and
/// quality combination.
///
/// # Arguments
///
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
///
/// # Returns
///
/// URL stem without CDN host, quality, or filename.
pub fn url_stem(username: &str, vod_id: i64, timestamp: i64) -> String {
    let hash = generate_hash(username, vod_id, timestamp);
    format!("{hash}_{username}_{vod_id}_{timestamp}")
}

/// Build the `index-dvr.m3u8` URL for one CDN and quality variant.
///
/// # Arguments
///
/// * `cdn` - CDN host serving the VOD.
/// * `stem` - URL stem from `url_stem`.
/// * `quality` - Quality variant (see `QUALITIES`).
///
/// # Returns
///
/// Direct URL to the variant's `index-dvr.m3u8` playlist.
pub fn playlist_url(cdn: &str, stem: &str, quality: &str) -> String {
    format!("https://{cdn}/{stem}/{quality}/index-dvr.m3u8")
}

/// Number of probed CDN hosts.
pub const DEFAULT_CDN_COUNT: usize = DEFAULT_CDNS.len();

/// Number of HEAD probes per timestamp (CDNs times qualities).
///
/// # Returns
///
/// Total probe count for one `check_availability` call.
pub fn probe_total() -> usize {
    DEFAULT_CDNS.len() * QUALITIES.len()
}

/// HEAD a URL, retrying timeouts, 429s, and 5xx responses.
///
/// Persistent 429s and 5xx responses count as `Failed`, not `Miss`: the
/// server never gave a usable answer.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `url` - URL to probe.
///
/// # Returns
///
/// `Hit` when the URL exists, `Miss` when the server says it does not,
/// `Failed` when no usable answer arrived.
pub async fn probe_head(client: &Client, url: &str) -> Probe {
    match with_retry(|| head_once(client, url), 3).await {
        Ok(probe) => probe,
        Err(_) => Probe::Failed,
    }
}

/// One HEAD attempt with error classification for retries.
async fn head_once(client: &Client, url: &str) -> Result<Probe, (Failure, anyhow::Error)> {
    match client.head(url).send().await {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                Ok(Probe::Hit)
            } else if status.as_u16() == 429 || status.is_server_error() {
                Err((
                    Failure::Transient,
                    anyhow::anyhow!("HEAD {url} answered {status}"),
                ))
            } else {
                Ok(Probe::Miss)
            }
        }
        Err(error) => Err(classify_request(error)),
    }
}

/// Probe default CDNs for `index-dvr.m3u8` playlists at a timestamp.
///
/// Checks every CDN and quality variant concurrently, then returns hits in
/// CDN/quality order with a count of failed probes.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
/// * `concurrency` - Maximum simultaneous HEAD requests.
///
/// # Returns
///
/// Hits and failed-probe count. Empty hits with zero failures means the VOD
/// is not there; failures mean the answer is unreliable.
pub async fn check_availability(
    client: &Client,
    username: &str,
    vod_id: i64,
    timestamp: i64,
    concurrency: usize,
) -> ScanOutcome {
    check_qualities(client, username, vod_id, timestamp, QUALITIES, concurrency).await
}

/// Probe selected quality variants at a timestamp.
///
/// Same as `check_availability` but restricted to `qualities`, so callers
/// can probe `chunked` first and expand only on hits.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
/// * `qualities` - Variants to probe, in output order.
/// * `concurrency` - Maximum simultaneous HEAD requests.
///
/// # Returns
///
/// Hits and failed-probe count.
pub async fn check_qualities(
    client: &Client,
    username: &str,
    vod_id: i64,
    timestamp: i64,
    qualities: &[&str],
    concurrency: usize,
) -> ScanOutcome {
    let stem = url_stem(username, vod_id, timestamp);
    let candidates: Vec<(usize, usize, String)> = DEFAULT_CDNS
        .iter()
        .enumerate()
        .flat_map(|(cdn_index, cdn)| {
            let stem = stem.clone();
            qualities
                .iter()
                .enumerate()
                .map(move |(quality_index, quality)| {
                    (cdn_index, quality_index, playlist_url(cdn, &stem, quality))
                })
        })
        .collect();

    let probed: Vec<(usize, usize, Option<VodInfo>, bool)> = stream::iter(candidates)
        .map(|(cdn_index, quality_index, url)| {
            let client = client.clone();
            let quality = qualities[quality_index].to_string();
            async move {
                match probe_head(&client, &url).await {
                    Probe::Hit => (
                        cdn_index,
                        quality_index,
                        Some(VodInfo {
                            playlist_url: url,
                            quality,
                        }),
                        false,
                    ),
                    Probe::Miss => (cdn_index, quality_index, None, false),
                    Probe::Failed => (cdn_index, quality_index, None, true),
                }
            }
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;

    let mut hits = Vec::new();
    let mut failed = 0_u64;
    for (cdn_index, quality_index, info, probe_failed) in probed {
        if let Some(info) = info {
            hits.push((cdn_index, quality_index, info));
        }
        if probe_failed {
            failed += 1;
        }
    }

    hits.sort_by_key(|(cdn_index, quality_index, _)| (*cdn_index, *quality_index));
    ScanOutcome {
        hits: hits.into_iter().map(|(_, _, info)| info).collect(),
        failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn hash_has_expected_length() {
        assert_eq!(
            generate_hash("destiny", 39_700_667_438, 1_605_781_794).len(),
            20
        );
    }

    #[test]
    fn hash_is_deterministic() {
        let first = generate_hash("streamer", 123, 456);
        let second = generate_hash("streamer", 123, 456);
        assert_eq!(first, second);
    }

    #[test]
    fn hash_differs_per_input() {
        assert_ne!(
            generate_hash("streamer", 123, 456),
            generate_hash("streamer", 123, 457)
        );
    }

    #[test]
    fn hash_matches_known_vod() {
        assert_eq!(
            generate_hash("arquel", 316_969_565_142, 1_790_752_835),
            "5c20ac68ff0f9469ef5e"
        );
    }

    #[test]
    fn playlist_url_matches_known_vod() {
        let stem = url_stem("arquel", 316_969_565_142, 1_790_752_835);
        assert_eq!(
            playlist_url("dgeft87wbj63p.cloudfront.net", &stem, "chunked"),
            "https://dgeft87wbj63p.cloudfront.net/5c20ac68ff0f9469ef5e_arquel_316969565142_1790752835/chunked/index-dvr.m3u8"
        );
    }

    #[test]
    fn qualities_list_is_not_empty() {
        assert!(!QUALITIES.is_empty());
    }

    #[test]
    fn vod_info_stores_fields() {
        let info = VodInfo {
            playlist_url: "https://example.com/index.m3u8".to_string(),
            quality: "chunked".to_string(),
        };
        assert_eq!(info.playlist_url, "https://example.com/index.m3u8");
        assert_eq!(info.quality, "chunked");
    }

    /// Serve canned HTTP statuses from a background thread.
    fn serve(statuses: Vec<u16>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("read local addr");
        std::thread::spawn(move || {
            for status in statuses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Error",
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                );
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn existing_url_is_hit() {
        let client = Client::new();
        let base = serve(vec![200]);
        assert_eq!(
            probe_head(&client, &format!("{base}/x.m3u8")).await,
            Probe::Hit
        );
    }

    #[tokio::test]
    async fn missing_url_is_miss() {
        let client = Client::new();
        let base = serve(vec![404]);
        assert_eq!(
            probe_head(&client, &format!("{base}/x.m3u8")).await,
            Probe::Miss
        );
    }

    #[tokio::test]
    async fn refused_connection_is_failed() {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind loopback")
            .local_addr()
            .expect("read local addr")
            .port();
        let client = Client::new();
        assert_eq!(
            probe_head(&client, &format!("http://127.0.0.1:{port}/x.m3u8")).await,
            Probe::Failed
        );
    }

    #[tokio::test]
    async fn rate_limit_then_success_is_hit() {
        let client = Client::new();
        let base = serve(vec![429, 200]);
        assert_eq!(
            probe_head(&client, &format!("{base}/x.m3u8")).await,
            Probe::Hit
        );
    }

    #[tokio::test]
    async fn persistent_server_error_is_failed() {
        let client = Client::new();
        let base = serve(vec![500, 500, 500]);
        assert_eq!(
            probe_head(&client, &format!("{base}/x.m3u8")).await,
            Probe::Failed
        );
    }
}
