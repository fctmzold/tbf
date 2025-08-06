use anyhow::Result;
use colored::*;
use indicatif::{ParallelProgressIterator, ProgressBar};
use log::{error, info};
use rayon::prelude::*;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::{collections::HashMap, str::FromStr};
use url::Url;

use crate::config::Cli;
use crate::error::Clip;
use crate::twitch::models::{ClipQuery, ClipResponse, ClipVars, ReturnURL};
use crate::util::info;

fn extract_slug(s: String) -> Result<Option<String>> {
    match Url::parse(s.as_str()) {
        Ok(resolved_url) => match resolved_url.domain() {
            Some(domain) => match domain.to_lowercase().as_str() {
                "twitch.tv" | "www.twitch.tv" => {
                    let segments = match resolved_url
                        .path_segments()
                        .map(|c| c.collect::<Vec<_>>())
                        .ok_or(Clip::SegmentMap)
                    {
                        Ok(s) => s,
                        Err(e) => return Err(e)?,
                    };
                    if segments.len() > 1 {
                        if segments[1] == "clip" {
                            Ok(Some(segments[2].to_string()))
                        } else {
                            Err(Clip::WrongURL("Not a clip URL".to_string()))?
                        }
                    } else {
                        Err(Clip::WrongURL("Not a clip URL".to_string()))?
                    }
                }
                "clips.twitch.tv" => {
                    let segments = match resolved_url
                        .path_segments()
                        .map(|c| c.collect::<Vec<_>>())
                        .ok_or(Clip::SegmentMap)
                    {
                        Ok(s) => s,
                        Err(e) => return Err(e)?,
                    };
                    Ok(Some(segments[0].to_string()))
                }
                _ => Err(Clip::WrongURL(
                    "Only twitch.tv URLs are supported".to_string(),
                ))?,
            },
            None => Err(Clip::WrongURL(
                "Only twitch.tv URLs are supported".to_string(),
            ))?,
        },
        Err(_) => Ok(Some(s)),
    }
}

pub fn find_bid_from_clip(s: String, flags: Cli) -> Result<Option<(String, i64)>> {
    let slug = match extract_slug(s) {
        Ok(s) => match s {
            Some(s) => s,
            None => return Ok(None),
        },
        Err(e) => return Err(e),
    };
    let endpoint = "https://gql.twitch.tv/gql";
    let mut headers = HashMap::new();
    headers.insert("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko");

    let mut header_map = HeaderMap::new();

    for (str_key, str_value) in headers {
        let key = match HeaderName::from_str(str_key) {
            Ok(h) => h,
            Err(e) => return Err(e)?,
        };
        let val = match HeaderValue::from_str(str_value) {
            Ok(h) => h,
            Err(e) => return Err(e)?,
        };

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

    let re = match request.send() {
        Ok(r) => r,
        Err(e) => return Err(e)?,
    };
    let data: ClipResponse = match re.json() {
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
        match data.data.clip.broadcast.id.parse::<i64>() {
            Ok(i) => i,
            Err(e) => return Err(e)?,
        },
    )))
}

pub fn clip_bruteforce(
    vod: i64,
    start: i64,
    end: i64,
    flags: Cli,
) -> Result<Option<Vec<ReturnURL>>> {
    let vod = vod.to_string();
    let pb = ProgressBar::new((end - start) as u64);
    let cloned_pb = pb.clone();

    let iter = (start..end).into_par_iter();
    let iter_pb = (start..end).into_par_iter().progress_with(pb);

    let res: Vec<ReturnURL> = if flags.progressbar {
        iter_pb.filter_map( |number| {
            let url = format!("https://clips-media-assets2.twitch.tv/{vod}-offset-{number}.mp4");
            let res = match crate::HTTP_CLIENT.get(url.as_str()).send() {
                Ok(r) => r,
                Err(e) => {
                    cloned_pb.println(format!("Error sending request for {}: {}", url, e));
                    return None
                }
            };
            cloned_pb.println(format!("Checking URL: {} - Status: {}", url, res.status()));
            if res.status() == 200 {
                if flags.verbose {
                    cloned_pb.println(format!("Got a clip! - {url}"));
                }
                Some(ReturnURL {
                    url,
                    muted: false,
                })
            } else if res.status() == 403 {
                if flags.verbose {
                    cloned_pb.println(format!("Still going! - {url}"));
                }
                None
            } else {
                cloned_pb.println(format!("You might be getting throttled (or your connection is dead)! Status code: {} - URL: {}", res.status(), res.url()));
                None
            }
        }).collect()
    } else {
        iter.filter_map( |number| {
            let url = format!("https://clips-media-assets2.twitch.tv/{vod}-offset-{number}.mp4");
            let res = match crate::HTTP_CLIENT.get(url.as_str()).send() {
                Ok(r) => r,
                Err(e) => {
                    cloned_pb.println(format!("Error sending request for {}: {}", url, e));
                    return None
                }
            };
            cloned_pb.println(format!("Checking URL: {} - Status: {}", url, res.status()));
            if res.status() == 200 {
                if flags.verbose {
                    cloned_pb.println(format!("Got a clip! - {url}"));
                }
                Some(ReturnURL {
                    url,
                    muted: false,
                })
            } else if res.status() == 403 {
                if flags.verbose {
                    cloned_pb.println(format!("Still going! - {url}"));
                }
                None
            } else {
                cloned_pb.println(format!("You might be getting throttled (or your connection is dead)! Status code: {} - URL: {}", res.status(), res.url()));
                None
            }
        }).collect()
    };

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

    #[test]
    fn find_bid_from_clip() {
        assert_eq!(
            bid(
                "SpotlessCrypticStapleAMPTropPunch-H_rVu0mGfGLNMlEx".to_string(),
                Cli::default()
            )
            .unwrap(),
            Some(("mrmouton".to_string(), 39905263305)),
            "testing valid clip"
        );
        assert_eq!(
            bid(
                "SpotlessCrypticStapleAMPTropPunch-H_rVu0mfGLNMlEx".to_string(),
                Cli::default()
            )
            .unwrap(),
            None,
            "testing invalid clip"
        );
    }
}
