use anyhow::Result;
use reqwest::Client;

use crate::cli::{Commands, GlobalOpts};

pub mod bruteforce;
pub mod clipforce;
pub mod exact;
pub mod fix;
pub mod link;
pub mod live;
pub mod vods;

/// Whether a command found what it was looking for.
///
/// Drives the process exit code: `Found` exits 0, `NotFound` exits 1
/// (grep-style), errors exit 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The command found at least one result.
    Found,
    /// The command ran fine but found nothing.
    NotFound,
}

/// Dispatch a parsed command to its handler.
///
/// Shared by the CLI and interactive entry points.
///
/// # Arguments
///
/// * `command` - Command to run.
/// * `client` - Shared HTTP client.
/// * `opts` - Global CLI options.
///
/// # Errors
///
/// Returns whatever the selected command returns.
pub async fn execute_command(
    command: Commands,
    client: &Client,
    opts: &GlobalOpts,
) -> Result<Outcome> {
    match command {
        Commands::Exact {
            username,
            id,
            stamp,
        } => exact::execute(client, &username, id, stamp, opts).await,
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
            bruteforce::execute(client, target, opts).await
        }
        Commands::Clipforce {
            id,
            start,
            end,
            yes,
        } => {
            let target = clipforce::ClipforceTarget {
                id,
                start,
                end,
                yes,
            };
            clipforce::execute(client, target, opts).await
        }
        Commands::Link { url } => link::execute(client, &url, opts).await,
        Commands::Live { username } => live::execute(client, &username, opts).await,
        Commands::Vods {
            username,
            video_type,
        } => vods::execute(client, &username, video_type, opts).await,
        Commands::Fix { url, output, force } => {
            fix::execute(client, &url, output, force, opts).await
        }
    }
}
