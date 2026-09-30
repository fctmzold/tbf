/// Default Twitch VOD CDN hosts probed for chunked playlists.
pub const DEFAULT_CDNS: &[&str] = &[
    "d1m7jfoe9zdc1j.cloudfront.net",
    "d2vjef5jvl6bfs.cloudfront.net",
    "d2nvs31859zcd8.cloudfront.net",
    "d2aba986w2f7eo.cloudfront.net",
    "d3vdhqjm8zsro9.cloudfront.net",
    "d1311y22p51b8x.cloudfront.net",
    "d3fi1amfgojobc.cloudfront.net",
    "dgeft87wbj63p.cloudfront.net",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdn_list_is_not_empty() {
        assert!(!DEFAULT_CDNS.is_empty());
    }

    #[test]
    fn cdn_list_covers_known_vod_host() {
        assert!(DEFAULT_CDNS.contains(&"dgeft87wbj63p.cloudfront.net"));
    }
}
