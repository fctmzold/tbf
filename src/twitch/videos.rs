use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Twitch persisted-query hash for channel video listings.
///
/// Twitch rotates these periodically; a sudden empty list for a channel
/// with VODs usually means this hash needs updating.
const VIDEO_TOWER_HASH: &str = "67004f7881e65c297936f32c75246470629557a393788fb5a69d6d9a25a8fd5f";
const VIDEO_TOWER_OPERATION: &str = "FilterableVideoTower_Videos";
const GQL_ENDPOINT: &str = "https://gql.twitch.tv/gql";
const GQL_CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";
const PAGE_SIZE: u32 = 100;

/// One channel video (VOD, highlight, or upload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    /// Twitch video ID.
    pub id: String,
    /// Stream title.
    pub title: Option<String>,
    /// Publish timestamp (RFC3339).
    pub published_at: Option<String>,
    /// Length in seconds.
    pub duration_seconds: Option<i64>,
    /// View count.
    pub view_count: Option<i64>,
    /// Game display name.
    pub game_name: Option<String>,
}

impl Video {
    /// Watch URL for the video.
    ///
    /// # Returns
    ///
    /// `https://www.twitch.tv/videos/<id>`.
    pub fn url(&self) -> String {
        format!("https://www.twitch.tv/videos/{}", self.id)
    }

    /// One-line summary for list output.
    ///
    /// # Returns
    ///
    /// `<published_at> | <title> (<id>)`.
    pub fn summary(&self) -> String {
        format!(
            "{} | {} ({})",
            self.published_at.as_deref().unwrap_or("unknown date"),
            self.title.as_deref().unwrap_or("(untitled)"),
            self.id
        )
    }
}

/// One fetched page of channel videos.
#[derive(Debug)]
pub struct VideosPage {
    /// Videos on this page, newest first.
    pub videos: Vec<Video>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether Twitch reports another page.
    pub has_next: bool,
}

#[derive(Serialize)]
struct PersistedQuery {
    version: u8,
    #[serde(rename = "sha256Hash")]
    sha256_hash: &'static str,
}

#[derive(Serialize)]
struct TowerExtensions {
    #[serde(rename = "persistedQuery")]
    persisted_query: PersistedQuery,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TowerVariables {
    channel_owner_login: String,
    broadcast_type: Option<String>,
    video_sort: &'static str,
    limit: u32,
    cursor: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TowerOperation {
    operation_name: &'static str,
    variables: TowerVariables,
    extensions: TowerExtensions,
}

#[derive(Deserialize, Debug)]
struct TowerResult {
    errors: Option<serde_json::Value>,
    data: Option<TowerData>,
}

#[derive(Deserialize, Debug)]
struct TowerData {
    user: Option<TowerUser>,
}

#[derive(Deserialize, Debug)]
struct TowerUser {
    videos: Option<VideosConnection>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct VideosConnection {
    edges: Option<Vec<VideoEdge>>,
    page_info: Option<PageInfo>,
}

#[derive(Deserialize, Debug)]
struct VideoEdge {
    cursor: Option<String>,
    node: serde_json::Value,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: Option<bool>,
}

/// Extract a `Video` from a raw node, tolerating string or numeric IDs.
fn extract_video(node: &serde_json::Value) -> Option<Video> {
    let id = match node.get("id") {
        Some(serde_json::Value::String(id)) => id.clone(),
        Some(serde_json::Value::Number(id)) => id.to_string(),
        _ => return None,
    };
    let game = node.get("game");
    Some(Video {
        id,
        title: node
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        published_at: node
            .get("publishedAt")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        duration_seconds: node.get("lengthSeconds").and_then(|value| value.as_i64()),
        view_count: node.get("viewCount").and_then(|value| value.as_i64()),
        game_name: game
            .and_then(|game| game.get("displayName"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
}

/// Parse one persisted-query response body into a page.
///
/// # Arguments
///
/// * `body` - Raw JSON response text.
/// * `channel` - Channel login, used in error messages.
///
/// # Returns
///
/// Videos plus pagination state.
///
/// # Errors
///
/// Returns an error for malformed responses, GraphQL errors, or unknown
/// channels.
fn parse_videos_page(body: &str, channel: &str) -> Result<VideosPage> {
    let payload: Vec<TowerResult> =
        serde_json::from_str(body).context("Unexpected video listing response")?;
    let result = payload
        .into_iter()
        .next()
        .context("Empty video listing response")?;
    if let Some(errors) = result.errors {
        if !errors.is_null() {
            anyhow::bail!("Video listing query failed: {errors}");
        }
    }
    let connection = result
        .data
        .and_then(|data| data.user)
        .ok_or_else(|| anyhow::anyhow!("Channel '{channel}' does not exist on Twitch"))?
        .videos
        .unwrap_or(VideosConnection {
            edges: None,
            page_info: None,
        });

    let edges = connection.edges.unwrap_or_default();
    let videos: Vec<Video> = edges
        .iter()
        .filter_map(|edge| extract_video(&edge.node))
        .collect();
    let next_cursor = edges.iter().rev().find_map(|edge| edge.cursor.clone());
    let has_next = connection
        .page_info
        .and_then(|info| info.has_next_page)
        .unwrap_or(false)
        && next_cursor.is_some();

    Ok(VideosPage {
        videos,
        next_cursor,
        has_next,
    })
}

/// Fetch one page of a channel's videos (newest first).
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `channel` - Channel login name.
/// * `cursor` - Pagination cursor, empty for the first page.
///
/// # Returns
///
/// Videos plus pagination state for the next call.
///
/// # Errors
///
/// Returns an error when the request fails, the response cannot be parsed,
/// or the channel does not exist.
pub async fn fetch_videos_page(client: &Client, channel: &str, cursor: &str) -> Result<VideosPage> {
    let operations = [TowerOperation {
        operation_name: VIDEO_TOWER_OPERATION,
        variables: TowerVariables {
            channel_owner_login: channel.to_string(),
            broadcast_type: None,
            video_sort: "TIME",
            limit: PAGE_SIZE,
            cursor: cursor.to_string(),
        },
        extensions: TowerExtensions {
            persisted_query: PersistedQuery {
                version: 1,
                sha256_hash: VIDEO_TOWER_HASH,
            },
        },
    }];
    let body =
        serde_json::to_string(&operations).context("Failed to encode video listing query")?;

    let response = client
        .post(GQL_ENDPOINT)
        .header("Client-ID", GQL_CLIENT_ID)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .body(body)
        .send()
        .await
        .context("Failed to query Twitch video listing")?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Video listing fetch failed with status {}",
            response.status()
        );
    }
    let text = response
        .text()
        .await
        .context("Failed to read video listing body")?;
    parse_videos_page(&text, channel)
}

/// Fetch all of a channel's videos, newest first.
///
/// Pages through the whole listing with retries per page.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `channel` - Channel login name.
///
/// # Returns
///
/// Every video reported by Twitch.
///
/// # Errors
///
/// Returns an error when a page repeatedly fails to fetch or parse.
pub async fn fetch_all_videos(client: &Client, channel: &str) -> Result<Vec<Video>> {
    let mut cursor = String::new();
    let mut videos = Vec::new();
    loop {
        let mut fetched = None;
        let mut last_error = String::new();
        for _ in 0..3 {
            match fetch_videos_page(client, channel, &cursor).await {
                Ok(page) => {
                    fetched = Some(page);
                    break;
                }
                Err(error) => {
                    last_error = format!("{error:#}");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
        match fetched {
            Some(page) => {
                videos.extend(page.videos);
                match page.next_cursor {
                    Some(next) if page.has_next => cursor = next,
                    _ => break,
                }
            }
            None => anyhow::bail!("Failed to fetch videos for '{channel}': {last_error}"),
        }
    }
    Ok(videos)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_FIXTURE: &str = r#"[{
        "data": {"user": {"videos": {
            "edges": [
                {"cursor": "cursor-1", "node": {
                    "id": "316969565142", "title": "Ranked grind",
                    "publishedAt": "2026-09-29T18:00:00Z",
                    "lengthSeconds": 7265, "viewCount": 1204,
                    "game": {"displayName": "Just Chatting"}}},
                {"cursor": "cursor-2", "node": {
                    "id": 316000000001, "title": null,
                    "publishedAt": "2026-09-28T18:00:00Z",
                    "lengthSeconds": 3600, "viewCount": 42,
                    "game": null}}
            ],
            "pageInfo": {"hasNextPage": true}
        }}}
    }]"#;

    #[test]
    fn parses_page_with_pagination() {
        let page = parse_videos_page(PAGE_FIXTURE, "arquel").expect("fixture parses");
        assert_eq!(page.videos.len(), 2);
        assert_eq!(page.videos[0].id, "316969565142");
        assert_eq!(page.videos[0].title.as_deref(), Some("Ranked grind"));
        assert_eq!(page.videos[0].duration_seconds, Some(7265));
        assert_eq!(page.videos[0].view_count, Some(1204));
        assert_eq!(page.videos[0].game_name.as_deref(), Some("Just Chatting"));
        assert_eq!(page.videos[1].id, "316000000001");
        assert!(page.has_next);
        assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));
    }

    #[test]
    fn watch_url_uses_video_id() {
        let page = parse_videos_page(PAGE_FIXTURE, "arquel").expect("fixture parses");
        assert_eq!(
            page.videos[0].url(),
            "https://www.twitch.tv/videos/316969565142"
        );
    }

    #[test]
    fn summary_shows_date_title_and_id() {
        let page = parse_videos_page(PAGE_FIXTURE, "arquel").expect("fixture parses");
        assert_eq!(
            page.videos[0].summary(),
            "2026-09-29T18:00:00Z | Ranked grind (316969565142)"
        );
    }

    #[test]
    fn unknown_channel_is_error() {
        let body = r#"[{"data": {"user": null}}]"#;
        let error = parse_videos_page(body, "nosuchchannel").expect_err("unknown channel fails");
        assert!(error.to_string().contains("nosuchchannel"));
    }

    #[test]
    fn graphql_errors_are_reported() {
        let body = r#"[{"errors": [{"message": "bad hash"}], "data": null}]"#;
        assert!(parse_videos_page(body, "arquel").is_err());
    }

    #[test]
    fn empty_edges_end_pagination() {
        let body = r#"[{"data": {"user": {"videos": {"edges": [], "pageInfo": {"hasNextPage": false}}}}}]"#;
        let page = parse_videos_page(body, "arquel").expect("empty page parses");
        assert!(page.videos.is_empty());
        assert!(!page.has_next);
        assert!(page.next_cursor.is_none());
    }
}
