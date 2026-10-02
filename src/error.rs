use thiserror::Error;

/// Application-level errors for the Twitch Broadcast Finder.
#[derive(Error, Debug)]
pub enum AppError {
    /// Timestamp string could not be parsed as Unix, RFC3339, or naive datetime.
    #[error(
        "Invalid timestamp {0:?}: expected unix epoch, RFC3339, or 'YYYY-MM-DD HH:MM[:SS]' (naive times are UTC)"
    )]
    InvalidTimestamp(String),
}
