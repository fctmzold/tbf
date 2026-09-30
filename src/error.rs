use thiserror::Error;

/// Application-level errors for the Twitch Broadcast Finder.
#[derive(Error, Debug)]
pub enum AppError {
    /// Timestamp string could not be parsed as Unix, RFC3339, or naive datetime.
    #[error("Invalid timestamp {0:?}: expected unix epoch, RFC3339, or 'YYYY-MM-DD HH:MM[:SS]' (naive times are UTC)")]
    InvalidTimestamp(String),

    /// Network request via reqwest failed.
    #[error("Network request failed: {0}")]
    NetworkError(#[from] reqwest::Error),

    /// Filesystem IO operation failed.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// M3U8 playlist could not be parsed or had an unexpected shape.
    #[error("M3U8 parsing error: {0}")]
    M3u8Error(String),
}
