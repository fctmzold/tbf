use anyhow::Result;
use dialoguer::{Confirm, Input, Select};
use reqwest::Client;

use crate::cli::{Cli, Commands};
use crate::commands::execute_command;
use crate::commands::link::parse_tracker_target;
use crate::report::print_error;
use crate::util::{format_utc, parse_timestamp};

/// Guided menu entries with descriptions; Quit stays last.
const MENU: [(&str, &str); 8] = [
    ("exact", "Exact - check one timestamp for a VOD"),
    ("bruteforce", "Bruteforce - scan a timestamp range"),
    ("clipforce", "Clipforce - scan a VOD for clips"),
    ("link", "Link - look up a StreamsCharts stream page"),
    ("live", "Live - find the DVR playlist of a live stream"),
    ("vods", "Vods - list a channel's VODs with links"),
    ("fix", "Fix - repair an unmuted VOD playlist"),
    ("quit", "Quit"),
];

/// Video kinds offered by the guided `vods` flow.
const VIDEO_TYPES: [&str; 4] = ["all", "archive", "highlight", "upload"];

/// Strip `@` prefixes and `twitch.tv/<name>` links down to a login name.
///
/// # Arguments
///
/// * `raw` - Raw username field.
///
/// # Returns
///
/// Lowercase login name, possibly empty when nothing usable was entered.
fn normalize_username(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('@');
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    if let Ok(url) = url::Url::parse(&candidate) {
        let twitch = url
            .host_str()
            .is_some_and(|host| host == "twitch.tv" || host == "www.twitch.tv");
        if twitch {
            let segments: Vec<&str> = url
                .path_segments()
                .map(|parts| parts.filter(|part| !part.is_empty()).collect())
                .unwrap_or_default();
            if let [name] = segments.as_slice() {
                return name.to_lowercase();
            }
        }
    }
    trimmed.to_lowercase()
}

/// Ask for a non-empty username, accepting `@name` and twitch.tv links.
fn ask_username(prompt: &str) -> Result<String> {
    let raw: String = Input::new()
        .with_prompt(prompt)
        .validate_with(|input: &String| {
            if normalize_username(input).is_empty() {
                Err("enter a channel name".to_string())
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    Ok(normalize_username(&raw))
}

/// Ask for a VOD/broadcast ID.
fn ask_id(prompt: &str) -> Result<i64> {
    Ok(Input::new()
        .with_prompt(prompt)
        .validate_with(|input: &String| {
            input
                .trim()
                .parse::<i64>()
                .map(|_| ())
                .map_err(|_| "expected a numeric ID".to_string())
        })
        .interact_text()?
        .trim()
        .parse()?)
}

/// Ask for a timestamp, echoing the parsed UTC value back.
fn ask_timestamp(prompt: &str) -> Result<i64> {
    let raw: String = Input::new()
        .with_prompt(format!(
            "{prompt} [unix, RFC3339, or YYYY-MM-DD HH:MM[:SS]]"
        ))
        .validate_with(|input: &String| {
            parse_timestamp(input.trim())
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .interact_text()?;
    let timestamp = parse_timestamp(raw.trim())?;
    println!("  -> {}", format_utc(timestamp));
    Ok(timestamp)
}

/// Ask for a StreamsCharts stream page URL.
fn ask_tracker_url() -> Result<String> {
    Ok(Input::new()
        .with_prompt("StreamsCharts stream page URL")
        .validate_with(|input: &String| {
            parse_tracker_target(input.trim())
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .interact_text()?)
}

/// Build the guided command for one menu pick.
fn build_command(key: &str) -> Result<Option<Commands>> {
    match key {
        "exact" => Ok(Some(Commands::Exact {
            username: ask_username("Streamer username (or @name / twitch.tv link)")?,
            id: ask_id("VOD/broadcast ID")?,
            stamp: ask_timestamp("Timestamp")?,
        })),
        "bruteforce" => {
            let username = ask_username("Streamer username (or @name / twitch.tv link)")?;
            let id = ask_id("VOD/broadcast ID")?;
            let from = ask_timestamp("Range start")?;
            let to = ask_timestamp("Range end")?;
            let stop_first = Confirm::new()
                .with_prompt("Stop at the first hit?")
                .default(true)
                .interact()?;
            Ok(Some(Commands::Bruteforce {
                username,
                id,
                from,
                to,
                all: !stop_first,
                yes: true,
            }))
        }
        "clipforce" => Ok(Some(Commands::Clipforce {
            id: ask_id("VOD/broadcast ID")?,
            start: ask_offset("Start offset in seconds")?,
            end: ask_offset("End offset in seconds")?,
        })),
        "link" => Ok(Some(Commands::Link {
            url: ask_tracker_url()?,
        })),
        "live" => Ok(Some(Commands::Live {
            username: ask_username("Streamer username (or @name / twitch.tv link)")?,
        })),
        "vods" => {
            let username = ask_username("Channel name")?;
            let picked = Select::new()
                .with_prompt("Video kind")
                .items(&VIDEO_TYPES)
                .default(0)
                .interact()?;
            Ok(Some(Commands::Vods {
                username,
                video_type: VIDEO_TYPES[picked].to_string(),
            }))
        }
        "fix" => {
            let url: String = Input::new()
                .with_prompt("VOD m3u8 playlist URL")
                .interact_text()?;
            let output: String = Input::new()
                .with_prompt("Output file (empty for fixed_playlist.m3u8)")
                .allow_empty(true)
                .interact_text()?;
            let force = Confirm::new()
                .with_prompt("Overwrite the output file if it exists?")
                .default(false)
                .interact()?;
            Ok(Some(Commands::Fix {
                url,
                output: (!output.trim().is_empty()).then(|| output.trim().to_string()),
                force,
            }))
        }
        _ => Ok(None),
    }
}

/// Ask for a numeric offset.
fn ask_offset(prompt: &str) -> Result<i64> {
    Ok(Input::new()
        .with_prompt(prompt)
        .validate_with(|input: &String| {
            input
                .trim()
                .parse::<i64>()
                .map(|_| ())
                .map_err(|_| "expected a number of seconds".to_string())
        })
        .interact_text()?
        .trim()
        .parse()?)
}

/// Run the guided menu until the user quits.
///
/// Each command runs and returns to the menu; command errors print cleanly
/// and never end the session. Input errors (EOF) quit.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `flags` - Global CLI flags.
///
/// # Errors
///
/// Returns an error when the menu itself cannot interact with the terminal.
pub async fn run(client: &Client, flags: &Cli) -> Result<()> {
    loop {
        let items: Vec<&str> = MENU.iter().map(|(_, description)| *description).collect();
        let picked = match Select::new()
            .with_prompt("What to do?")
            .items(&items)
            .default(0)
            .interact()
        {
            Ok(picked) => picked,
            Err(_) => return Ok(()),
        };
        let key = MENU[picked].0;
        let command = match build_command(key) {
            Ok(command) => command,
            Err(_) => return Ok(()),
        };
        let Some(command) = command else {
            return Ok(());
        };
        if let Err(report) = execute_command(command, client, flags).await {
            print_error(&report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_strips_at_prefix() {
        assert_eq!(normalize_username("@Arquel"), "arquel");
    }

    #[test]
    fn username_extracts_twitch_link() {
        assert_eq!(normalize_username("https://www.twitch.tv/Arquel"), "arquel");
        assert_eq!(normalize_username("twitch.tv/arquel"), "arquel");
    }

    #[test]
    fn username_rejects_empty() {
        assert!(normalize_username("   ").is_empty());
    }

    #[test]
    fn menu_quit_stays_last() {
        assert_eq!(MENU[MENU.len() - 1].0, "quit");
    }
}
