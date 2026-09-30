use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use reqwest::Client;
use tbf_new::cli::Cli;
use tbf_new::{commands::execute_command, interactive, report::print_error};

/// Binary entry point with clean error reporting.
#[tokio::main]
async fn main() {
    if let Err(report) = run().await {
        print_error(&report);
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

    if let Some(command) = args.command.take() {
        return execute_command(command, &client, &args).await;
    }
    interactive::run(&client, &args).await
}
