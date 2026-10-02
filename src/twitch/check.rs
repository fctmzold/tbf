use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::stream::{self, StreamExt};
use reqwest::Client;
use sha1::{Digest, Sha1};
use tokio::sync::Semaphore;

use crate::twitch::cdns::DEFAULT_CDNS;
use crate::twitch::retry::{Failure, classify_request, retry_after_secs, with_retry};

/// Wall-clock milliseconds since the Unix epoch.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
pub fn generate_hash(username: &str, vod_id: u64, timestamp: i64) -> String {
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
pub fn url_stem(username: &str, vod_id: u64, timestamp: i64) -> String {
    let hash = generate_hash(username, vod_id, timestamp);
    format!("{hash}_{username}_{vod_id}_{timestamp}")
}

/// Build the `index-dvr.m3u8` URL for one CDN and quality variant.
///
/// Hosts containing `://` (loopback test servers, custom mirrors) are used
/// as-is; bare hostnames get `https://`.
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
    if cdn.contains("://") {
        format!("{cdn}/{stem}/{quality}/index-dvr.m3u8")
    } else {
        format!("https://{cdn}/{stem}/{quality}/index-dvr.m3u8")
    }
}

/// Number of probed CDN hosts.
pub const DEFAULT_CDN_COUNT: usize = DEFAULT_CDNS.len();

/// HTTP prober bounding concurrent requests with a shared semaphore.
///
/// Previously every scan layer took a `concurrency` number and nested them,
/// so real concurrency multiplied (`threads` timestamp tasks each fanning
/// out to `threads` probes). Permits here are the single global bound: only
/// `permits` requests are ever in flight, however scans nest.
///
/// Hosts are configurable so CDN rotations do not require recompiling; tests
/// point the prober at loopback servers the same way.
///
/// # Examples
///
/// ```
/// use tbf_new::twitch::check::Prober;
///
/// let prober = Prober::new(reqwest::Client::new(), 100);
/// assert_eq!(prober.permits(), 100);
/// ```
#[derive(Debug, Clone)]
pub struct Prober {
    client: Client,
    sem: Arc<Semaphore>,
    permits: usize,
    hosts: Vec<String>,
    /// Shared unix-millis "not before" deadline after a 429.
    cooldown_until: Arc<AtomicU64>,
}

impl Prober {
    /// Create a prober sharing one request budget.
    ///
    /// Probes the built-in CDN hosts.
    ///
    /// # Arguments
    ///
    /// * `client` - Shared HTTP client.
    /// * `permits` - Maximum simultaneous requests; clamped to at least 1.
    pub fn new(client: Client, permits: usize) -> Self {
        Self::with_hosts(
            client,
            permits,
            DEFAULT_CDNS.iter().map(|host| (*host).to_string()),
        )
    }

    /// Create a prober with explicit CDN hosts.
    ///
    /// An empty host list falls back to the built-in hosts.
    ///
    /// # Arguments
    ///
    /// * `client` - Shared HTTP client.
    /// * `permits` - Maximum simultaneous requests; clamped to at least 1.
    /// * `hosts` - CDN hosts (or full base URLs) to probe, in order.
    pub fn with_hosts(
        client: Client,
        permits: usize,
        hosts: impl IntoIterator<Item = String>,
    ) -> Self {
        let permits = permits.max(1);
        let hosts: Vec<String> = hosts.into_iter().collect();
        let hosts = if hosts.is_empty() {
            DEFAULT_CDNS
                .iter()
                .map(|host| (*host).to_string())
                .collect()
        } else {
            hosts
        };
        Self {
            client,
            sem: Arc::new(Semaphore::new(permits)),
            permits,
            hosts,
            cooldown_until: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Configured request budget.
    pub fn permits(&self) -> usize {
        self.permits
    }

    /// Number of probed hosts; the request estimate for one timestamp.
    pub fn host_count(&self) -> usize {
        self.hosts.len()
    }

    /// Delay after a 429: pause this task and tell every sibling task to
    /// hold until the deadline, so the whole scan backs off together.
    ///
    /// # Arguments
    ///
    /// * `delay` - How long to wait; capped at one minute.
    async fn cool_down(&self, delay: Duration) {
        let delay = delay.min(Duration::from_secs(60));
        let until = now_millis().saturating_add(delay.as_millis() as u64);
        self.cooldown_until.fetch_max(until, Ordering::Relaxed);
        tokio::time::sleep(delay).await;
    }

    /// Wait out a shared 429 cooldown before the next attempt.
    async fn wait_for_cooldown(&self) {
        let remaining = self
            .cooldown_until
            .load(Ordering::Relaxed)
            .saturating_sub(now_millis())
            .min(60_000);
        if remaining > 0 {
            tokio::time::sleep(Duration::from_millis(remaining)).await;
        }
    }

    /// HEAD a URL, retrying timeouts, resets, 429s, and 5xx responses.
    ///
    /// One semaphore permit is held per attempt, so nested scans share the
    /// global budget. Persistent 429s and 5xx responses count as `Failed`,
    /// not `Miss`: the server never gave a usable answer.
    ///
    /// # Arguments
    ///
    /// * `url` - URL to probe.
    ///
    /// # Returns
    ///
    /// `Hit` when the URL exists, `Miss` when the server says it does not,
    /// `Failed` when no usable answer arrived.
    pub async fn probe_head(&self, url: &str) -> Probe {
        match with_retry(|| self.head_once(url), 3).await {
            Ok(probe) => probe,
            Err(_) => Probe::Failed,
        }
    }

    /// One HEAD attempt with error classification for retries.
    ///
    /// The semaphore permit is released before any `Retry-After` sleep, so
    /// a burst of 429s cannot stall the whole request budget.
    async fn head_once(&self, url: &str) -> Result<Probe, (Failure, anyhow::Error)> {
        self.wait_for_cooldown().await;
        let response = {
            let _permit = self
                .sem
                .acquire()
                .await
                .map_err(|_| (Failure::Permanent, anyhow::anyhow!("prober shut down")))?;
            self.client.head(url).send().await
        };
        match response {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    Ok(Probe::Hit)
                } else if status.as_u16() == 429 {
                    if let Some(seconds) = retry_after_secs(&response) {
                        self.cool_down(Duration::from_secs(seconds)).await;
                    }
                    Err((
                        Failure::RateLimited,
                        anyhow::anyhow!("HEAD {url} answered {status}"),
                    ))
                } else if status.is_server_error() {
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
}

/// Number of HEAD probes per timestamp (hosts times qualities).
///
/// # Arguments
///
/// * `prober` - Shared prober carrying the CDN hosts.
///
/// # Returns
///
/// Total probe count for one `check_availability` call.
pub fn probe_total(prober: &Prober) -> usize {
    prober.host_count() * QUALITIES.len()
}

/// Probe default CDNs for `index-dvr.m3u8` playlists at a timestamp.
///
/// Checks every CDN and quality variant concurrently, then returns hits in
/// CDN/quality order with a count of failed probes. Actual request
/// concurrency is bounded by the prober's semaphore.
///
/// # Arguments
///
/// * `prober` - Shared prober bounding concurrent requests.
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
///
/// # Returns
///
/// Hits and failed-probe count. Empty hits with zero failures means the VOD
/// is not there; failures mean the answer is unreliable.
pub async fn check_availability(
    prober: &Prober,
    username: &str,
    vod_id: u64,
    timestamp: i64,
) -> ScanOutcome {
    check_qualities(prober, username, vod_id, timestamp, QUALITIES).await
}

/// Probe selected quality variants at a timestamp.
///
/// Same as `check_availability` but restricted to `qualities`, so callers
/// can probe `chunked` first and expand only on hits.
///
/// # Arguments
///
/// * `prober` - Shared prober bounding concurrent requests.
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
/// * `qualities` - Variants to probe, in output order.
///
/// # Returns
///
/// Hits and failed-probe count.
pub async fn check_qualities(
    prober: &Prober,
    username: &str,
    vod_id: u64,
    timestamp: i64,
    qualities: &[&str],
) -> ScanOutcome {
    let stem = url_stem(username, vod_id, timestamp);
    let candidates: Vec<(usize, usize, String)> = prober
        .hosts
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

    // All candidates run as tasks at once; the prober's semaphore is the
    // real bound on simultaneous requests.
    let window = candidates.len().max(1);
    let probed: Vec<(usize, usize, Option<VodInfo>, bool)> = stream::iter(candidates)
        .map(|(cdn_index, quality_index, url)| {
            let quality = qualities[quality_index].to_string();
            async move {
                match prober.probe_head(&url).await {
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
        .buffer_unordered(window)
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
        let prober = Prober::new(Client::new(), 8);
        let base = serve(vec![200]);
        assert_eq!(
            prober.probe_head(&format!("{base}/x.m3u8")).await,
            Probe::Hit
        );
    }

    #[tokio::test]
    async fn missing_url_is_miss() {
        let prober = Prober::new(Client::new(), 8);
        let base = serve(vec![404]);
        assert_eq!(
            prober.probe_head(&format!("{base}/x.m3u8")).await,
            Probe::Miss
        );
    }

    /// Run a probe with frozen time, fast-forwarding through backoffs.
    async fn run_paused<T>(task: impl std::future::Future<Output = T> + Send + 'static) -> T
    where
        T: Send + 'static,
    {
        tokio::time::pause();
        let handle = tokio::spawn(task);
        tokio::time::advance(std::time::Duration::from_secs(120)).await;
        handle.await.expect("probe completes")
    }

    #[tokio::test]
    async fn refused_connection_is_failed() {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind loopback")
            .local_addr()
            .expect("read local addr")
            .port();
        let prober = Prober::new(Client::new(), 8);
        assert_eq!(
            run_paused(async move {
                prober
                    .probe_head(&format!("http://127.0.0.1:{port}/x.m3u8"))
                    .await
            })
            .await,
            Probe::Failed
        );
    }

    #[tokio::test]
    async fn rate_limit_then_success_is_hit() {
        let prober = Prober::new(Client::new(), 8);
        let base = serve(vec![429, 200]);
        assert_eq!(
            run_paused(async move { prober.probe_head(&format!("{base}/x.m3u8")).await }).await,
            Probe::Hit
        );
    }

    #[tokio::test]
    async fn persistent_server_error_is_failed() {
        let prober = Prober::new(Client::new(), 8);
        let base = serve(vec![500, 500, 500]);
        assert_eq!(
            run_paused(async move { prober.probe_head(&format!("{base}/x.m3u8")).await }).await,
            Probe::Failed
        );
    }

    #[tokio::test]
    async fn zero_permits_clamps_to_one() {
        let prober = Prober::new(Client::new(), 0);
        assert_eq!(prober.permits(), 1);
        let base = serve(vec![200]);
        assert_eq!(
            prober.probe_head(&format!("{base}/x.m3u8")).await,
            Probe::Hit
        );
    }

    #[tokio::test]
    async fn availability_scan_hits_loopback_host() {
        let base = serve(vec![200; 7]);
        let prober = Prober::with_hosts(Client::new(), 8, [base.clone()]);
        let outcome = check_availability(&prober, "arquel", 316_969_565_142, 1_790_752_835).await;
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.hits.len(), QUALITIES.len());
        assert!(
            outcome
                .hits
                .iter()
                .all(|hit| hit.playlist_url.starts_with(&base))
        );
    }

    #[tokio::test]
    async fn prober_bounds_in_flight_requests() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const PROBES: usize = 7;
        const PERMITS: usize = 4;
        let live = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("read local addr");
        let peak = max.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..PROBES {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let live = live.clone();
                let max = peak.clone();
                std::thread::spawn(move || {
                    let current = live.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(current, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let mut request = [0_u8; 1024];
                    let _ = stream.read(&mut request);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    live.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        let prober = Prober::with_hosts(Client::new(), PERMITS, [format!("http://{addr}")]);
        let outcome = check_availability(&prober, "arquel", 1, 2).await;
        server.join().expect("server finishes");
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.hits.len(), PROBES);
        let peak = max.load(Ordering::SeqCst);
        assert!(peak <= PERMITS, "peak {peak} exceeds {PERMITS} permits");
        assert!(peak >= 2, "peak {peak} shows no parallelism");
    }
}
