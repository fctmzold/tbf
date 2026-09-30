use clap::{Parser, Subcommand};

/// Twitch Broadcast Finder command-line arguments.
#[derive(Parser, Debug)]
#[command(author, version, about = "Twitch Broadcast Finder", long_about = None)]
pub struct Cli {
    /// Subcommand to run. When omitted, an interactive TUI is shown.
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Amount of concurrent requests to use for scanning operations.
    #[arg(short, long, default_value_t = 100)]
    pub threads: usize,

    /// Provide minimal output.
    #[arg(short, long)]
    pub simple: bool,

    /// Enable a progress bar for long-running operations.
    #[arg(short, long)]
    pub progressbar: bool,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Check a specific timestamp for a VOD.
    Exact {
        /// Streamer login name.
        username: String,
        /// VOD/broadcast ID.
        id: i64,
        /// Timestamp as Unix epoch, RFC3339, or YYYY-MM-DD HH:MM:SS.
        stamp: String,
    },
    /// Bruteforce a range of timestamps to find a VOD.
    Bruteforce {
        /// Streamer login name.
        username: String,
        /// VOD/broadcast ID.
        id: i64,
        /// Range start timestamp.
        from: String,
        /// Range end timestamp.
        to: String,
    },
    /// Find available clips within a VOD time range.
    Clipforce {
        /// VOD/broadcast ID.
        id: i64,
        /// Start offset in seconds.
        start: i64,
        /// End offset in seconds.
        end: i64,
    },
    /// Extract data from a TwitchTracker or StreamsCharts URL.
    Link {
        /// TwitchTracker or StreamsCharts URL.
        url: String,
    },
    /// Get the VOD of a currently live stream.
    Live {
        /// Streamer login name.
        username: String,
    },
    /// Fix an unplayable unmuted VOD playlist.
    Fix {
        /// Twitch VOD m3u8 playlist URL.
        url: String,
        /// Output file path. Defaults to fixed_playlist.m3u8.
        output: Option<String>,
    },
}
