use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::twitch::retry::{Failure, check_http_status, classify_request, with_retry};

/// Shared Twitch GraphQL endpoint.
pub const GQL_ENDPOINT: &str = "https://gql.twitch.tv/gql";
/// Public client ID used by third-party Twitch tools.
pub const GQL_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

#[derive(Serialize)]
struct GqlStreamInfoQuery {
    query: String,
    variables: GqlStreamVars,
}

#[derive(Serialize)]
struct GqlStreamVars {
    login: String,
}

#[derive(Deserialize, Debug)]
struct GqlStreamInfoResponse {
    data: Option<GqlStreamData>,
}

#[derive(Deserialize, Debug)]
struct GqlStreamData {
    user: Option<GqlUser>,
}

#[derive(Deserialize, Debug)]
struct GqlUser {
    stream: Option<GqlStream>,
}

#[derive(Deserialize, Debug)]
struct GqlStream {
    id: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

/// Extract the broadcast ID and start timestamp from a stream info response.
fn extract_stream_info(response: GqlStreamInfoResponse) -> Result<Option<(u64, String)>> {
    if let Some(data) = response.data {
        if let Some(user) = data.user {
            if let Some(stream) = user.stream {
                let id = stream
                    .id
                    .parse::<u64>()
                    .context("Invalid stream ID format")?;
                return Ok(Some((id, stream.created_at)));
            }
        }
    }
    Ok(None)
}

#[derive(Deserialize, Debug)]
struct GqlVodTokenResponse {
    data: Option<GqlVodTokenData>,
}

#[derive(Deserialize, Debug)]
struct GqlVodTokenData {
    #[serde(rename = "videoPlaybackAccessToken")]
    video_playback_access_token: Option<GqlVodAccessToken>,
}

#[derive(Deserialize, Debug)]
struct GqlVodAccessToken {
    value: String,
    signature: String,
}

/// Fetch the VOD playback token and signature for one video.
///
/// The token authorizes the Usher VOD manifest, which lists every quality
/// variant. No hash guessing is involved.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `vod_id` - Twitch video ID.
///
/// # Returns
///
/// `Some((token, signature))` when Twitch issues a token, `None` for
/// restricted videos.
///
/// # Errors
///
/// Returns an error when the GQL request fails or the response cannot be
/// parsed.
pub async fn get_vod_token(client: &Client, vod_id: &str) -> Result<Option<(String, String)>> {
    const QUERY: &str = "query($vodID: ID!) { videoPlaybackAccessToken(id: $vodID, params: {platform: \"web\", playerBackend: \"mediaplayer\", playerType: \"embed\"}) { value signature } }";

    let payload = serde_json::json!({
        "query": QUERY,
        "variables": { "vodID": vod_id },
    });

    let parsed: GqlVodTokenResponse = with_retry(
        || async {
            let response = client
                .post(GQL_ENDPOINT)
                .header("Client-ID", GQL_CLIENT_ID)
                .json(&payload)
                .send()
                .await
                .map_err(classify_request)?;
            check_http_status(response.status())?;
            response.json().await.map_err(|error| {
                (
                    Failure::Permanent,
                    anyhow::anyhow!("Failed to parse GQL response: {error}"),
                )
            })
        },
        3,
    )
    .await
    .context("VOD token request failed")?;

    Ok(extract_vod_token(parsed))
}

/// Extract the token pair from a VOD token response.
fn extract_vod_token(response: GqlVodTokenResponse) -> Option<(String, String)> {
    response
        .data?
        .video_playback_access_token
        .map(|token| (token.value, token.signature))
}

/// Fetch the broadcast ID and start timestamp for a currently live stream.
///
/// The metadata feeds the `exact` lookup, which reconstructs the hidden
/// `index-dvr.m3u8` URL holding the full seekable timeline.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
///
/// # Returns
///
/// `Some((broadcast_id, created_at))` when live, `None` otherwise.
///
/// # Errors
///
/// Returns an error when the GQL request fails, the response cannot be
/// parsed, or the stream ID is not numeric.
pub async fn get_stream_info(client: &Client, username: &str) -> Result<Option<(u64, String)>> {
    const QUERY: &str =
        "query($login: String!) { user(login: $login) { stream { id createdAt } } }";

    let payload = GqlStreamInfoQuery {
        query: QUERY.to_string(),
        variables: GqlStreamVars {
            login: username.to_string(),
        },
    };

    let parsed: GqlStreamInfoResponse = with_retry(
        || async {
            let response = client
                .post(GQL_ENDPOINT)
                .header("Client-ID", GQL_CLIENT_ID)
                .json(&payload)
                .send()
                .await
                .map_err(classify_request)?;
            check_http_status(response.status())?;
            response.json().await.map_err(|error| {
                (
                    Failure::Permanent,
                    anyhow::anyhow!("Failed to parse GQL response: {error}"),
                )
            })
        },
        3,
    )
    .await
    .context("Stream info request failed")?;

    extract_stream_info(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fixture(json: &str) -> GqlStreamInfoResponse {
        serde_json::from_str(json).expect("fixture parses")
    }

    #[test]
    fn extracts_live_stream_info() {
        let response = parse_fixture(
            r#"{"data":{"user":{"stream":{"id":"39700667438","createdAt":"2020-11-19T04:29:54Z"}}}}"#,
        );
        let info = extract_stream_info(response).expect("extraction succeeds");
        assert_eq!(
            info,
            Some((39_700_667_438, "2020-11-19T04:29:54Z".to_string()))
        );
    }

    #[test]
    fn offline_stream_yields_none() {
        let response = parse_fixture(r#"{"data":{"user":{"stream":null}}}"#);
        let info = extract_stream_info(response).expect("extraction succeeds");
        assert!(info.is_none());
    }

    #[test]
    fn unknown_user_yields_none() {
        let response = parse_fixture(r#"{"data":{"user":null}}"#);
        let info = extract_stream_info(response).expect("extraction succeeds");
        assert!(info.is_none());
    }

    #[test]
    fn invalid_id_returns_error() {
        let response = parse_fixture(
            r#"{"data":{"user":{"stream":{"id":"not-a-number","createdAt":"2020-11-19T04:29:54Z"}}}}"#,
        );
        assert!(extract_stream_info(response).is_err());
    }

    #[test]
    fn extracts_vod_token() {
        let response: GqlVodTokenResponse = serde_json::from_str(
            r#"{"data":{"videoPlaybackAccessToken":{"value":"{\"vod_id\":1}","signature":"sig"}}}"#,
        )
        .expect("fixture parses");
        assert_eq!(
            extract_vod_token(response),
            Some(("{\"vod_id\":1}".to_string(), "sig".to_string()))
        );
    }

    #[test]
    fn missing_vod_token_yields_none() {
        let response: GqlVodTokenResponse =
            serde_json::from_str(r#"{"data":{"videoPlaybackAccessToken":null}}"#)
                .expect("fixture parses");
        assert!(extract_vod_token(response).is_none());
    }
}
