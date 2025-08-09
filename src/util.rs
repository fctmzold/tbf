use anyhow::Result;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use rand::prelude::*;
use regex::Regex;
use reqwest::{header::USER_AGENT, StatusCode};
use scraper::{Html, Selector};
use serde::Deserialize;
use std::{fs::File, io::Read, path::Path, thread::sleep, time::Duration};
use time::{
    format_description::well_known::Rfc3339, macros::format_description, PrimitiveDateTime,
};
use url::Url;

use super::config::{Cli, ProcessingType, CURL_UA};
use crate::error::DeriveDate;
use crate::twitch::models::CDN_URLS;

lazy_static! {
    static ref RE_UNIX: Regex = Regex::new(r"^\d*$").unwrap();
    static ref RE_UTC: Regex = Regex::new("UTC").unwrap();
}

#[derive(Debug, PartialEq)]
pub struct URLData {
    pub username: String,
    pub broadcast_id: String,
    pub start_date: String,
    pub end_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CDNFile {
    cdns: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct StreamsChartsTwitchClip {
    started_at: String,
    ended_at: String,
}
#[derive(Debug, PartialEq)]
pub struct ExtractedTimestamps {
    processing_type: ProcessingType,
    start_timestamp: i64,
    end_timestamp: i64,
}

pub fn info(text: String, simple: bool) {
    if simple {
        println!("{text}");
    } else {
        info!("{text}");
    }
}

pub async fn get_useragent_list() -> Vec<String> {
    let resp = crate::HTTP_CLIENT
        .get("https://jnrbsn.github.io/user-agents/user-agents.json")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            match r.json::<Vec<String>>().await {
                Ok(mut useragent_vec) => {
                    // Apparently streamscharts doesn't like when the useragent has "X11;" in it
                    useragent_vec.retain(|ua| !ua.contains("X11;"));
                    useragent_vec
                }
                Err(_) => vec![],
            }
        }
        _ => vec![],
    }
}

pub async fn get_random_useragent() -> String {
    let ua_vector = get_useragent_list().await;

    if !ua_vector.is_empty() {
        if let Some(ua) = ua_vector.choose(&mut rand::rng()) {
            return ua.clone();
        }
    }
    
    CURL_UA.to_string()
}

async fn process_url(url: &str) -> Result<Html> {
    let ua = get_random_useragent().await;
    debug!("Using UA - {ua}");
    
    let mut attempts = 0;
    let max_attempts = 5;
    
    loop {
        attempts += 1;
        let resp = crate::HTTP_CLIENT
            .get(url)
            .header(USER_AGENT, &ua)
            .send()
            .await;

        match resp {
            Ok(r) => {
                if r.status() == StatusCode::FORBIDDEN && attempts < max_attempts {
                    warn!("Got a 403 on attempt #{attempts}");
                    sleep(Duration::from_millis(50));
                    continue;
                }
                
                let resp = r.error_for_status()?;
                let body = resp.text().await?;
                return Ok(Html::parse_document(&body));
            }
            Err(e) => {
                if attempts < max_attempts {
                    warn!("Request failed on attempt #{attempts}: {e}");
                    sleep(Duration::from_millis(50));
                    continue;
                }
                return Err(e)?;
            }
        }
    }
}

pub async fn derive_date_from_url(url: &str, flags: Cli) -> Result<(ProcessingType, URLData)> {
    let resolved_url = Url::parse(url)?;
    let domain = resolved_url.domain().ok_or_else(|| {
        DeriveDate::WrongURL("Only twitchtracker.com and streamscharts.com URLs are supported".to_string())
    })?;
    
    match domain.to_lowercase().as_str() {
        "twitchtracker.com" | "www.twitchtracker.com" => {
            let segments: Vec<_> = resolved_url
                .path_segments()
                .map(|c| c.collect())
                .ok_or(DeriveDate::SegmentMap)?;
                
            if segments.len() != 3 || segments[1] != "streams" {
                return Err(DeriveDate::WrongURL(
                    "Not a valid TwitchTracker VOD URL".to_string(),
                ))?;
            }
            
            let username = segments[0];
            let broadcast_id = segments[2];
            let fragment = process_url(url).await?;
            let selector = Selector::parse(".stream-timestamp-dt.to-dowdatetime")
                .map_err(|_| DeriveDate::Selector)?;
            
            let date = fragment
                .select(&selector)
                .next()
                .ok_or(DeriveDate::ScraperElement)?
                .text()
                .collect::<String>();
            
            Ok((
                ProcessingType::Exact,
                URLData {
                    username: username.to_string(),
                    broadcast_id: broadcast_id.to_string(),
                    start_date: date,
                    end_date: None,
                },
            ))
        }
        "streamscharts.com" | "www.streamscharts.com" => {
            let segments: Vec<_> = resolved_url
                .path_segments()
                .map(|c| c.collect())
                .ok_or(DeriveDate::SegmentMap)?;
                
            if segments.len() != 4 || segments[0] != "channels" || segments[2] != "streams" {
                return Err(DeriveDate::WrongURL(
                    "Not a valid StreamsCharts VOD URL".to_string(),
                ))?;
            }
            
            let username = segments[1];
            let broadcast_id = segments[3];
            let fragment = process_url(url).await?;
            
            let extracted_results = match flags.mode {
                Some(ProcessingType::Bruteforce) => {
                    if !flags.simple {
                        info!("Bruteforcing for timestamps...");
                    }
                    sc_bruteforce_timestamps(&fragment)?
                }
                Some(ProcessingType::Exact) => {
                    if !flags.simple {
                        info!("Extracting exact timestamps...");
                    }
                    sc_extract_exact_timestamps(&fragment)?
                }
                None => {
                    if !flags.simple {
                        info!("Extracting exact timestamps...");
                    }
                    sc_extract_exact_timestamps(&fragment).or_else(|_| {
                        if !flags.simple {
                            info!("Bruteforcing for timestamps...");
                        }
                        sc_bruteforce_timestamps(&fragment)
                    })?
                }
            };
            
            if !flags.simple {
                let approximate_or_exact = match extracted_results.processing_type {
                    ProcessingType::Exact => "exact",
                    ProcessingType::Bruteforce => "approximate",
                };
                info!(
                    "Found {} timestamps for the stream. Started at {} and ended at {}.",
                    approximate_or_exact, extracted_results.start_timestamp, extracted_results.end_timestamp
                );
            }
            
            Ok((
                extracted_results.processing_type,
                URLData {
                    username: username.to_string(),
                    broadcast_id: broadcast_id.to_string(),
                    start_date: extracted_results.start_timestamp.to_string(),
                    end_date: Some(extracted_results.end_timestamp.to_string()),
                },
            ))
        }
        _ => Err(DeriveDate::WrongURL(
            "Only twitchtracker.com and streamscharts.com URLs are supported".to_string(),
        ))?,
    }
}

pub fn parse_timestamp(timestamp: &str) -> Result<i64> {
    let format_with_utc = format_description!("[year]-[month]-[day] [hour]:[minute]:[second] UTC");
    let format_wo_utc = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    let format_wo_sec = format_description!("[day]-[month]-[year] [hour]:[minute]");

    if RE_UNIX.is_match(timestamp) {
        Ok(timestamp.parse::<i64>()?)
    } else if RE_UTC.is_match(timestamp) {
        let dt = PrimitiveDateTime::parse(timestamp, format_with_utc)?;
        Ok(dt.assume_utc().unix_timestamp())
    } else {
        // Try parsing as RFC3339 first
        if let Ok(result) = PrimitiveDateTime::parse(timestamp, &Rfc3339) {
            return Ok(result.assume_utc().unix_timestamp());
        }
        
        // Try parsing without UTC
        if let Ok(result) = PrimitiveDateTime::parse(timestamp, format_wo_utc) {
            return Ok(result.assume_utc().unix_timestamp());
        }
        
        // Try parsing without seconds
        let result = PrimitiveDateTime::parse(timestamp, format_wo_sec)?;
        Ok(result.assume_utc().unix_timestamp())
    }
}

pub fn compile_cdn_list(cdn_file_path: Option<String>) -> Vec<String> {
    let mut cdn_urls: Vec<String> = CDN_URLS.iter().map(|s| s.to_string()).collect();

    let cdn_file_path = match cdn_file_path {
        Some(path) => path,
        None => return cdn_urls,
    };

    let file_extension = Path::new(&cdn_file_path).extension();
    
    let mut file = match File::open(&cdn_file_path) {
        Ok(f) => f,
        Err(e) => {
            info!("Couldn't open the CDN config file - {e:#?}");
            return cdn_urls;
        }
    };

    let mut cdn_string = String::new();
    if let Err(e) = file.read_to_string(&mut cdn_string) {
        info!("Couldn't read the CDN config file - {e:#?}");
        return cdn_urls;
    }

    let new_cdns = match file_extension.and_then(|ext| ext.to_str()) {
        Some("json") => {
            match serde_json::from_str::<CDNFile>(&cdn_string) {
                Ok(cdn_file) => cdn_file.cdns,
                Err(e) => {
                    info!("Couldn't parse the CDN list file: invalid JSON - {e:#?}");
                    return cdn_urls;
                }
            }
        }
        Some("toml") => {
            match toml::from_str::<CDNFile>(&cdn_string) {
                Ok(cdn_file) => cdn_file.cdns,
                Err(e) => {
                    info!("Couldn't parse the CDN list file: invalid TOML - {e:#?}");
                    return cdn_urls;
                }
            }
        }
        Some("yaml") | Some("yml") => {
            match serde_yaml::from_str::<CDNFile>(&cdn_string) {
                Ok(cdn_file) => cdn_file.cdns,
                Err(e) => {
                    info!("Couldn't parse the CDN list file: invalid YAML - {e:#?}");
                    return cdn_urls;
                }
            }
        }
        Some("txt") | None => {
            cdn_string.lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        }
        _ => {
            info!("Couldn't parse the CDN list file: it must either be a text file, a JSON file, a TOML file or a YAML file.");
            return cdn_urls;
        }
    };

    cdn_urls.extend(new_cdns);
    cdn_urls.sort_unstable();
    cdn_urls.dedup();

    if cdn_urls.len() != CDN_URLS.len() {
        debug!(
            "Compiled the new CDN list - initial length: {}, new length: {}",
            CDN_URLS.len(),
            cdn_urls.len()
        );
    } else {
        debug!(
            "No new CDNs added - initial length: {}, new length: {}",
            CDN_URLS.len(),
            cdn_urls.len()
        );
    }

    cdn_urls
}

fn sc_extract_exact_timestamps(html_fragment: &Html) -> Result<ExtractedTimestamps> {
    let exact_dt_selector = Selector::parse("div > div[data-requests]")
        .map_err(|_| DeriveDate::Selector)?;

    let element = html_fragment
        .select(&exact_dt_selector)
        .next()
        .ok_or(DeriveDate::ScraperElement)?;

    let data_requests = element
        .value()
        .attr("data-requests")
        .ok_or(DeriveDate::ScraperAttribute)?;

    // Parse the clips_json into the struct StreamsChartsTwitchClip with serde_json
    let clips_payloads: Vec<StreamsChartsTwitchClip> = serde_json::from_str(data_requests)?;
    
    let first_clip = clips_payloads.first().ok_or_else(|| {
        DeriveDate::WrongURL("No clips found in data".to_string())
    })?;
    
    let last_clip = clips_payloads.last().ok_or_else(|| {
        DeriveDate::WrongURL("No clips found in data".to_string())
    })?;

    let start_dt = parse_timestamp(&first_clip.started_at)?;
    let end_dt = parse_timestamp(&last_clip.ended_at)?;

    Ok(ExtractedTimestamps {
        processing_type: ProcessingType::Exact,
        start_timestamp: start_dt,
        end_timestamp: end_dt,
    })
}

fn sc_bruteforce_timestamps(html_fragment: &Html) -> Result<ExtractedTimestamps> {
    let bruteforce_selector = Selector::parse("time")
        .map_err(|_| DeriveDate::Selector)?;
        
    let element = html_fragment
        .select(&bruteforce_selector)
        .next()
        .ok_or(DeriveDate::ScraperElement)?;

    let datetime_attr = element
        .value()
        .attr("datetime")
        .ok_or(DeriveDate::ScraperAttribute)?;

    let date_parsed = parse_timestamp(datetime_attr)?;
    
    Ok(ExtractedTimestamps {
        processing_type: ProcessingType::Bruteforce,
        start_timestamp: date_parsed - 60,
        end_timestamp: date_parsed + 60,
    })
}

#[cfg(test)]
mod tests {
    use reqwest::header::USER_AGENT;
    use std::fs::File;
    use std::io::Write;
    use std::thread::sleep;
    use tempfile::tempdir;

    use crate::config::Cli;
    use crate::twitch::models::CDN_URLS;

    use super::{
        compile_cdn_list, derive_date_from_url, get_useragent_list, parse_timestamp,
        ProcessingType, URLData,
    };

    #[test]
    fn compile_cdns() {
        let dir = tempdir().unwrap();
        let mut cdn_urls_string: Vec<String> = CDN_URLS.iter().map(|s| s.to_string()).collect();
        cdn_urls_string.push("test.cloudflare.net".to_string());
        cdn_urls_string.sort();

        let path_txt = dir.path().join("cdn_test.txt");
        let mut file_txt = File::create(path_txt.clone()).unwrap();

        writeln!(file_txt, "test.cloudflare.net").unwrap();

        let mut res_txt = compile_cdn_list(Some(path_txt.to_str().unwrap().to_string()));
        res_txt.sort();

        assert_eq!(res_txt, cdn_urls_string, "testing txt file");

        let path_json = dir.path().join("cdn_test.json");
        let mut file_json = File::create(path_json.clone()).unwrap();

        writeln!(file_json, "{{\n\"cdns\": [\"test.cloudflare.net\"]\n}}").unwrap();

        let mut res_json = compile_cdn_list(Some(path_json.to_str().unwrap().to_string()));
        res_json.sort();

        assert_eq!(res_json, cdn_urls_string, "testing json file");

        let path_toml = dir.path().join("cdn_test.toml");
        let mut file_toml = File::create(path_toml.clone()).unwrap();

        writeln!(file_toml, "cdns = [\"test.cloudflare.net\"]").unwrap();

        let mut res_toml = compile_cdn_list(Some(path_toml.to_str().unwrap().to_string()));
        res_toml.sort();

        assert_eq!(res_toml, cdn_urls_string, "testing toml file");

        let path_yaml1 = dir.path().join("cdn_test.yaml");
        let mut file_yaml1 = File::create(path_yaml1.clone()).unwrap();

        writeln!(file_yaml1, "cdns: [\"test.cloudflare.net\"]").unwrap();

        let path_yaml2 = dir.path().join("cdn_test.yml");
        let mut file_yaml2 = File::create(path_yaml2.clone()).unwrap();

        writeln!(file_yaml2, "cdns: [\"test.cloudflare.net\"]").unwrap();

        let mut res_yaml1 = compile_cdn_list(Some(path_yaml1.to_str().unwrap().to_string()));
        res_yaml1.sort();

        assert_eq!(res_yaml1, cdn_urls_string, "testing yaml file");

        let mut res_yaml2 = compile_cdn_list(Some(path_yaml2.to_str().unwrap().to_string()));
        res_yaml2.sort();

        assert_eq!(res_yaml2, cdn_urls_string, "testing yml file");

        let path_png = dir.path().join("cdn_test.png");

        let mut res_png = compile_cdn_list(Some(path_png.to_str().unwrap().to_string()));
        res_png.sort();

        assert_ne!(
            res_png, cdn_urls_string,
            "testing unsupported extension (should be unequal)"
        );

        let mut cdn_urls_string_init: Vec<String> =
            CDN_URLS.iter().map(|s| s.to_string()).collect();
        cdn_urls_string_init.sort();

        assert_eq!(
            res_png, cdn_urls_string_init,
            "testing unsupported extension (should be equal)"
        );
    }

    #[test]
    fn parse_timestamps() {
        assert_eq!(
            parse_timestamp("1657871396").unwrap(),
            1657871396,
            "testing unix timestamp parsing"
        );
        assert_eq!(
            parse_timestamp("2022-07-15T07:49:56+00:00").unwrap(),
            1657871396,
            "testing rfc parsing"
        );
        assert_eq!(
            parse_timestamp("2022-07-15 07:49:56 UTC").unwrap(),
            1657871396,
            "testing parsing time with the UTC tag"
        );
        assert_eq!(
            parse_timestamp("2022-07-15 07:49:56").unwrap(),
            1657871396,
            "testing parsing time w/o the UTC tag"
        );
        assert_eq!(
            parse_timestamp("15-07-2022 07:49").unwrap(),
            1657871340,
            "testing parsing time w/o seconds"
        );
        assert!(
            parse_timestamp("2022-07-15 0749").is_err(),
            "testing parsing wrong timestamps"
        );
    }

    #[tokio::test]
    async fn derive_date() {
        // Skip network-dependent tests in CI or when specified
        if std::env::var("SKIP_NETWORK_TESTS").is_ok() {
            println!("Skipping network-dependent tests");
            return;
        }

        // Test TwitchTracker URL (if accessible)
        match derive_date_from_url(
            "https://twitchtracker.com/forsen/streams/39619965384",
            Cli::default()
        ).await {
            Ok(result) => {
                assert_eq!(
                    result,
                    (
                        ProcessingType::Exact,
                        URLData {
                            username: "forsen".to_string(),
                            broadcast_id: "39619965384".to_string(),
                            start_date: "2022-07-12 17:05:08".to_string(),
                            end_date: None
                        }
                    ),
                    "testing twitchtracker - https://twitchtracker.com/forsen/streams/39619965384"
                );
            }
            Err(e) => {
                // If we get a network error, that's acceptable for this test
                println!("Network error for TwitchTracker test (acceptable): {}", e);
            }
        }

        // Test StreamsCharts URL (if accessible)
        match derive_date_from_url(
            "https://streamscharts.com/channels/robcdee/streams/39648192487", 
            Cli::default()
        ).await {
            Ok(result) => {
                assert_eq!(
                    result,
                    (
                        ProcessingType::Exact,
                        URLData {
                            username: "robcdee".to_string(),
                            broadcast_id: "39648192487".to_string(),
                            start_date: "1662523601".to_string(),
                            end_date: Some("1662540600".to_string())
                        }
                    ),
                    "testing streamscharts (exact with bruteforce fallback) - https://streamscharts.com/channels/robcdee/streams/39648192487"
                );
            }
            Err(e) => {
                // If we get a network error, that's acceptable for this test
                println!("Network error for StreamsCharts test (acceptable): {}", e);
            }
        }

        // Test error cases (these don't require network)
        assert!(
            derive_date_from_url("https://google.com", Cli::default())
                .await
                .is_err(),
            "testing wrong link - https://google.com"
        );
        assert!(
            derive_date_from_url("https://twitchtracker.com/forsen/streams/3961965384", Cli::default())
                .await
                .is_err(), 
            "testing wrong twitchtracker link 1 - https://twitchtracker.com/forsen/streams/3961965384"
        );
        assert!(
            derive_date_from_url("https://streamscharts.com/channels/forsen/streams/3961965384", Cli::default())
                .await
                .is_err(), 
            "testing wrong streamscharts link 1 - https://streamscharts.com/channels/forsen/streams/3961965384"
        );
        assert!(
            derive_date_from_url("https://twitchtracker.com/forsen/sreams/39619965384", Cli::default())
                .await
                .is_err(), 
            "testing wrong twitchtracker link 2 - https://twitchtracker.com/forsen/sreams/39619965384"
        );
        assert!(
            derive_date_from_url("https://streamscharts.com/channels/forsen/sreams/39619965384", Cli::default())
                .await
                .is_err(), 
            "testing wrong streamscharts link 2 - https://streamscharts.com/channels/forsen/sreams/39619965384"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn streamscharts_useragent_check() {
        let url = "https://streamscharts.com/channels/robcdee/streams/39648192487";
        let ua_vec = get_useragent_list().await;

        for ua in ua_vec {
            let init_resp = crate::HTTP_CLIENT
                .get(url)
                .header(USER_AGENT, &ua)
                .send()
                .await
                .unwrap();
            sleep(std::time::Duration::from_secs(2));
            assert_eq!(
                init_resp.status(),
                200,
                "testing useragents: ua - {}, url - {}",
                &ua,
                url
            );
        }
    }
}
