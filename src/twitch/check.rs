use futures::stream::{self, StreamExt};
use reqwest::Client;
use sha1::{Digest, Sha1};

use crate::twitch::cdns::DEFAULT_CDNS;

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

/// Build the `index-dvr.m3u8` URL for one CDN and quality variant.
///
/// # Arguments
///
/// * `cdn` - CDN host serving the VOD.
/// * `username` - Streamer login name.
/// * `vod_id` - VOD/broadcast ID.
/// * `timestamp` - Unix epoch seconds.
/// * `quality` - Quality variant (see `QUALITIES`).
///
/// # Returns
///
/// Direct URL to the variant's `index-dvr.m3u8` playlist.
pub fn playlist_url(
    cdn: &str,
    username: &str,
    vod_id: i64,
    timestamp: i64,
    quality: &str,
) -> String {
    let hash = generate_hash(username, vod_id, timestamp);
    format!("https://{cdn}/{hash}_{username}_{vod_id}_{timestamp}/{quality}/index-dvr.m3u8")
}

/// Probe default CDNs for `index-dvr.m3u8` playlists at a timestamp.
///
/// Checks every CDN and quality variant concurrently, then returns hits in
/// CDN/quality order. Faster and more reliable than guessing filenames.
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
/// Playable playlists found across CDNs and qualities.
/// Empty when nothing is available.
pub async fn check_availability(
    client: &Client,
    username: &str,
    vod_id: i64,
    timestamp: i64,
    concurrency: usize,
) -> Vec<VodInfo> {
    let candidates: Vec<(usize, usize, String)> = DEFAULT_CDNS
        .iter()
        .enumerate()
        .flat_map(|(cdn_index, cdn)| {
            QUALITIES
                .iter()
                .enumerate()
                .map(move |(quality_index, quality)| {
                    (
                        cdn_index,
                        quality_index,
                        playlist_url(cdn, username, vod_id, timestamp, quality),
                    )
                })
        })
        .collect();

    let mut found: Vec<(usize, usize, VodInfo)> = stream::iter(candidates)
        .map(|(cdn_index, quality_index, url)| {
            let client = client.clone();
            async move {
                let available = client
                    .head(&url)
                    .send()
                    .await
                    .map(|response| response.status().is_success())
                    .unwrap_or(false);
                available.then(|| {
                    let quality = QUALITIES[quality_index].to_string();
                    (
                        cdn_index,
                        quality_index,
                        VodInfo {
                            playlist_url: url,
                            quality,
                        },
                    )
                })
            }
        })
        .buffer_unordered(concurrency.max(1))
        .filter_map(|item| async move { item })
        .collect()
        .await;

    found.sort_by_key(|(cdn_index, quality_index, _)| (*cdn_index, *quality_index));
    found.into_iter().map(|(_, _, info)| info).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            playlist_url(
                "dgeft87wbj63p.cloudfront.net",
                "arquel",
                316_969_565_142,
                1_790_752_835,
                "chunked"
            ),
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
}
