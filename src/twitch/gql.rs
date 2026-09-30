use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

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
fn extract_stream_info(response: GqlStreamInfoResponse) -> Result<Option<(i64, String)>> {
    if let Some(data) = response.data {
        if let Some(user) = data.user {
            if let Some(stream) = user.stream {
                let id = stream
                    .id
                    .parse::<i64>()
                    .context("Invalid stream ID format")?;
                return Ok(Some((id, stream.created_at)));
            }
        }
    }
    Ok(None)
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
pub async fn get_stream_info(client: &Client, username: &str) -> Result<Option<(i64, String)>> {
    const ENDPOINT: &str = "https://gql.twitch.tv/gql";
    const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
    const QUERY: &str =
        "query($login: String!) { user(login: $login) { stream { id createdAt } } }";

    let payload = GqlStreamInfoQuery {
        query: QUERY.to_string(),
        variables: GqlStreamVars {
            login: username.to_string(),
        },
    };

    let response = client
        .post(ENDPOINT)
        .header("Client-ID", CLIENT_ID)
        .json(&payload)
        .send()
        .await
        .context("Failed to query Twitch GQL API for stream info")?;

    let parsed: GqlStreamInfoResponse = response
        .json()
        .await
        .context("Failed to parse GQL response")?;

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
}
