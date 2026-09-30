use chrono::{DateTime, NaiveDateTime};

use crate::error::AppError;

/// Parse a timestamp string into a Unix epoch.
///
/// # Arguments
///
/// * `stamp` - Unix integer, RFC3339, or `%Y-%m-%d %H:%M:%S` string.
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
    if let Ok(timestamp) = stamp.parse::<i64>() {
        return Ok(timestamp);
    }

    if let Ok(datetime) = DateTime::parse_from_rfc3339(stamp) {
        return Ok(datetime.timestamp());
    }

    if let Ok(naive) = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S") {
        return Ok(naive.and_utc().timestamp());
    }

    Err(AppError::InvalidTimestamp(stamp.to_string()))
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
    fn rejects_invalid_timestamp() {
        assert!(parse_timestamp("not-a-date").is_err());
    }
}
