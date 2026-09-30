use anyhow::{Context, Result};
use m3u8_rs::{parse_playlist_res, Playlist};
use url::Url;

/// One playable variant from a master manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Direct playlist URL.
    pub url: String,
    /// Human label like `1920x1080` or `6398281bps`.
    pub label: String,
}

/// Build the Usher VOD manifest URL from a playback token and signature.
///
/// # Arguments
///
/// * `vod_id` - Twitch video ID.
/// * `signature` - Token signature from the VOD playback token.
/// * `token` - JSON token payload (encoded into the query string).
///
/// # Returns
///
/// Usher master playlist URL listing every quality variant.
///
/// # Errors
///
/// Returns an error when the static base URL fails to parse, which signals
/// a programming error rather than bad input.
pub fn vod_manifest_url(vod_id: &str, signature: &str, token: &str) -> Result<String> {
    let mut url = Url::parse(&format!("https://usher.ttvnw.net/vod/{vod_id}.m3u8"))
        .context("Static Usher base URL failed to parse")?;
    url.query_pairs_mut()
        .append_pair("allow_source", "true")
        .append_pair("platform", "web")
        .append_pair("player_backend", "mediaplayer")
        .append_pair("player_type", "embed")
        .append_pair("sig", signature)
        .append_pair("supported_codecs", "av1,h265,h264")
        .append_pair("token", token)
        .finish();
    Ok(url.into())
}

/// Collect playable variants from a master manifest body.
///
/// # Arguments
///
/// * `body` - Master `.m3u8` text.
///
/// # Returns
///
/// Variants in manifest order, empty for media playlists or garbage.
pub fn variant_playlists(body: &str) -> Vec<Variant> {
    let parsed = parse_playlist_res(body.as_bytes());
    let Ok(Playlist::MasterPlaylist(master)) = parsed else {
        return Vec::new();
    };
    master
        .variants
        .into_iter()
        .map(|variant| Variant {
            label: variant
                .resolution
                .map(|size| size.to_string())
                .unwrap_or_else(|| format!("{}bps", variant.bandwidth)),
            url: variant.uri,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_url_encodes_token() {
        let url = vod_manifest_url("123", "sig", r#"{"vod_id":123}"#).expect("URL builds");
        assert!(url.starts_with("https://usher.ttvnw.net/vod/123.m3u8?"));
        assert!(url.contains("sig=sig"));
        assert!(url.contains("token=%7B%22vod_id%22%3A123%7D"));
    }

    #[test]
    fn variants_carry_labels_and_urls() {
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100,RESOLUTION=1280x720\nhttps://cdn.example.com/720.m3u8\n";
        assert_eq!(
            variant_playlists(body),
            [Variant {
                url: "https://cdn.example.com/720.m3u8".to_string(),
                label: "1280x720".to_string(),
            }]
        );
    }

    #[test]
    fn variants_fall_back_to_bandwidth() {
        let body =
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=64000\nhttps://cdn.example.com/audio.m3u8\n";
        assert_eq!(
            variant_playlists(body),
            [Variant {
                url: "https://cdn.example.com/audio.m3u8".to_string(),
                label: "64000bps".to_string(),
            }]
        );
    }

    #[test]
    fn garbage_manifest_yields_no_variants() {
        assert!(variant_playlists("#EXTM3U\n#EXT-X-ENDLIST\n").is_empty());
        assert!(variant_playlists("not a playlist").is_empty());
    }
}
