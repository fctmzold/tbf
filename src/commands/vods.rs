use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use indicatif::ProgressBar;
use reqwest::Client;

use crate::cli::Cli;
use crate::progress::scanning_progress;
use crate::twitch::videos::Video;
use crate::twitch::{gql, usher, videos};

/// Print one message without breaking the progress bar.
fn out(progress: &Option<ProgressBar>, message: String) {
    match progress {
        Some(bar) => bar.println(message),
        None => println!("{message}"),
    }
}

/// Resolve one video to its playable variant playlists.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `video` - Listed channel video.
///
/// # Returns
///
/// Variant playlist URLs, or `None` when Twitch issues no token or the
/// manifest holds no variants.
///
/// # Errors
///
/// Returns an error when the token or manifest request fails at the
/// transport level.
async fn resolve_video(client: &Client, video: &Video) -> Result<Option<Vec<String>>> {
    let Some((token, signature)) = gql::get_vod_token(client, &video.id).await? else {
        return Ok(None);
    };
    let url = usher::vod_manifest_url(&video.id, &signature, &token);
    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to fetch VOD manifest")?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let body = response
        .text()
        .await
        .context("Failed to read VOD manifest")?;
    let variants = usher::variant_playlists(&body);
    Ok(if variants.is_empty() {
        None
    } else {
        Some(variants)
    })
}

/// List a channel's VODs with playable `index-dvr.m3u8` links.
///
/// Each video resolves through its VOD playback token to the Usher master
/// manifest, which lists every quality variant.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Channel login name.
/// * `flags` - Global CLI flags controlling concurrency and progress output.
///
/// # Errors
///
/// Returns an error when the video listing cannot be fetched.
pub async fn execute(client: &Client, username: &str, flags: &Cli) -> Result<()> {
    let username = username.to_lowercase();
    if !flags.simple {
        println!("Fetching VOD list for '{username}'...");
    }

    let all = videos::fetch_all_videos(client, &username)
        .await
        .context("Failed to fetch channel videos")?;
    if all.is_empty() {
        println!("No videos found for '{username}'.");
        return Ok(());
    }
    println!("Found {} videos.", all.len());

    let progress = scanning_progress(
        all.len() as u64,
        "Resolving VOD playlists...",
        flags.simple,
        flags.progressbar,
    );

    let (with_playlist, failed) = stream::iter(all)
        .map(|video| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let mut message = format!("{}\n  Link: {}", video.summary(), video.url());
                let outcome = match resolve_video(&client, &video).await {
                    Ok(Some(variants)) => {
                        for variant in &variants {
                            message.push_str(&format!("\n  {variant}"));
                        }
                        (1_u64, 0_u64)
                    }
                    Ok(None) => {
                        message.push_str("\n  No playable playlist found.");
                        (0_u64, 0_u64)
                    }
                    Err(error) => {
                        message.push_str(&format!("\n  Lookup failed: {error:#}"));
                        (0_u64, 1_u64)
                    }
                };
                out(&progress, message);
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                outcome
            }
        })
        .buffer_unordered(usize::from(flags.threads))
        .fold(
            (0_u64, 0_u64),
            |(listed, failed), (listed_one, failed_one)| async move {
                (listed + listed_one, failed + failed_one)
            },
        )
        .await;

    if let Some(bar) = progress {
        bar.finish_with_message("Scan complete");
    }
    println!("{with_playlist} videos with playable playlists.");
    if failed > 0 {
        println!("Warning: {failed} lookups failed; some playlists may be missing.");
    }
    Ok(())
}
