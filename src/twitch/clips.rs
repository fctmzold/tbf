use anyhow::Result;
use colored::*;
use futures::StreamExt;
use indicatif::ProgressBar;
use log::{error, info};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::{collections::HashMap, str::FromStr};
use url::Url;

use crate::config::Cli;
use crate::error::Clip;
use crate::twitch::models::{ClipQuery, ClipResponse, ClipVars, ReturnURL};
use crate::util::info;

fn extract_slug(s: String) -> Result<Option<String>> {
    match Url::parse(&s) {
        Ok(resolved_url) => {
            let domain = resolved_url
                .domain()
                .ok_or_else(|| Clip::WrongURL("Invalid URL".to_string()))?;

            match domain.to_lowercase().as_str() {
                "twitch.tv" | "www.twitch.tv" => {
                    let segments: Vec<_> = resolved_url
                        .path_segments()
                        .map(|c| c.collect())
                        .ok_or(Clip::SegmentMap)?;

                    if segments.len() > 1 && segments[1] == "clip" {
                        Ok(Some(segments[2].to_string()))
                    } else {
                        Err(Clip::WrongURL("Not a clip URL".to_string()))?
                    }
                }
                "clips.twitch.tv" => {
                    let segments: Vec<_> = resolved_url
                        .path_segments()
                        .map(|c| c.collect())
                        .ok_or(Clip::SegmentMap)?;

                    Ok(Some(segments[0].to_string()))
                }
                _ => Err(Clip::WrongURL(
                    "Only twitch.tv URLs are supported".to_string(),
                ))?,
            }
        }
        Err(_) => Ok(Some(s)), // Assume it's already a slug
    }
}

pub async fn find_bid_from_clip(s: String, flags: Cli) -> Result<Option<(String, i64)>> {
    let slug = match extract_slug(s) {
        Ok(Some(slug)) => slug,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };

    let endpoint = "https://gql.twitch.tv/gql";
    let mut headers = HashMap::new();
    headers.insert("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko");

    let mut header_map = HeaderMap::new();

    for (str_key, str_value) in headers {
        let key = HeaderName::from_str(str_key)?;
        let val = HeaderValue::from_str(str_value)?;

        header_map.insert(key, val);
    }

    let query = ClipQuery {
        query: "query($slug:ID!){clip(slug: $slug){broadcaster{login}broadcast{id}}}".to_string(),
        variables: ClipVars { slug },
    };

    let request = crate::HTTP_CLIENT
        .post(endpoint)
        .json(&query)
        .headers(header_map.clone());

    let re = request.send().await?;
    let data: ClipResponse = match re.json().await {
        Ok(d) => d,
        Err(e) => {
            if !flags.simple {
                error!("Couldn't get the info from the clip: {e}");
            }
            return Ok(None);
        }
    };

    Ok(Some((
        data.data.clip.broadcaster.login,
        data.data.clip.broadcast.id.parse::<i64>()?,
    )))
}

pub async fn clip_bruteforce(
    vod: i64,
    start: i64,
    end: i64,
    flags: Cli,
) -> Result<Option<Vec<ReturnURL>>> {
    let vod = vod.to_string();
    let pb = ProgressBar::new((end - start) as u64);

    let fetches = futures::stream::iter((start..end).map(|number| {
        let url = format!(
            "https://clips-media-assets2.twitch.tv/{vod}-offset-{number}.mp4"
        );
        let pb_clone = pb.clone();
        async move {
            match crate::HTTP_CLIENT.get(url.as_str()).send().await {
                Ok(r) => {
                    if flags.progressbar {
                        pb_clone.inc(1);
                    }
                    if r.status() == 200 {
                        if flags.verbose {
                            pb_clone.println(format!("Got a clip! - {url}"));
                        }
                        Some(ReturnURL {
                            url,
                            muted: false,
                        })
                    } else if r.status() == 403 {
                        if flags.verbose {
                            pb_clone.println(format!("Still going! - {url}"));
                        }
                        None
                    } else {
                        pb_clone.println(format!(
                            "You might be getting throttled (or your connection is dead)! Status code: {} - URL: {}",
                            r.status(),
                            r.url()
                        ));
                        None
                    }
                }
                Err(e) => {
                    pb_clone.println(format!("Error sending request for {}: {}", url, e));
                    None
                }
            }
        }
    }))
    .buffer_unordered(flags.threads)
    .collect::<Vec<Option<ReturnURL>>>()
    .await;

    let res: Vec<ReturnURL> = fetches.into_iter().flatten().collect();

    if !res.is_empty() {
        if !flags.simple {
            info!("{}! Here are the URLs:", "Got some clips".green());
        }
        for line in res.clone() {
            info(line.url, flags.simple);
        }
    } else if !flags.simple {
        info!("{}", "Couldn't find anything :(".red());
    }
    Ok(Some(res))
}

#[cfg(test)]
mod tests {
    use crate::config::Cli;

    use super::{extract_slug as es, find_bid_from_clip as bid};

    #[test]
    fn extract_slug() {
        assert_eq!(
            es("SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string()).unwrap(),
            Some("SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string()),
            "testing slug string"
        );
        assert_eq!(es("https://www.twitch.tv/mrmouton/clip/SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string()).unwrap(), Some("SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string()), "testing twitch.tv link");
        assert_eq!(
            es(
                "https://clips.twitch.tv/SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx"
                    .to_string()
            )
            .unwrap(),
            Some("SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string()),
            "testing clips.twitch.tv link"
        );
        assert!(
            es("https://google.com".to_string()).is_err(),
            "testing non-twitch link"
        );
        assert!(es("https://www.twitch.tv/mrmouton/clp/SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string()).is_err(), "testing twitch non-clip link 1");
        assert!(
            es(
                "https://cps.twitch.tv/SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx"
                    .to_string()
            )
            .is_err(),
            "testing twitch non-clip link 1"
        );
    }

    #[tokio::test]
    async fn find_bid_from_clip() {
        assert_eq!(
            bid(
                "SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string(),
                Cli::default()
            )
            .await
            .unwrap(),
            Some(("mrmouton".to_string(), 39905263305)),
            "testing valid clip"
        );
        assert_eq!(
            bid(
                "SpotlessCrypticStapleAMPTropPunch-H_rVu0mfGLNMlEx".to_string(),
                Cli::default()
            )
            .await
            .unwrap(),
            None,
            "testing invalid clip"
        );
    }
}
