use anyhow::{Context, Result};
use m3u8_rs::{parse_playlist_res, Playlist};
use reqwest::Client;
use std::fs::File;
use std::io::Write;

use crate::cli::Cli;

/// Rewrite an unmuted playlist, preferring muted segments when available.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `url` - Source `index-dvr.m3u8` URL.
/// * `output` - Output path, defaults to `fixed_playlist.m3u8`.
/// * `_flags` - Global CLI flags (currently unused).
///
/// # Errors
///
/// Returns an error when fetching, parsing, or writing the playlist fails.
pub async fn execute(
    client: &Client,
    url: &str,
    output: Option<String>,
    _flags: &Cli,
) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to fetch playlist")?;
    let body = response
        .text()
        .await
        .context("Failed to read playlist body")?;

    let parsed =
        parse_playlist_res(body.as_bytes()).map_err(|error| anyhow::anyhow!("{error:?}"))?;

    let playlist = match parsed {
        Playlist::MediaPlaylist(media) => media,
        Playlist::MasterPlaylist(_) => anyhow::bail!("Expected a media playlist"),
    };

    let slash = url.rfind('/').unwrap_or(0);
    let base_url = format!("{}/", &url[..slash]);

    let mut fixed_segments = Vec::with_capacity(playlist.segments.len());
    for mut segment in playlist.segments.clone() {
        if !segment.uri.starts_with("http") {
            let absolute = format!("{base_url}{}", segment.uri);
            if absolute.contains("unmuted") {
                let muted = absolute.replace("unmuted", "muted");
                if let Ok(response) = client.head(&muted).send().await {
                    if response.status().is_success() {
                        segment.uri = muted;
                        fixed_segments.push(segment);
                        continue;
                    }
                }
            }
            segment.uri = absolute;
        }
        fixed_segments.push(segment);
    }

    let mut playlist = playlist;
    playlist.segments = fixed_segments;

    let output_path = output.unwrap_or_else(|| "fixed_playlist.m3u8".to_string());
    let mut file = File::create(&output_path).context("Failed to create output file")?;
    let mut buffer = Vec::new();
    playlist
        .write_to(&mut buffer)
        .context("Failed to write M3U8 to buffer")?;
    file.write_all(&buffer).context("Failed to write to file")?;

    println!("Successfully fixed playlist and saved to: {output_path}");
    Ok(())
}
