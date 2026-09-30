use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use reqwest::Client;
use tbf_new::cli::{Cli, Commands};
use tbf_new::{commands, tui};
use tracing::error;
use tracing_subscriber::EnvFilter;

/// Binary entry point with tracing-based error reporting.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(report) = run().await {
        error!("{report:?}");
        std::process::exit(1);
    }
}

/// Parse CLI input and dispatch to the selected command.
async fn run() -> Result<()> {
    let mut args = Cli::parse();
    let client = Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .context("Failed to build HTTP client")?;

    let command = if let Some(command) = args.command.take() {
        command
    } else {
        let selected = tui::run().context("TUI failed to initialize")?;
        match selected.as_deref() {
            Some("exact") => prompt_exact()?,
            Some("bruteforce") => prompt_bruteforce()?,
            Some("clipforce") => prompt_clipforce()?,
            Some("link") => prompt_link()?,
            Some("live") => prompt_live()?,
            Some("vods") => prompt_vods()?,
            Some("fix") => prompt_fix()?,
            _ => return Ok(()),
        }
    };

    execute_command(command, &client, &args).await
}

/// Dispatch a parsed command to its handler.
async fn execute_command(command: Commands, client: &Client, flags: &Cli) -> Result<()> {
    match command {
        Commands::Exact {
            username,
            id,
            stamp,
        } => commands::exact::execute(client, &username, id, &stamp, flags).await,
        Commands::Bruteforce {
            username,
            id,
            from,
            to,
        } => {
            let target = commands::bruteforce::BruteforceTarget {
                username: &username,
                id,
                from: &from,
                to: &to,
            };
            commands::bruteforce::execute(client, target, flags).await
        }
        Commands::Clipforce { id, start, end } => {
            commands::clipforce::execute(client, id, start, end, flags).await
        }
        Commands::Link { url } => commands::link::execute(client, &url, flags).await,
        Commands::Live { username } => commands::live::execute(client, &username, flags).await,
        Commands::Vods {
            username,
            video_type,
        } => commands::vods::execute(client, &username, &video_type, flags).await,
        Commands::Fix { url, output, force } => {
            commands::fix::execute(client, &url, output, force, flags).await
        }
    }
}

/// Read a trimmed line from stdin after showing a message.
fn prompt(message: &str) -> String {
    println!("{message}");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or_default();
    input.trim().to_string()
}

/// Collect inputs for the exact command.
fn prompt_exact() -> Result<Commands> {
    let username = prompt("Enter streamer's username:");
    let id_input = prompt("Enter VOD/broadcast ID:");
    let stamp = prompt("Enter timestamp (Unix epoch or RFC3339):");
    Ok(Commands::Exact {
        username,
        id: id_input.parse().context("Invalid ID format")?,
        stamp,
    })
}

/// Collect inputs for the bruteforce command.
fn prompt_bruteforce() -> Result<Commands> {
    let username = prompt("Enter streamer's username:");
    let id_input = prompt("Enter VOD/broadcast ID:");
    let from = prompt("Enter start timestamp:");
    let to = prompt("Enter end timestamp:");
    Ok(Commands::Bruteforce {
        username,
        id: id_input.parse().context("Invalid ID format")?,
        from,
        to,
    })
}

/// Collect inputs for the clipforce command.
fn prompt_clipforce() -> Result<Commands> {
    let id_input = prompt("Enter VOD/broadcast ID:");
    let start_input = prompt("Enter start offset in seconds:");
    let end_input = prompt("Enter end offset in seconds:");
    Ok(Commands::Clipforce {
        id: id_input.parse().context("Invalid ID format")?,
        start: start_input.parse().context("Invalid start offset")?,
        end: end_input.parse().context("Invalid end offset")?,
    })
}

/// Collect inputs for the link command.
fn prompt_link() -> Result<Commands> {
    let url = prompt("Enter StreamsCharts stream page URL:");
    Ok(Commands::Link { url })
}

/// Collect inputs for the live command.
fn prompt_live() -> Result<Commands> {
    let username = prompt("Enter streamer's username:");
    Ok(Commands::Live { username })
}

/// Collect inputs for the vods command.
fn prompt_vods() -> Result<Commands> {
    let username = prompt("Enter channel name:");
    Ok(Commands::Vods {
        username,
        video_type: "all".to_string(),
    })
}

/// Collect inputs for the fix command.
fn prompt_fix() -> Result<Commands> {
    let url = prompt("Enter Twitch VOD m3u8 playlist URL:");
    let output = prompt("Enter output file path (leave blank for default):");
    let output_option = if output.is_empty() {
        None
    } else {
        Some(output)
    };
    Ok(Commands::Fix {
        url,
        output: output_option,
        force: false,
    })
}
