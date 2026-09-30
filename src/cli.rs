use clap::{Parser, Subcommand};

/// Twitch Broadcast Finder command-line arguments.
#[derive(Parser, Debug)]
#[command(author, version, about = "Twitch Broadcast Finder", long_about = None)]
pub struct Cli {
    /// Subcommand to run. When omitted, an interactive TUI is shown.
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Amount of concurrent requests to use for scanning operations.
    #[arg(short, long, default_value_t = 100, global = true,
          value_parser = clap::value_parser!(u16).range(1..=1000))]
    pub threads: u16,

    /// Provide minimal output.
    #[arg(short, long, global = true)]
    pub simple: bool,

    /// Enable a progress bar for long-running operations.
    #[arg(short, long, global = true)]
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
    /// Extract data from a StreamsCharts stream page URL.
    Link {
        /// StreamsCharts stream page URL.
        url: String,
    },
    /// Get the VOD of a currently live stream.
    Live {
        /// Streamer login name.
        username: String,
    },
    /// List a channel's VODs with playable playlist links.
    Vods {
        /// Channel login name.
        username: String,
        /// Video kind: all, archive, highlight, or upload.
        #[arg(long = "type", default_value = "all", value_parser = ["all", "archive", "highlight", "upload"])]
        video_type: String,
    },
    /// Fix an unplayable unmuted VOD playlist.
    Fix {
        /// Twitch VOD m3u8 playlist URL.
        url: String,
        /// Output file path. Defaults to fixed_playlist.m3u8.
        output: Option<String>,
        /// Overwrite the output file when it exists.
        #[arg(long)]
        force: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn flags_work_after_subcommand() {
        let cli = Cli::try_parse_from(["tbf", "exact", "-t", "50", "-s", "user", "1", "2"])
            .expect("global flags parse after subcommand");
        assert_eq!(cli.threads, 50);
        assert!(cli.simple);
    }

    #[test]
    fn zero_threads_is_rejected() {
        assert!(Cli::try_parse_from(["tbf", "-t", "0", "live", "user"]).is_err());
    }
}
