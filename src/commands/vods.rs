use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::Cli;
use crate::progress::{emit, scanning_progress};
use crate::twitch::retry::{check_http_status, classify_request, with_retry};
use crate::twitch::usher::Variant;
use crate::twitch::videos::Video;
use crate::twitch::{gql, usher, videos};

/// API fan-out cap; token and manifest calls stay polite.
const API_CONCURRENCY: usize = 8;

/// Resolve one video to its playable variant playlists.
///
/// A 403 means the video is restricted and yields `None`; other failures
/// are errors so rate limits never look like missing VODs.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `video` - Listed channel video.
///
/// # Returns
///
/// Variant playlists, or `None` when unavailable or restricted.
///
/// # Errors
///
/// Returns an error when the token or manifest request fails at the
/// transport level.
async fn resolve_video(client: &Client, video: &Video) -> Result<Option<Vec<Variant>>> {
    let Some((token, signature)) = gql::get_vod_token(client, &video.id).await? else {
        return Ok(None);
    };
    let manifest = usher::vod_manifest_url(&video.id, &signature, &token)?;
    let response = with_retry(
        || async {
            let response = client
                .get(&manifest)
                .send()
                .await
                .map_err(classify_request)?;
            if response.status() == reqwest::StatusCode::FORBIDDEN {
                return Ok(None);
            }
            check_http_status(response.status())?;
            Ok(Some(response))
        },
        3,
    )
    .await
    .context("VOD manifest request failed")?;
    let Some(response) = response else {
        return Ok(None);
    };
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
/// manifest. Results keep newest-first order.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Channel login name.
/// * `video_type` - `all`, `archive`, `highlight`, or `upload`.
/// * `flags` - Global CLI flags controlling progress output.
///
/// # Errors
///
/// Returns an error when the video listing cannot be fetched.
pub async fn execute(client: &Client, username: &str, video_type: &str, flags: &Cli) -> Result<()> {
    let username = username.to_lowercase();
    if !flags.simple {
        println!("Fetching VOD list for '{username}'...");
    }

    let all = videos::fetch_all_videos(client, &username, videos::broadcast_filter(video_type))
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

    let resolved: Vec<(Video, Result<Option<Vec<Variant>>>)> = stream::iter(all)
        .map(|video| {
            let client = client.clone();
            let progress = progress.clone();
            async move {
                let result = resolve_video(&client, &video).await;
                if let Some(bar) = &progress {
                    bar.inc(1);
                }
                (video, result)
            }
        })
        .buffered(API_CONCURRENCY)
        .collect()
        .await;

    // Printing happens here, in listing order, so concurrent resolutions
    // above cannot interleave lines.
    let mut with_playlist = 0_u64;
    let mut failed = 0_u64;
    for (video, result) in resolved {
        let mut message = format!(
            "{}\n  Link: {}\n  Duration: {}   Views: {}   Game: {}",
            video.summary(),
            video.url(),
            videos::format_duration(video.duration_seconds),
            video
                .view_count
                .map(|views| views.to_string())
                .unwrap_or_else(|| "-".to_string()),
            video.game_name.as_deref().unwrap_or("-")
        );
        match result {
            Ok(Some(variants)) => {
                for variant in &variants {
                    message.push_str(&format!("\n  [{}] {}", variant.label, variant.url));
                }
                with_playlist += 1;
            }
            Ok(None) => message.push_str("\n  No playable playlist found."),
            Err(error) => {
                message.push_str(&format!("\n  Lookup failed: {error:#}"));
                failed += 1;
            }
        }
        emit(progress.as_ref(), message);
    }

    if let Some(bar) = progress {
        bar.finish_with_message("Scan complete");
    }
    println!("{with_playlist} videos with playable playlists.");
    if failed > 0 {
        println!("Warning: {failed} lookups failed; some playlists may be missing.");
    }
    Ok(())
}
