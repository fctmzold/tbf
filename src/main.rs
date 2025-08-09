mod config;
mod error;
mod interface;
mod twitch;
mod update;
mod util;

use anyhow::Result;
use clap::{crate_name, crate_version, Parser};
use crossterm::{execute, terminal::SetTitle};
use env_logger::Env;
use lazy_static::lazy_static;
use log::{debug, error};
use std::{io::stdout, panic};

use config::Cli;
use interface::main_interface;

lazy_static! {
    // HTTP client to share
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
}

#[tokio::main]
async fn main() {
    if let Err(e) = execute().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn execute() -> Result<()> {
    execute!(
        stdout(),
        SetTitle(format!("{} v{}", crate_name!(), crate_version!()))
    )?;

    let matches = Cli::parse();

    let log_level = if matches.verbose { "debug" } else { "info" };

    env_logger::Builder::from_env(Env::default().filter_or(
        env_logger::DEFAULT_FILTER_ENV,
        format!("{log_level},html5ever=info,selectors=info"),
    ))
    .format_timestamp_millis()
    .init();

    // making panics look nicer
    panic::set_hook(Box::new(move |panic_info| {
        debug!("{panic_info}");
        if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            error!("{s}");
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            error!("{s}");
        } else {
            error!("{panic_info}");
        }
    }));

    match matches.command {
        Some(ref sub) => {
            if let Err(e) = sub.execute(matches.clone()).await {
                error!("{e}");
            }
        }
        None => main_interface(matches).await,
    }
    
    Ok(())
}
