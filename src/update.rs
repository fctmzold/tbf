use anyhow::Result;
use clap::crate_version;
use guess_host_triple::guess_host_triple;
use reqwest::header::USER_AGENT;
use semver::Version;
use serde::Deserialize;

use crate::config::{Cli, CURL_UA};

#[derive(Debug, Deserialize)]
struct GithubUpdate {
    tag_name: String,
    assets: Vec<GithubAssets>,
}

#[derive(Debug, Deserialize)]
struct GithubAssets {
    browser_download_url: String,
}

pub async fn update(matches: Cli) -> Result<()> {
    let target_triple = guess_host_triple();
    let current_version = crate_version!();
    let cur_version_parsed = Version::parse(current_version)?;

    let resp = crate::HTTP_CLIENT
        .get("https://api.github.com/repos/vyneer/tbf/releases/latest")
        .header(USER_AGENT, CURL_UA)
        .send()
        .await;

    let gh = match resp {
        Ok(r) if r.status().is_success() => {
            let gh: GithubUpdate = r.json().await?;
            gh
        }
        Ok(r) => {
            if !matches.simple {
                println!("Failed to fetch latest release: {}", r.status());
            }
            return Ok(());
        }
        Err(e) => {
            if !matches.simple {
                println!("Failed to connect to GitHub: {}", e);
            }
            return Ok(());
        }
    };

    if !gh.tag_name.is_empty() && !gh.assets.is_empty() {
        // Remove the 'v' prefix from tag_name
        let tag_name = if gh.tag_name.starts_with('v') {
            &gh.tag_name[1..]
        } else {
            &gh.tag_name
        };

        match Version::parse(tag_name) {
            Ok(new_version_parsed) => {
                if new_version_parsed > cur_version_parsed {
                    if !matches.simple {
                        println!("New version available ({}):", gh.tag_name);
                    }
                    for url in gh.assets {
                        match target_triple {
                            Some(triple) => {
                                if url.browser_download_url.contains(triple) {
                                    println!("{}", url.browser_download_url)
                                }
                            }
                            None => println!("{}", url.browser_download_url),
                        }
                    }
                } else if !matches.simple {
                    println!("No updates available");
                }
            }
            Err(e) => {
                if !matches.simple {
                    println!("Failed to parse version: {}", e);
                }
            }
        }
    } else if !matches.simple {
        println!("No release information available");
    }

    Ok(())
}
