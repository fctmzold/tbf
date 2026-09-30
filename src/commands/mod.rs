use anyhow::Result;
use reqwest::Client;

use crate::cli::{Cli, Commands};

pub mod bruteforce;
pub mod clipforce;
pub mod exact;
pub mod fix;
pub mod link;
pub mod live;
pub mod vods;

/// Dispatch a parsed command to its handler.
///
/// Shared by the CLI and interactive entry points.
///
/// # Arguments
///
/// * `command` - Command to run.
/// * `client` - Shared HTTP client.
/// * `flags` - Global CLI flags.
///
/// # Errors
///
/// Returns whatever the selected command returns.
pub async fn execute_command(command: Commands, client: &Client, flags: &Cli) -> Result<()> {
    match command {
        Commands::Exact {
            username,
            id,
            stamp,
        } => exact::execute(client, &username, id, stamp, flags).await,
        Commands::Bruteforce {
            username,
            id,
            from,
            to,
            all,
            yes,
        } => {
            let target = bruteforce::BruteforceTarget {
                username: &username,
                id,
                from,
                to,
                all,
                yes,
            };
            bruteforce::execute(client, target, flags).await
        }
        Commands::Clipforce { id, start, end } => {
            clipforce::execute(client, id, start, end, flags).await
        }
        Commands::Link { url } => link::execute(client, &url, flags).await,
        Commands::Live { username } => live::execute(client, &username, flags).await,
        Commands::Vods {
            username,
            video_type,
        } => vods::execute(client, &username, &video_type, flags).await,
        Commands::Fix { url, output, force } => {
            fix::execute(client, &url, output, force, flags).await
        }
    }
}
