use anyhow::Result;
use colored::Colorize;
use log::error;
use std::io::stdin;
use strum::{EnumMessage, IntoEnumIterator};

use crate::config::{Cli, Commands, ProcessingType};
use crate::twitch::{
    clips::{clip_bruteforce, find_bid_from_clip},
    models::ReturnURL,
    vods::{bruteforcer, exact, fix, live},
};
use crate::update::update;
use crate::util::derive_date_from_url;

impl Commands {
    fn fill_out_values(&mut self) -> Result<()> {
        match self {
            Self::Exact {
                username,
                id,
                stamp,
            } => {
                let mut vod = String::new();

                ask_for_value("Please enter the streamer's username:", username);

                ask_for_value("Please enter the VOD/broadcast ID:", &mut vod);
                *id = vod.parse::<i64>()?;

                ask_for_value("Please enter the timestamp:", stamp);

                Ok(())
            }
            Self::Bruteforce {
                username,
                id,
                from,
                to,
            } => {
                let mut vod = String::new();

                ask_for_value("Please enter the streamer's username:", username);

                ask_for_value("Please enter the VOD/broadcast ID:", &mut vod);
                *id = vod.parse::<i64>()?;

                ask_for_value("Please enter the first timestamp: [year]-[month]-[day] [hour]:[minute]:[second]", from);
                ask_for_value("Please enter the last timestamp: [year]-[month]-[day] [hour]:[minute]:[second]", to);

                Ok(())
            }
            Self::Link { url } => {
                ask_for_value("Please enter the TwitchTracker or StreamsCharts URL:", url);
                Ok(())
            }
            Self::Live { username } => {
                ask_for_value("Please enter the streamer's username:", username);
                Ok(())
            }
            Self::Clip { clip } => {
                ask_for_value("Please enter the clip's URL (twitch.tv/%username%/clip/%slug% and clips.twitch.tv/%slug% are both supported) or the slug (\"GentleAthleticWombatHoneyBadger-ohJAsKzGinIgFUx2\" for example):", clip);
                Ok(())
            }
            Self::Clipforce { id, start, end } => {
                let mut id_string = String::new();
                let mut start_string = String::new();
                let mut end_string = String::new();

                ask_for_value("Please enter the VOD/broadcast ID:", &mut id_string);
                *id = id_string.parse::<i64>()?;

                ask_for_value(
                    "Please enter the starting timestamp (in seconds):",
                    &mut start_string,
                );
                *start = start_string.parse::<i64>()?;

                ask_for_value(
                    "Please enter the end timestamp (in seconds):",
                    &mut end_string,
                );
                *end = end_string.parse::<i64>()?;

                Ok(())
            }
            Self::Fix { url, .. } => {
                ask_for_value("Please enter Twitch VOD m3u8 playlist URL (only twitch.tv and cloudfront.net URLs are supported):", url);
                Ok(())
            }
            Self::Update => Ok(()),
        }
    }

    pub async fn execute(&self, matches: Cli) -> Result<Option<Vec<ReturnURL>>> {
        match self {
            Self::Exact {
                username,
                id,
                stamp,
            } => exact(username.as_str(), *id, stamp.as_str(), matches).await,
            Self::Bruteforce {
                username,
                id,
                from,
                to,
            } => bruteforcer(username.as_str(), *id, from.as_str(), to.as_str(), matches).await,
            Self::Link { url } => {
                let (proc, data) = match derive_date_from_url(url, matches.clone()).await {
                    Ok(a) => a,
                    Err(e) => {
                        return Err(e)?;
                    }
                };

                match proc {
                    ProcessingType::Exact => {
                        exact(
                            data.username.as_str(),
                            match data.broadcast_id.parse::<i64>() {
                                Ok(b) => b,
                                Err(e) => {
                                    return Err(e)?;
                                }
                            },
                            data.start_date.as_str(),
                            matches.clone(),
                        )
                        .await
                    }
                    ProcessingType::Bruteforce => {
                        let end_date = match data.end_date {
                            Some(d) => d,
                            None => {
                                error!("Couldn't get the end date for the bruteforce method");
                                return Ok(None);
                            }
                        };
                        bruteforcer(
                            data.username.as_str(),
                            match data.broadcast_id.parse::<i64>() {
                                Ok(b) => b,
                                Err(e) => return Err(e)?,
                            },
                            data.start_date.as_str(),
                            end_date.as_str(),
                            matches.clone(),
                        )
                        .await
                    }
                }
            }
            Self::Live { username } => live(username.as_str(), matches).await,
            Self::Clip { clip } => match find_bid_from_clip(clip.clone(), matches.clone()).await {
                Ok(r) => match r {
                    Some((username, vod)) => {
                        let url = format!("https://twitchtracker.com/{username}/streams/{vod}");
                        let (_, data) = match derive_date_from_url(&url, matches.clone()).await {
                            Ok(a) => a,
                            Err(e) => Err(e)?,
                        };

                        exact(&username, vod, &data.start_date, matches).await
                    }
                    None => Ok(None),
                },
                Err(e) => Err(e)?,
            },
            Self::Clipforce { id, start, end } => clip_bruteforce(*id, *start, *end, matches).await,
            Self::Fix { url, output, slow } => {
                if let Err(e) = fix(url.as_str(), output.clone(), *slow, matches).await {
                    error!("Failed to fix playlist: {e}");
                }
                // this might not be the right way to this
                // but i want to combine everything into one method
                Ok(None)
            }
            Self::Update => {
                match update(matches).await {
                    Ok(_) => (),
                    Err(e) => return Err(e)?,
                }
                Ok(None)
            }
        }
    }
}

pub fn trim_newline(s: &mut String) {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
}

fn ask_for_value(desc: &str, buf: &mut String) {
    println!("{}", desc.bright_blue());
    stdin().read_line(buf).expect("Failed to read line.");
    trim_newline(buf);
}

async fn try_to_fix(valid_urls: Vec<ReturnURL>, matches: Cli) {
    if !valid_urls.is_empty() && valid_urls[0].muted {
        let mut response = String::new();

        ask_for_value(
            "Do you want to download the fixed playlist? (Y/n)",
            &mut response,
        );

        match response.to_lowercase().as_str() {
            "y" | "" => {
                let fix_command = Commands::Fix {
                    url: valid_urls[0].url.clone(),
                    output: None,
                    slow: false,
                };
                if let Err(e) = fix_command.execute(matches).await {
                    error!("Failed to fix playlist: {e}");
                }
            }
            _ => (),
        };
    }
}

pub async fn main_interface(mut matches: Cli) {
    // forcing the progress bar option on
    matches = Cli {
        progressbar: true,
        threads: matches.threads,
        ..matches
    };

    loop {
        let mut mode = String::new();

        println!("{}", "Select the application mode:".green());
        for (i, com) in Commands::iter().enumerate() {
            let selector = match com.to_selector() {
                Some(str) => str,
                None => (i + 1).to_string(),
            };
            let name = com.to_short_desc();

            match com.show_description() {
                true => println!(
                    "[{}] {} - {}",
                    selector.yellow(),
                    name.bright_green(),
                    com.get_documentation()
                        .unwrap_or("<error - couldn't get mode description>")
                        .italic()
                ),
                false => println!("[{}] {}", selector.yellow(), name.bright_green()),
            }
        }

        stdin().read_line(&mut mode).expect("Failed to read line.");
        trim_newline(&mut mode);

        match Commands::from_selector(mode) {
            Some(mut sub) => {
                if let Err(e) = sub.fill_out_values() {
                    error!("{e}");
                    continue;
                }
                let valid_urls = match sub.execute(matches.clone()).await {
                    Ok(u) => match u {
                        Some(u) => u,
                        None => Vec::new(),
                    },
                    Err(e) => {
                        error!("{e}");
                        continue;
                    }
                };
                try_to_fix(valid_urls, matches.clone()).await;
            }
            None => {
                error!("Couldn't select the specified mode");
                continue;
            }
        }
    }
}
