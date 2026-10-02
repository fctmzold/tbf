use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::cli::{GlobalOpts, VideoType};
use crate::commands::Outcome;
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

/// Render one video with its variants as a JSON line.
///
/// # Arguments
///
/// * `video` - Listed channel video.
/// * `variants` - Resolved playable variants, possibly empty.
fn video_json(video: &Video, variants: &[Variant]) -> serde_json::Value {
    serde_json::json!({
        "id": video.id,
        "title": video.title,
        "published_at": video.published_at,
        "duration_seconds": video.duration_seconds,
        "view_count": video.view_count,
        "game": video.game_name,
        "url": video.url(),
        "variants": variants.iter().map(|variant| serde_json::json!({
            "label": variant.label,
            "url": variant.url,
        })).collect::<Vec<_>>(),
    })
}

/// List a channel's VODs with playable `index-dvr.m3u8` links.
///
/// Each video resolves through its VOD playback token to the Usher master
/// manifest. Results stream out in listing order as they resolve.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Channel login name.
/// * `video_type` - Kind of videos to list.
/// * `opts` - Global CLI options controlling progress output.
///
/// # Errors
///
/// Returns an error when the video listing cannot be fetched.
pub async fn execute(
    client: &Client,
    username: &str,
    video_type: VideoType,
    opts: &GlobalOpts,
) -> Result<Outcome> {
    let username = username.to_lowercase();
    eprintln!("Fetching VOD list for '{username}'...");

    let all = videos::fetch_all_videos(client, &username, video_type.api_value())
        .await
        .context("Failed to fetch channel videos")?;
    if all.is_empty() {
        eprintln!("No videos found for '{username}'.");
        return Ok(Outcome::NotFound);
    }
    eprintln!("Found {} videos.", all.len());
    let total = all.len() as u64;

    // Piped or minimal output carries only variant URLs; the rich blocks
    // below stay on interactive terminals.
    let urls_only = opts.simple || !std::io::IsTerminal::is_terminal(&std::io::stdout());

    let progress = scanning_progress(
        all.len() as u64,
        "Resolving VOD playlists...",
        opts.simple,
        opts.progressbar,
    );

    // `buffered` preserves listing order, so each result prints as it
    // arrives instead of after every lookup finishes.
    let mut results = stream::iter(all)
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
        .buffered(API_CONCURRENCY);

    let mut with_playlist = 0_u64;
    let mut failed = 0_u64;
    while let Some((video, result)) = results.next().await {
        match result {
            Ok(Some(variants)) => {
                with_playlist += 1;
                if opts.json {
                    crate::report::stdout_line(
                        progress.as_ref(),
                        &video_json(&video, &variants).to_string(),
                    );
                    continue;
                }
                if urls_only {
                    for variant in &variants {
                        crate::report::stdout_line(progress.as_ref(), &variant.url);
                    }
                    continue;
                }
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
                for variant in &variants {
                    message.push_str(&format!("\n  [{}] {}", variant.label, variant.url));
                }
                emit(progress.as_ref(), message);
            }
            Ok(None) => {
                if opts.json {
                    crate::report::stdout_line(
                        progress.as_ref(),
                        &video_json(&video, &[]).to_string(),
                    );
                } else if !urls_only {
                    emit(
                        progress.as_ref(),
                        format!(
                            "{}\n  Link: {}\n  No playable playlist found.",
                            video.summary(),
                            video.url()
                        ),
                    );
                }
            }
            Err(error) => {
                failed += 1;
                if opts.json {
                    crate::report::stdout_line(
                        progress.as_ref(),
                        &serde_json::json!({"id": video.id, "error": format!("{error:#}")})
                            .to_string(),
                    );
                } else if !urls_only {
                    emit(
                        progress.as_ref(),
                        format!("{}\n  Lookup failed: {error:#}", video.summary()),
                    );
                }
            }
        }
    }
    drop(results);

    if let Some(bar) = progress {
        bar.finish_with_message("Scan complete");
    }
    eprintln!("{with_playlist} videos with playable playlists.");
    if failed > 0 {
        eprintln!("Warning: {failed} lookups failed; some playlists may be missing.");
    }
    if with_playlist == 0 {
        if failed == total {
            anyhow::bail!("All {total} lookups failed; check your connection and try again.");
        }
        return Ok(Outcome::NotFound);
    }
    Ok(Outcome::Found)
}
