use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use reqwest::Client;
use tbf_new::cli::Cli;
use tbf_new::commands::{Outcome, execute_command};
use tbf_new::{interactive, report::print_error};

/// Binary entry point with grep-style exit codes.
///
/// `0` for hits, `1` for clean misses, `2` for errors.
#[tokio::main]
async fn main() {
    match run().await {
        Ok(Outcome::Found) => {}
        Ok(Outcome::NotFound) => std::process::exit(1),
        Err(report) => {
            print_error(&report);
            std::process::exit(2);
        }
    }
}

/// Parse CLI input and dispatch to the selected command.
async fn run() -> Result<Outcome> {
    let args = Cli::parse();
    let Cli { command, opts } = args;
    let client = Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(usize::from(opts.threads).min(64))
        .build()
        .context("Failed to build HTTP client")?;

    match command {
        Some(command) => execute_command(command, &client, &opts).await,
        None => {
            interactive::run(&client, &opts).await?;
            Ok(Outcome::Found)
        }
    }
}
