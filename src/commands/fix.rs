use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use m3u8_rs::{parse_playlist_res, Playlist};
use reqwest::Client;
use std::fs::File;
use std::io::Write;
use url::Url;

use crate::cli::Cli;
use crate::twitch::check::{probe_head, Probe};

/// Build the muted alternative for a segment URL, if it is an unmuted file.
///
/// Only the filename is rewritten; the rest of the URL is preserved.
///
/// # Arguments
///
/// * `segment_url` - Absolute segment URL.
///
/// # Returns
///
/// Muted URL when the filename contains `unmuted`, `None` otherwise.
fn muted_alternative(segment_url: &Url) -> Option<Url> {
    let filename = segment_url.path_segments()?.next_back()?;
    if !filename.contains("unmuted") {
        return None;
    }
    let muted = filename.replace("unmuted", "muted");
    let mut url = segment_url.clone();
    let mut segments = url.path_segments_mut().ok()?;
    segments.pop();
    segments.push(&muted);
    drop(segments);
    Some(url)
}

/// Resolve the output path, refusing to overwrite an existing file.
///
/// # Arguments
///
/// * `output` - User-supplied path or `None` for the default.
///
/// # Returns
///
/// Writable output path.
///
/// # Errors
///
/// Returns an error when the resolved path already exists.
fn resolve_output(output: Option<String>) -> Result<String> {
    let path = output.unwrap_or_else(|| "fixed_playlist.m3u8".to_string());
    if Path::new(&path).exists() {
        anyhow::bail!("Refusing to overwrite existing file: {path}");
    }
    Ok(path)
}

/// Rewrite an unmuted playlist, preferring muted segments when available.
///
/// Muted candidates are checked concurrently. Relative segment URIs become
/// absolute against the playlist URL.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `url` - Source `index-dvr.m3u8` URL.
/// * `output` - Output path, defaults to `fixed_playlist.m3u8`.
/// * `flags` - Global CLI flags controlling concurrency.
///
/// # Errors
///
/// Returns an error when fetching, parsing, or writing the playlist fails,
/// or when the output file already exists.
pub async fn execute(
    client: &Client,
    url: &str,
    output: Option<String>,
    flags: &Cli,
) -> Result<()> {
    let output_path = resolve_output(output)?;

    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to fetch playlist")?;
    if !response.status().is_success() {
        anyhow::bail!("Playlist fetch failed with status {}", response.status());
    }
    let body = response
        .text()
        .await
        .context("Failed to read playlist body")?;

    let parsed =
        parse_playlist_res(body.as_bytes()).map_err(|error| anyhow::anyhow!("{error:?}"))?;

    let mut playlist = match parsed {
        Playlist::MediaPlaylist(media) => media,
        Playlist::MasterPlaylist(_) => anyhow::bail!("Expected a media playlist"),
    };

    let base = Url::parse(url).context("Invalid playlist URL")?;
    let mut absolute_uris = Vec::with_capacity(playlist.segments.len());
    for segment in &playlist.segments {
        let absolute = base
            .join(&segment.uri)
            .context("Failed to resolve segment URL")?;
        absolute_uris.push(absolute.to_string());
    }

    let candidates: Vec<(usize, Url)> = absolute_uris
        .iter()
        .enumerate()
        .filter_map(|(index, uri)| {
            let parsed = Url::parse(uri).ok()?;
            muted_alternative(&parsed).map(|muted| (index, muted))
        })
        .collect();

    let swaps: Vec<(usize, String)> = stream::iter(candidates)
        .map(|(index, muted)| {
            let client = client.clone();
            async move {
                (probe_head(&client, muted.as_str()).await == Probe::Hit)
                    .then(|| (index, muted.into()))
            }
        })
        .buffer_unordered(usize::from(flags.threads))
        .filter_map(|item| async move { item })
        .collect()
        .await;

    let swapped: HashMap<usize, String> = swaps.into_iter().collect();
    for (index, segment) in playlist.segments.iter_mut().enumerate() {
        segment.uri = swapped
            .get(&index)
            .cloned()
            .unwrap_or_else(|| absolute_uris[index].clone());
    }

    let mut file = File::create(&output_path).context("Failed to create output file")?;
    let mut buffer = Vec::new();
    playlist
        .write_to(&mut buffer)
        .context("Failed to write M3U8 to buffer")?;
    file.write_all(&buffer).context("Failed to write to file")?;

    println!("Successfully fixed playlist and saved to: {output_path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment_url(raw: &str) -> Url {
        Url::parse(raw).expect("fixture URL parses")
    }

    #[test]
    fn unmuted_filename_maps_to_muted() {
        let url = segment_url("https://cdn.example.com/hash/chunked/1-unmuted.ts");
        assert_eq!(
            muted_alternative(&url)
                .expect("muted variant exists")
                .as_str(),
            "https://cdn.example.com/hash/chunked/1-muted.ts"
        );
    }

    #[test]
    fn muted_filename_has_no_alternative() {
        let url = segment_url("https://cdn.example.com/hash/chunked/1-muted.ts");
        assert!(muted_alternative(&url).is_none());
    }

    #[test]
    fn plain_filename_has_no_alternative() {
        let url = segment_url("https://cdn.example.com/hash/chunked/1.ts");
        assert!(muted_alternative(&url).is_none());
    }

    #[test]
    fn query_string_survives_rewrite() {
        let url = segment_url("https://cdn.example.com/1-unmuted.ts?token=abc");
        let muted = muted_alternative(&url).expect("muted variant exists");
        assert!(muted.as_str().contains("1-muted.ts"));
        assert!(muted.as_str().contains("token=abc"));
    }

    #[test]
    fn resolves_missing_output_file() {
        let missing = std::env::temp_dir().join("tbf-fix-missing-output.m3u8");
        let _ = std::fs::remove_file(&missing);
        let resolved = resolve_output(Some(missing.to_string_lossy().to_string()))
            .expect("missing file resolves");
        assert!(resolved.ends_with("tbf-fix-missing-output.m3u8"));
    }

    #[test]
    fn rejects_existing_output_file() {
        let path = std::env::temp_dir().join("tbf-fix-clobber-test.m3u8");
        std::fs::write(&path, "data").expect("fixture writes");
        let error = resolve_output(Some(path.to_string_lossy().to_string()))
            .expect_err("existing file is rejected");
        assert!(error.to_string().contains("Refusing to overwrite"));
        std::fs::remove_file(&path).expect("fixture cleanup");
    }
}
