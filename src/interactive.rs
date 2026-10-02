use std::io::IsTerminal;

use anyhow::Result;
use dialoguer::{Confirm, Input, Select};
use reqwest::Client;

use crate::cli::{GlobalOpts, VideoType};
use crate::commands::bruteforce::CONFIRM_THRESHOLD;
use crate::commands::execute_command;
use crate::commands::link::parse_tracker_target;
use crate::report::print_error;
use crate::util::{format_utc, normalize_login, parse_login, parse_timestamp, range_len};

/// Guided menu choices; Quit stays last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Menu {
    /// Check one timestamp for a VOD.
    Exact,
    /// Scan a timestamp range.
    Bruteforce,
    /// Scan a VOD for clips.
    Clipforce,
    /// Look up a StreamsCharts stream page.
    Link,
    /// Find the DVR playlist of a live stream.
    Live,
    /// List a channel's VODs with links.
    Vods,
    /// Repair an unmuted VOD playlist.
    Fix,
    /// Leave the menu.
    Quit,
}

impl Menu {
    /// Every choice in display order.
    const ALL: [Menu; 8] = [
        Menu::Exact,
        Menu::Bruteforce,
        Menu::Clipforce,
        Menu::Link,
        Menu::Live,
        Menu::Vods,
        Menu::Fix,
        Menu::Quit,
    ];

    /// One-line description shown in the menu.
    fn description(self) -> &'static str {
        match self {
            Menu::Exact => "Exact - check one timestamp for a VOD",
            Menu::Bruteforce => "Bruteforce - scan a timestamp range",
            Menu::Clipforce => "Clipforce - scan a VOD for clips",
            Menu::Link => "Link - look up a StreamsCharts stream page",
            Menu::Live => "Live - find the DVR playlist of a live stream",
            Menu::Vods => "Vods - list a channel's VODs with links",
            Menu::Fix => "Fix - repair an unmuted VOD playlist",
            Menu::Quit => "Quit",
        }
    }
}

/// Quiet Ctrl-C quit, distinct from real terminal failures.
#[derive(Debug)]
struct QuietQuit;

impl std::fmt::Display for QuietQuit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled")
    }
}

impl std::error::Error for QuietQuit {}

/// Run one dialoguer prompt, mapping Ctrl-C to a quiet quit.
///
/// # Arguments
///
/// * `result` - Dialoguer interaction result.
fn prompt<T>(result: Result<T, dialoguer::Error>) -> Result<T> {
    result.map_err(|error| match error {
        dialoguer::Error::IO(io) if io.kind() == std::io::ErrorKind::Interrupted => {
            anyhow::anyhow!(QuietQuit)
        }
        error => error.into(),
    })
}

/// Ask for a non-empty username, accepting `@name` and twitch.tv links.
fn ask_username(prompt_text: &str) -> Result<String> {
    let raw: String = prompt(
        Input::new()
            .with_prompt(prompt_text)
            .validate_with(|input: &String| parse_login(input).map(|_| ()))
            .interact_text(),
    )?;
    Ok(normalize_login(&raw))
}

/// Extract the video ID from a `twitch.tv/videos/<id>` link.
///
/// # Arguments
///
/// * `raw` - Raw ID field.
///
/// # Returns
///
/// Owned ID text when the input is a videos link, `None` otherwise.
fn videos_link_id(raw: &str) -> Option<String> {
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = url::Url::parse(&candidate).ok()?;
    let twitch = url
        .host_str()
        .is_some_and(|host| host == "twitch.tv" || host == "www.twitch.tv");
    if !twitch {
        return None;
    }
    let mut segments = url.path_segments()?;
    if segments.next()? != "videos" {
        return None;
    }
    segments.next().map(str::to_string)
}

/// Parse a VOD/broadcast ID, accepting `twitch.tv/videos/<id>` links.
///
/// # Arguments
///
/// * `raw` - Raw ID field.
///
/// # Errors
///
/// Returns an error for non-numeric IDs.
fn parse_id(raw: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if let Some(id) = videos_link_id(trimmed) {
        return id.parse::<u64>().map_err(|error| anyhow::anyhow!(error));
    }
    trimmed
        .parse::<u64>()
        .map_err(|error| anyhow::anyhow!(error))
}

/// Ask for a VOD/broadcast ID.
fn ask_id(prompt_text: &str) -> Result<u64> {
    let raw: String = prompt(
        Input::new()
            .with_prompt(prompt_text)
            .validate_with(|input: &String| {
                parse_id(input)
                    .map(|_| ())
                    .map_err(|_| "expected a numeric ID or twitch.tv/videos link".to_string())
            })
            .interact_text(),
    )?;
    parse_id(&raw)
}

/// Ask for a timestamp, echoing the parsed UTC value back.
fn ask_timestamp(prompt_text: &str) -> Result<i64> {
    let raw: String = prompt(
        Input::new()
            .with_prompt(format!(
                "{prompt_text} [unix, RFC3339, or YYYY-MM-DD HH:MM[:SS]]"
            ))
            .validate_with(|input: &String| {
                parse_timestamp(input.trim())
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
            .interact_text(),
    )?;
    let timestamp = parse_timestamp(raw.trim())?;
    println!("  -> {}", format_utc(timestamp));
    Ok(timestamp)
}

/// Ask for a StreamsCharts stream page URL.
fn ask_tracker_url() -> Result<String> {
    prompt(
        Input::new()
            .with_prompt("StreamsCharts stream page URL")
            .validate_with(|input: &String| {
                parse_tracker_target(input.trim())
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
            .interact_text(),
    )
}

/// Ask whether a large scan may proceed.
///
/// Small scans pass silently; estimates above `CONFIRM_THRESHOLD` get a
/// `Confirm` prompt defaulting to no.
///
/// # Arguments
///
/// * `estimate` - Approximate request count.
///
/// # Errors
///
/// Returns terminal errors, mapping Ctrl-C to a quiet [`QuietQuit`].
fn confirm_large(estimate: u64) -> Result<bool> {
    if estimate <= CONFIRM_THRESHOLD {
        return Ok(true);
    }
    eprintln!("This scan implies about {estimate} requests.");
    prompt(
        Confirm::new()
            .with_prompt("Proceed?")
            .default(false)
            .interact(),
    )
}

/// Build the guided command for one menu pick.
///
/// `None` returns to the menu (declined confirmation).
fn build_command(choice: Menu, opts: &GlobalOpts) -> Result<Option<crate::cli::Commands>> {
    use crate::cli::Commands;
    match choice {
        Menu::Exact => Ok(Some(Commands::Exact {
            username: ask_username("Streamer username (or @name / twitch.tv link)")?,
            id: ask_id("VOD/broadcast ID")?,
            stamp: ask_timestamp("Timestamp")?,
        })),
        Menu::Bruteforce => {
            let username = ask_username("Streamer username (or @name / twitch.tv link)")?;
            let id = ask_id("VOD/broadcast ID")?;
            let from = ask_timestamp("Range start")?;
            let to = ask_timestamp("Range end")?;
            let stop_first = prompt(
                Confirm::new()
                    .with_prompt("Stop at the first hit?")
                    .default(true)
                    .interact(),
            )?;
            let estimate =
                range_len(from.min(to), from.max(to)).saturating_mul(opts.cdn_hosts().len() as u64);
            let yes = confirm_large(estimate)?;
            if !yes {
                return Ok(None);
            }
            Ok(Some(Commands::Bruteforce {
                username,
                id,
                from,
                to,
                all: !stop_first,
                yes,
            }))
        }
        Menu::Clipforce => {
            let id = ask_id("VOD/broadcast ID")?;
            let start = ask_offset("Start offset in seconds")?;
            let end = ask_offset("End offset in seconds")?;
            let estimate = range_len(start.min(end), start.max(end));
            let yes = confirm_large(estimate)?;
            if !yes {
                return Ok(None);
            }
            Ok(Some(Commands::Clipforce {
                id,
                start,
                end,
                yes,
            }))
        }
        Menu::Link => Ok(Some(Commands::Link {
            url: ask_tracker_url()?,
        })),
        Menu::Live => Ok(Some(Commands::Live {
            username: ask_username("Streamer username (or @name / twitch.tv link)")?,
        })),
        Menu::Vods => {
            let username = ask_username("Channel name")?;
            let labels: Vec<&str> = VideoType::ALL.iter().map(|kind| kind.label()).collect();
            let picked = prompt(
                Select::new()
                    .with_prompt("Video kind")
                    .items(&labels)
                    .default(0)
                    .interact(),
            )?;
            Ok(Some(Commands::Vods {
                username,
                video_type: VideoType::ALL[picked],
            }))
        }
        Menu::Fix => {
            let url: String = prompt(
                Input::new()
                    .with_prompt("VOD m3u8 playlist URL")
                    .interact_text(),
            )?;
            let output: String = prompt(
                Input::new()
                    .with_prompt("Output file (empty for fixed_playlist.m3u8)")
                    .allow_empty(true)
                    .interact_text(),
            )?;
            let force = prompt(
                Confirm::new()
                    .with_prompt("Overwrite the output file if it exists?")
                    .default(false)
                    .interact(),
            )?;
            Ok(Some(Commands::Fix {
                url,
                output: (!output.trim().is_empty()).then(|| output.trim().to_string()),
                force,
            }))
        }
        Menu::Quit => Ok(None),
    }
}

/// Ask for a numeric offset.
fn ask_offset(prompt_text: &str) -> Result<i64> {
    prompt(
        Input::new()
            .with_prompt(prompt_text)
            .validate_with(|input: &String| {
                input
                    .trim()
                    .parse::<i64>()
                    .map(|_| ())
                    .map_err(|_| "expected a number of seconds".to_string())
            })
            .interact_text(),
    )?
    .trim()
    .parse::<i64>()
    .map_err(|error| anyhow::anyhow!(error))
}

/// Run the guided menu until the user quits.
///
/// Each command runs and returns to the menu; command errors print cleanly
/// and never end the session. Esc quits; Ctrl-C quits quietly.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `opts` - Global CLI options.
///
/// # Errors
///
/// Returns an error when the menu itself cannot interact with the terminal.
pub async fn run(client: &Client, opts: &GlobalOpts) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("Interactive mode needs a terminal; pass a subcommand instead.");
    }
    loop {
        let items: Vec<&str> = Menu::ALL
            .iter()
            .map(|choice| choice.description())
            .collect();
        let picked = match prompt(
            Select::new()
                .with_prompt("What to do?")
                .items(&items)
                .default(0)
                .interact_opt(),
        )? {
            Some(picked) => picked,
            None => return Ok(()),
        };
        let Some(choice) = Menu::ALL.get(picked).copied() else {
            return Ok(());
        };
        if choice == Menu::Quit {
            return Ok(());
        }
        let command = match build_command(choice, opts) {
            Ok(command) => command,
            Err(report) if report.is::<QuietQuit>() => return Ok(()),
            Err(report) => return Err(report),
        };
        let Some(command) = command else {
            continue;
        };
        if let Err(report) = execute_command(command, client, opts).await {
            print_error(&report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_quit_stays_last() {
        assert_eq!(*Menu::ALL.last().expect("menu is not empty"), Menu::Quit);
    }

    #[test]
    fn menu_descriptions_cover_every_choice() {
        assert_eq!(Menu::ALL.len(), 8);
        for choice in Menu::ALL {
            assert!(!choice.description().is_empty());
        }
    }

    #[test]
    fn videos_link_extracts_id() {
        assert_eq!(
            videos_link_id("https://www.twitch.tv/videos/316969565142").as_deref(),
            Some("316969565142")
        );
        assert_eq!(
            videos_link_id("twitch.tv/videos/123").as_deref(),
            Some("123")
        );
        assert!(videos_link_id("twitch.tv/arquel").is_none());
        assert!(videos_link_id("https://example.com/videos/123").is_none());
    }

    #[test]
    fn id_parses_plain_and_link_forms() {
        assert_eq!(parse_id("316969565142").expect("plain ID"), 316_969_565_142);
        assert_eq!(parse_id("twitch.tv/videos/123").expect("link ID"), 123);
        assert!(parse_id("not-a-number").is_err());
        assert!(parse_id("-5").is_err());
    }

    #[test]
    fn quiet_quit_marks_cancelled() {
        let io = std::io::Error::new(std::io::ErrorKind::Interrupted, "eof");
        let error = prompt::<()>(Err(dialoguer::Error::IO(io))).expect_err("quit is an error");
        assert!(error.is::<QuietQuit>());
    }
}
