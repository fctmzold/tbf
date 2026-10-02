use chrono::{DateTime, NaiveDateTime};

use crate::error::AppError;

/// Smallest plausible VOD timestamp (2001-09-09); smaller integers are
/// almost certainly a typo like `YYYYMMDD`, which would otherwise parse as
/// a 1970 epoch.
const MIN_PLAUSIBLE_TIMESTAMP: i64 = 1_000_000_000;
/// Largest plausible VOD timestamp (2100-01-01); larger values are typos or
/// un-normalized sub-second units.
const MAX_PLAUSIBLE_TIMESTAMP: i64 = 4_102_444_800;

/// Parse a timestamp string into a Unix epoch.
///
/// The integer's digit count decides its unit: 10 digits are seconds,
/// 13 are milliseconds, 16 are microseconds, and 19 are nanoseconds (common
/// copy-paste slips), all divided down to seconds. Anything else, values
/// below 2001, and values past 2100 are rejected as typos.
///
/// # Arguments
///
/// * `stamp` - Unix integer (or milliseconds), RFC3339 (`Z` suffix
///   accepted), or `%Y-%m-%d %H:%M:%S` string. Naive datetimes are UTC.
///   The `T` separator (`%Y-%m-%dT%H:%M[:SS]`) is accepted too.
///
/// # Returns
///
/// Unix epoch seconds.
///
/// # Errors
///
/// Returns `AppError::InvalidTimestamp` when no format matches.
///
/// # Examples
///
/// ```
/// use tbf_new::util::parse_timestamp;
///
/// assert_eq!(parse_timestamp("1605781794").unwrap(), 1605781794);
/// ```
pub fn parse_timestamp(stamp: &str) -> Result<i64, AppError> {
    let stamp = stamp.trim();
    if let Ok(timestamp) = stamp.parse::<i64>() {
        // Digit count decides the unit; the sign is not a digit.
        let digits = stamp.trim_start_matches(['+', '-']).len();
        let seconds = match digits {
            10 => timestamp,
            13 => timestamp / 1000,
            16 => timestamp / 1_000_000,
            19 => timestamp / 1_000_000_000,
            _ => return Err(AppError::InvalidTimestamp(stamp.to_string())),
        };
        if !(MIN_PLAUSIBLE_TIMESTAMP..=MAX_PLAUSIBLE_TIMESTAMP).contains(&seconds) {
            return Err(AppError::InvalidTimestamp(stamp.to_string()));
        }
        return Ok(seconds);
    }

    if let Ok(datetime) = DateTime::parse_from_rfc3339(stamp) {
        return Ok(datetime.timestamp());
    }

    for format in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(stamp, format) {
            return Ok(naive.and_utc().timestamp());
        }
    }

    Err(AppError::InvalidTimestamp(stamp.to_string()))
}

/// Format a Unix epoch as RFC3339 UTC for echoing parsed input.
///
/// # Arguments
///
/// * `timestamp` - Unix epoch seconds.
///
/// # Returns
///
/// RFC3339 string, or the raw number when out of range.
pub fn format_utc(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|moment| moment.to_rfc3339())
        .unwrap_or_else(|| timestamp.to_string())
}

/// Count timestamps in an inclusive range without overflow.
///
/// Plain `to - from + 1` overflows `i64` for wide ranges, and collecting
/// the range first allocates before any size guard can refuse it. Callers
/// check this count before iterating lazily.
///
/// # Arguments
///
/// * `from` - Range start.
/// * `to` - Range end.
///
/// # Returns
///
/// Timestamp count, saturating at `u64::MAX`; `0` when reversed.
pub fn range_len(from: i64, to: i64) -> u64 {
    (to as i128 - from as i128 + 1).clamp(0, u64::MAX as i128) as u64
}

/// Reduce `@name` and `twitch.tv` links to a login name.
///
/// Multi-segment paths (`twitch.tv/<name>/videos`) reduce to their first
/// segment.
///
/// # Arguments
///
/// * `raw` - Raw username field.
///
/// # Returns
///
/// Lowercase candidate, possibly empty when nothing usable was entered.
pub fn normalize_login(raw: &str) -> String {
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
            if let Some(name) = segments.first() {
                return name.to_lowercase();
            }
        }
    }
    trimmed.to_lowercase()
}

/// Check a Twitch login name: `[a-z0-9_]{1,25}`.
///
/// # Arguments
///
/// * `login` - Candidate login name.
pub fn is_valid_login(login: &str) -> bool {
    const MAX_LOGIN_LEN: usize = 25;
    !login.is_empty()
        && login.len() <= MAX_LOGIN_LEN
        && login
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

/// First path segments on twitch.tv that are site routes, not logins.
const RESERVED_LOGIN_PATHS: &[&str] = &[
    "videos",
    "directory",
    "downloads",
    "settings",
    "subscriptions",
    "wallet",
    "p",
    "u",
    "popout",
    "prime",
    "jobs",
];

/// Parse a username argument at the CLI boundary.
///
/// Accepts `@name` and `twitch.tv` links, then enforces the Twitch login
/// shape so typos fail fast instead of probing a nonsense channel.
///
/// # Arguments
///
/// * `raw` - Raw argument text.
///
/// # Returns
///
/// Normalized login name.
///
/// # Errors
///
/// Returns a message for clap to display when the login is invalid.
pub fn parse_login(raw: &str) -> Result<String, String> {
    let login = normalize_login(raw);
    if RESERVED_LOGIN_PATHS.contains(&login.as_str()) {
        return Err(format!(
            "invalid Twitch login {raw:?}: {login:?} is a site route, not a channel"
        ));
    }
    if is_valid_login(&login) {
        Ok(login)
    } else {
        Err(format!(
            "invalid Twitch login {raw:?}: expected 1-25 lowercase letters, digits, or '_'"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_timestamp() {
        assert_eq!(parse_timestamp("1605781794").unwrap(), 1_605_781_794);
    }

    #[test]
    fn parses_rfc3339_timestamp() {
        assert_eq!(
            parse_timestamp("2020-11-19T04:29:54+00:00").unwrap(),
            1_605_760_194
        );
    }

    #[test]
    fn parses_naive_datetime() {
        assert_eq!(
            parse_timestamp("2020-11-19 04:29:54").unwrap(),
            1_605_760_194
        );
    }

    #[test]
    fn parses_datetime_without_seconds() {
        assert_eq!(parse_timestamp("2020-11-19 04:29").unwrap(), 1_605_760_140);
    }

    #[test]
    fn formats_utc_echo() {
        assert_eq!(format_utc(1_605_760_194), "2020-11-19T04:29:54+00:00");
    }

    #[test]
    fn parses_rfc3339_zulu_timestamp() {
        assert_eq!(
            parse_timestamp("2020-11-19T04:29:54Z").unwrap(),
            1_605_760_194
        );
    }

    #[test]
    fn rejects_invalid_timestamp() {
        assert!(parse_timestamp("not-a-date").is_err());
    }

    #[test]
    fn parses_t_separated_datetime() {
        assert_eq!(
            parse_timestamp("2020-11-19T04:29:54").unwrap(),
            1_605_760_194
        );
        assert_eq!(parse_timestamp("2020-11-19T04:29").unwrap(), 1_605_760_140);
    }

    #[test]
    fn parses_millisecond_timestamp() {
        assert_eq!(parse_timestamp("1605781794000").unwrap(), 1_605_781_794);
    }

    #[test]
    fn parses_microsecond_timestamp() {
        assert_eq!(parse_timestamp("1605781794000000").unwrap(), 1_605_781_794);
    }

    #[test]
    fn parses_nanosecond_timestamp() {
        assert_eq!(
            parse_timestamp("1605781794000000000").unwrap(),
            1_605_781_794
        );
    }

    #[test]
    fn rejects_ambiguous_digit_counts() {
        assert!(parse_timestamp("16057817940000").is_err());
        assert!(parse_timestamp("160578179").is_err());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(parse_timestamp("  1605781794  ").unwrap(), 1_605_781_794);
    }

    #[test]
    fn negative_looking_long_values_are_rejected() {
        assert!(parse_timestamp("-123456789012").is_err());
    }

    #[test]
    fn rejects_implausibly_small_timestamp() {
        assert!(parse_timestamp("20260930").is_err());
        assert!(parse_timestamp("0").is_err());
    }

    #[test]
    fn rejects_far_future_timestamp() {
        assert!(parse_timestamp("9999999999").is_err());
    }

    #[test]
    fn range_len_counts_inclusive_bounds() {
        assert_eq!(range_len(5, 10), 6);
        assert_eq!(range_len(7, 7), 1);
    }

    #[test]
    fn range_len_is_zero_when_reversed() {
        assert_eq!(range_len(10, 5), 0);
    }

    #[test]
    fn range_len_saturates_full_i64_range() {
        assert_eq!(range_len(i64::MIN, i64::MAX), u64::MAX);
    }

    #[test]
    fn login_strips_at_prefix() {
        assert_eq!(normalize_login("@Arquel"), "arquel");
    }

    #[test]
    fn login_extracts_twitch_link() {
        assert_eq!(normalize_login("https://www.twitch.tv/Arquel"), "arquel");
        assert_eq!(normalize_login("twitch.tv/arquel"), "arquel");
    }

    #[test]
    fn login_reduces_multi_segment_path() {
        assert_eq!(normalize_login("twitch.tv/arquel/videos"), "arquel");
    }

    #[test]
    fn login_validation_rejects_bad_shapes() {
        assert!(is_valid_login("arquel"));
        assert!(is_valid_login("xqc_123"));
        assert!(!is_valid_login(""));
        assert!(!is_valid_login("HasUpper"));
        assert!(!is_valid_login("has-dash"));
        assert!(!is_valid_login(&"a".repeat(26)));
        assert!(parse_login("twitch.tv/Arquel/videos").is_ok());
        assert!(parse_login("not a name").is_err());
    }

    #[test]
    fn login_rejects_reserved_site_routes() {
        assert!(parse_login("twitch.tv/videos/123").is_err());
        assert!(parse_login("twitch.tv/directory").is_err());
        assert!(parse_login("videos").is_err());
    }
}
