use urlencoding::encode;

/// Build the Usher VOD manifest URL from a playback token and signature.
///
/// # Arguments
///
/// * `vod_id` - Twitch video ID.
/// * `signature` - Token signature from the VOD playback token.
/// * `token` - JSON token payload (URL-encoded by this function).
///
/// # Returns
///
/// Usher master playlist URL listing every quality variant.
pub fn vod_manifest_url(vod_id: &str, signature: &str, token: &str) -> String {
    let encoded = encode(token);
    format!(
        "https://usher.ttvnw.net/vod/{vod_id}.m3u8?allow_source=true&platform=web&player_backend=mediaplayer&player_type=embed&sig={signature}&supported_codecs=av1,h265,h264&token={encoded}"
    )
}

/// Collect variant playlist URLs from a master manifest body.
///
/// # Arguments
///
/// * `body` - Master `.m3u8` text.
///
/// # Returns
///
/// Absolute playlist URLs in manifest order.
pub fn variant_playlists(body: &str) -> Vec<String> {
    body.lines()
        .filter(|line| line.starts_with("https://"))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_url_encodes_token() {
        let url = vod_manifest_url("123", "sig", r#"{"vod_id":123}"#);
        assert!(url.starts_with("https://usher.ttvnw.net/vod/123.m3u8?"));
        assert!(url.contains("sig=sig"));
        assert!(url.contains("token=%7B%22vod_id%22%3A123%7D"));
    }

    #[test]
    fn variants_collect_https_lines() {
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nhttps://cdn.example.com/720.m3u8\n#EXT-X-ENDLIST\n";
        assert_eq!(
            variant_playlists(body),
            ["https://cdn.example.com/720.m3u8".to_string()]
        );
    }

    #[test]
    fn variants_ignore_non_urls() {
        assert!(variant_playlists("#EXTM3U\n#EXT-X-ENDLIST\n").is_empty());
    }
}
