use std::collections::HashMap;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use m3u8_rs::{parse_playlist_res, Playlist};
use reqwest::Client;
use std::fs::{File, OpenOptions};
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

/// Create the output file, refusing to overwrite unless forced.
///
/// Atomic `create_new` closes the check-then-create race: an existing file
/// fails here instead of being truncated.
///
/// # Arguments
///
/// * `output` - User-supplied path or `None` for the default.
/// * `force` - Overwrite an existing file.
///
/// # Returns
///
/// Open file handle.
///
/// # Errors
///
/// Returns an error when the file exists and `force` is false, or when
/// creation fails otherwise.
fn create_output(output: Option<String>, force: bool) -> Result<File> {
    let path = output.unwrap_or_else(|| "fixed_playlist.m3u8".to_string());
    if force {
        return File::create(&path).context("Failed to create output file");
    }
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::bail!("Refusing to overwrite existing file: {path} (use --force)")
        }
        Err(error) => Err(error).context("Failed to create output file"),
    }
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
/// * `force` - Overwrite the output file when it exists.
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
    force: bool,
    flags: &Cli,
) -> Result<()> {
    let output_name = output
        .clone()
        .unwrap_or_else(|| "fixed_playlist.m3u8".to_string());

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

    let candidate_count = candidates.len();
    let checked: Vec<(usize, Option<String>, bool)> = stream::iter(candidates)
        .map(|(index, muted)| {
            let client = client.clone();
            async move {
                match probe_head(&client, muted.as_str()).await {
                    Probe::Hit => (index, Some(muted.to_string()), false),
                    Probe::Miss => (index, None, false),
                    Probe::Failed => (index, None, true),
                }
            }
        })
        .buffer_unordered(usize::from(flags.threads))
        .collect()
        .await;

    let mut swapped = 0_usize;
    let mut failed = 0_u64;
    let mut replacements = HashMap::new();
    for (index, muted, probe_failed) in checked {
        if let Some(url) = muted {
            replacements.insert(index, url);
            swapped += 1;
        }
        if probe_failed {
            failed += 1;
        }
    }
    for (index, segment) in playlist.segments.iter_mut().enumerate() {
        segment.uri = replacements
            .remove(&index)
            .unwrap_or_else(|| absolute_uris[index].clone());
    }

    let mut file = create_output(output, force)?;
    let mut buffer = Vec::new();
    playlist
        .write_to(&mut buffer)
        .context("Failed to write M3U8 to buffer")?;
    file.write_all(&buffer).context("Failed to write to file")?;

    eprintln!(
        "Swapped {swapped} of {candidate_count} unmuted segments to muted ({failed} probes failed); saved to: {output_name}",
    );
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
    fn creates_missing_output_file() {
        let path = std::env::temp_dir().join("tbf-fix-missing-output.m3u8");
        let _ = std::fs::remove_file(&path);
        create_output(Some(path.to_string_lossy().to_string()), false)
            .expect("missing file is created");
        assert!(path.exists());
        std::fs::remove_file(&path).expect("fixture cleanup");
    }

    #[test]
    fn rejects_existing_output_file() {
        let path = std::env::temp_dir().join("tbf-fix-clobber-test.m3u8");
        std::fs::write(&path, "data").expect("fixture writes");
        let error = create_output(Some(path.to_string_lossy().to_string()), false)
            .expect_err("existing file is rejected");
        assert!(error.to_string().contains("Refusing to overwrite"));
        std::fs::remove_file(&path).expect("fixture cleanup");
    }

    #[test]
    fn force_overwrites_existing_file() {
        let path = std::env::temp_dir().join("tbf-fix-force-test.m3u8");
        std::fs::write(&path, "data").expect("fixture writes");
        create_output(Some(path.to_string_lossy().to_string()), true).expect("force overwrites");
        std::fs::remove_file(&path).expect("fixture cleanup");
    }
}
