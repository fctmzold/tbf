use anyhow::{Context, Result};
use reqwest::Client;

use crate::cli::GlobalOpts;
use crate::commands::{Outcome, exact};
use crate::report::hint;
use crate::twitch::gql;
use crate::util::parse_timestamp;

/// Resolve a live stream to its hidden DVR playlist via an exact lookup.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `opts` - Global CLI options forwarded to the exact lookup.
///
/// # Errors
///
/// Returns an error when the metadata lookup or the DVR search fails.
pub async fn execute(client: &Client, username: &str, opts: &GlobalOpts) -> Result<Outcome> {
    let username = username.to_lowercase();
    eprintln!("Fetching live stream metadata for '{username}'...");

    match gql::get_stream_info(client, &username).await? {
        Some((broadcast_id, created_at)) => {
            eprintln!("Stream is LIVE!");
            eprintln!("Broadcast ID (VOD ID): {broadcast_id}");
            eprintln!("Stream Started At: {created_at}");
            eprintln!("Searching for the hidden DVR/VOD playlist...");
            let timestamp = parse_timestamp(&created_at).context("Failed to parse stream start")?;
            exact::execute(client, &username, broadcast_id, timestamp, opts).await
        }
        None => {
            eprintln!("User '{username}' is not currently live or the channel does not exist.");
            hint("list past broadcasts with `vods <username>`");
            Ok(Outcome::NotFound)
        }
    }
}
