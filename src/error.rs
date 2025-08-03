use reqwest::header::{InvalidHeaderName, InvalidHeaderValue};
use std::{error::Error, fmt::Display, num::ParseIntError};
use time::error::Parse;
use url::ParseError as UrlPError;

#[derive(Debug)]
pub enum PlaylistFix {
    Reqwest(reqwest::Error),
    Io(std::io::Error),
    URL,
}

impl From<reqwest::Error> for PlaylistFix {
    fn from(e: reqwest::Error) -> Self {
        Self::Reqwest(e)
    }
}

impl From<std::io::Error> for PlaylistFix {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Display for PlaylistFix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reqwest(e) => write!(f, "couldn't process the url: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::URL => write!(f, "only twitch.tv and cloudfront.net URLs are supported"),
        }
    }
}

impl Error for PlaylistFix {}

#[derive(Debug)]
pub enum Vod {
    IntegerParse(ParseIntError),
    StringParse(Parse),
    HeaderName(InvalidHeaderName),
    HeaderValue(InvalidHeaderValue),
    UrlProcess(reqwest::Error),
}

impl From<InvalidHeaderName> for Vod {
    fn from(e: InvalidHeaderName) -> Self {
        Self::HeaderName(e)
    }
}

impl From<InvalidHeaderValue> for Vod {
    fn from(e: InvalidHeaderValue) -> Self {
        Self::HeaderValue(e)
    }
}

impl From<reqwest::Error> for Vod {
    fn from(e: reqwest::Error) -> Self {
        Self::UrlProcess(e)
    }
}

impl From<ParseIntError> for Vod {
    fn from(e: ParseIntError) -> Self {
        Self::IntegerParse(e)
    }
}

impl From<Parse> for Vod {
    fn from(e: Parse) -> Self {
        Self::StringParse(e)
    }
}

impl Display for Vod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IntegerParse(e) => write!(f, "couldn't parse the unix timestamp: {e}"),
            Self::StringParse(e) => write!(f, "couldn't parse the string timestamp: {e}"),
            Self::HeaderName(e) => write!(f, "invalid header name: {e}"),
            Self::HeaderValue(e) => write!(f, "invalid header value: {e}"),
            Self::UrlProcess(e) => write!(f, "couldn't process the url: {e}"),
        }
    }
}

impl Error for Vod {}

#[derive(Debug)]
pub enum DeriveDate {
    SegmentMap,
    ScraperElement,
    ScraperAttribute,
    Selector,
    TimestampParser(Vod),
    UrlProcess(reqwest::Error),
    UrlParse(UrlPError),
    WrongURL(String),
}

impl From<Vod> for DeriveDate {
    fn from(e: Vod) -> Self {
        Self::TimestampParser(e)
    }
}

impl From<UrlPError> for DeriveDate {
    fn from(e: UrlPError) -> Self {
        Self::UrlParse(e)
    }
}

impl From<reqwest::Error> for DeriveDate {
    fn from(e: reqwest::Error) -> Self {
        Self::UrlProcess(e)
    }
}

impl Display for DeriveDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SegmentMap => write!(f, "couldn't map the URL segments"),
            Self::ScraperElement => write!(f, "couldn't find the nth html element"),
            Self::ScraperAttribute => write!(f, "couldn't find the html attribute"),
            Self::Selector => write!(f, "couldn't parse the selector"),
            Self::TimestampParser(e) => write!(f, "{e}"),
            Self::UrlProcess(e) => write!(f, "couldn't process the url: {e}"),
            Self::WrongURL(e) => write!(f, "{e}"),
            Self::UrlParse(e) => write!(f, "couldn't parse the url: {e}"),
        }
    }
}

impl Error for DeriveDate {}

#[derive(Debug)]
pub enum Clip {
    IntegerParse(ParseIntError),
    SegmentMap,
    HeaderName(InvalidHeaderName),
    HeaderValue(InvalidHeaderValue),
    WrongURL(String),
    UrlProcess(reqwest::Error),
}

impl From<ParseIntError> for Clip {
    fn from(e: ParseIntError) -> Self {
        Self::IntegerParse(e)
    }
}

impl From<InvalidHeaderName> for Clip {
    fn from(e: InvalidHeaderName) -> Self {
        Self::HeaderName(e)
    }
}

impl From<InvalidHeaderValue> for Clip {
    fn from(e: InvalidHeaderValue) -> Self {
        Self::HeaderValue(e)
    }
}

impl From<reqwest::Error> for Clip {
    fn from(e: reqwest::Error) -> Self {
        Self::UrlProcess(e)
    }
}

impl Display for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IntegerParse(e) => write!(f, "couldn't parse the broadcast id: {e}"),
            Self::SegmentMap => write!(f, "couldn't map the URL segments"),
            Self::HeaderName(e) => write!(f, "invalid header name: {e}"),
            Self::HeaderValue(e) => write!(f, "invalid header value: {e}"),
            Self::WrongURL(e) => write!(f, "{e}"),
            Self::UrlProcess(e) => write!(f, "couldn't process the url: {e}"),
        }
    }
}

impl Error for Clip {}
