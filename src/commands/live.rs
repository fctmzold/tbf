use anyhow::Result;
use reqwest::Client;

use crate::cli::Cli;
use crate::commands::exact;
use crate::twitch::gql;

/// Resolve a live stream to its hidden DVR playlist via an exact lookup.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `username` - Streamer login name.
/// * `flags` - Global CLI flags forwarded to the exact lookup.
///
/// # Errors
///
/// Returns an error when the metadata lookup or the DVR search fails.
pub async fn execute(client: &Client, username: &str, flags: &Cli) -> Result<()> {
    let username = username.to_lowercase();
    println!("Fetching live stream metadata for '{username}'...");

    match gql::get_stream_info(client, &username).await? {
        Some((broadcast_id, created_at)) => {
            println!("Stream is LIVE!");
            println!("Broadcast ID (VOD ID): {broadcast_id}");
            println!("Stream Started At: {created_at}");
            println!("Searching for the hidden DVR/VOD playlist...");
            exact::execute(client, &username, broadcast_id, &created_at, flags).await
        }
        None => {
            println!("User '{username}' is not currently live or the channel does not exist.");
            Ok(())
        }
    }
}
