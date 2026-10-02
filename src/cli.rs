use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::util::{parse_login, parse_timestamp};

/// Parse a timestamp argument at the CLI boundary.
///
/// # Arguments
///
/// * `raw` - Raw argument text.
///
/// # Returns
///
/// Unix epoch seconds.
///
/// # Errors
///
/// Returns the parse failure message for clap to display.
fn parse_cli_timestamp(raw: &str) -> Result<i64, String> {
    parse_timestamp(raw).map_err(|error| error.to_string())
}

/// Options shared by every command.
#[derive(Args, Debug, Clone)]
pub struct GlobalOpts {
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

    /// Offsets scanned around a timestamp by `exact` (and `live`/`link`,
    /// which delegate to it). No effect on other commands.
    #[arg(long, default_value_t = 10, global = true,
          value_parser = clap::value_parser!(u64).range(0..=3600))]
    pub window: u64,

    /// Override the CDN hosts to probe; repeatable. No effect on
    /// `clipforce` (clips host), `vods` or `fix` (fixed endpoints).
    /// Falls back to `TBF_CDNS` (comma-separated) and then the built-in list.
    #[arg(long, global = true)]
    pub cdn: Vec<String>,

    /// Print hits as JSON lines instead of plain URLs.
    ///
    /// Applies to `exact`, `bruteforce`, `clipforce`, `link`, `live`
    /// (through `exact`), and `vods`; `fix` writes a file and is unaffected.
    #[arg(long, global = true)]
    pub json: bool,
}

impl Default for GlobalOpts {
    /// Defaults matching the clap declarations above.
    fn default() -> Self {
        Self {
            threads: 100,
            simple: false,
            progressbar: false,
            window: 10,
            cdn: Vec::new(),
            json: false,
        }
    }
}

/// Resolve CDN hosts from flags and environment, in order.
///
/// Pure for testability; [`GlobalOpts::cdn_hosts`] supplies the environment.
///
/// # Arguments
///
/// * `flags` - Explicit `--cdn` hosts.
/// * `env` - Value of `TBF_CDNS`, if set.
fn resolve_hosts(flags: &[String], env: Option<&str>) -> Vec<String> {
    if !flags.is_empty() {
        return flags
            .iter()
            .map(|host| host.trim().trim_end_matches('/').to_string())
            .filter(|host| !host.is_empty())
            .collect();
    }
    if let Some(env) = env {
        let hosts: Vec<String> = env
            .split(',')
            .map(str::trim)
            .map(|host| host.trim_end_matches('/'))
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect();
        if !hosts.is_empty() {
            return hosts;
        }
    }
    crate::twitch::cdns::DEFAULT_CDNS
        .iter()
        .map(|host| (*host).to_string())
        .collect()
}

impl GlobalOpts {
    /// CDN hosts to probe, in order.
    ///
    /// Explicit `--cdn` flags win, then `TBF_CDNS`, then the built-in list.
    pub fn cdn_hosts(&self) -> Vec<String> {
        resolve_hosts(&self.cdn, std::env::var("TBF_CDNS").ok().as_deref())
    }
}

/// Video kinds accepted by `vods --type`.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoType {
    /// Every video kind.
    All,
    /// Past broadcasts.
    Archive,
    /// Highlights.
    Highlight,
    /// Uploads.
    Upload,
}

impl VideoType {
    /// All variants in menu order.
    pub const ALL: [VideoType; 4] = [
        VideoType::All,
        VideoType::Archive,
        VideoType::Highlight,
        VideoType::Upload,
    ];

    /// Lowercase name for menus and output.
    pub fn label(self) -> &'static str {
        match self {
            VideoType::All => "all",
            VideoType::Archive => "archive",
            VideoType::Highlight => "highlight",
            VideoType::Upload => "upload",
        }
    }

    /// Twitch API filter value, or `None` for unfiltered listings.
    pub fn api_value(self) -> Option<&'static str> {
        match self {
            VideoType::All => None,
            VideoType::Archive => Some("ARCHIVE"),
            VideoType::Highlight => Some("HIGHLIGHT"),
            VideoType::Upload => Some("UPLOAD"),
        }
    }
}

/// Twitch Broadcast Finder command-line arguments.
#[derive(Parser, Debug)]
#[command(author, version, about = "Twitch Broadcast Finder", long_about = None)]
pub struct Cli {
    /// Subcommand to run. When omitted, an interactive TUI is shown.
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Options shared by every command.
    #[command(flatten)]
    pub opts: GlobalOpts,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Check a specific timestamp for a VOD.
    ///
    /// Example: `tbf exact arquel 316969565142 "2026-09-30 10:00"`.
    Exact {
        /// Streamer login name.
        #[arg(value_parser = parse_login)]
        username: String,
        /// VOD/broadcast ID.
        id: u64,
        /// Timestamp: unix epoch, RFC3339, or `YYYY-MM-DD HH:MM[:SS]` (UTC).
        #[arg(value_parser = parse_cli_timestamp)]
        stamp: i64,
    },
    /// Bruteforce a range of timestamps to find a VOD.
    ///
    /// Example: `tbf bruteforce arquel 316969565142 1790752800 1790752900`.
    Bruteforce {
        /// Streamer login name.
        #[arg(value_parser = parse_login)]
        username: String,
        /// VOD/broadcast ID.
        id: u64,
        /// Range start timestamp.
        #[arg(value_parser = parse_cli_timestamp)]
        from: i64,
        /// Range end timestamp.
        #[arg(value_parser = parse_cli_timestamp)]
        to: i64,
        /// Keep scanning after the first hit.
        #[arg(long)]
        all: bool,
        /// Skip the large-range confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Find available clips within a VOD time range.
    Clipforce {
        /// VOD/broadcast ID.
        id: u64,
        /// Start offset in seconds.
        start: i64,
        /// End offset in seconds.
        end: i64,
        /// Skip the large-range confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Extract data from a StreamsCharts stream page URL.
    Link {
        /// StreamsCharts stream page URL.
        url: String,
    },
    /// Get the VOD of a currently live stream.
    Live {
        /// Streamer login name.
        #[arg(value_parser = parse_login)]
        username: String,
    },
    /// List a channel's VODs with playable playlist links.
    Vods {
        /// Channel login name.
        #[arg(value_parser = parse_login)]
        username: String,
        /// Video kind to list.
        #[arg(long = "type", default_value = "all")]
        video_type: VideoType,
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
        let cli =
            Cli::try_parse_from(["tbf", "exact", "-t", "50", "-s", "user", "1", "1605781794"])
                .expect("global flags parse after subcommand");
        assert_eq!(cli.opts.threads, 50);
        assert!(cli.opts.simple);
    }

    #[test]
    fn zero_threads_is_rejected() {
        assert!(Cli::try_parse_from(["tbf", "-t", "0", "live", "user"]).is_err());
    }

    #[test]
    fn username_is_normalized_at_the_boundary() {
        let cli = Cli::try_parse_from(["tbf", "live", "twitch.tv/Arquel/videos"])
            .expect("twitch link parses");
        let Some(Commands::Live { username }) = cli.command else {
            panic!("expected live command");
        };
        assert_eq!(username, "arquel");
    }

    #[test]
    fn bad_username_is_rejected() {
        assert!(Cli::try_parse_from(["tbf", "live", "not a name"]).is_err());
    }

    #[test]
    fn huge_window_is_rejected() {
        assert!(
            Cli::try_parse_from([
                "tbf",
                "exact",
                "user",
                "1",
                "1605781794",
                "--window",
                "99999"
            ])
            .is_err()
        );
    }

    #[test]
    fn video_type_parses() {
        let cli = Cli::try_parse_from(["tbf", "vods", "user", "--type", "archive"])
            .expect("video type parses");
        let Some(Commands::Vods { video_type, .. }) = cli.command else {
            panic!("expected vods command");
        };
        assert_eq!(video_type, VideoType::Archive);
        assert_eq!(video_type.api_value(), Some("ARCHIVE"));
    }

    #[test]
    fn cdn_hosts_prefer_flags_then_env() {
        let opts = GlobalOpts {
            cdn: vec!["flag.example.com".to_string()],
            ..GlobalOpts::default()
        };
        assert_eq!(opts.cdn_hosts(), ["flag.example.com"]);
        // Pure edge cases live in `resolve_hosts_*`; here only assert that
        // the fallback never comes back empty regardless of the shell env.
        let opts = GlobalOpts::default();
        assert!(!opts.cdn_hosts().is_empty());
    }

    #[test]
    fn resolve_hosts_prefers_flags_over_env() {
        let flags = ["flag.example.com".to_string()];
        assert_eq!(
            resolve_hosts(&flags, Some("env.example.com")),
            ["flag.example.com"]
        );
    }

    #[test]
    fn resolve_hosts_parses_env_list() {
        use crate::twitch::cdns::DEFAULT_CDNS;
        assert_eq!(
            resolve_hosts(&[], Some(" a.example.com,, b.example.com ")),
            ["a.example.com", "b.example.com"]
        );
        assert_eq!(resolve_hosts(&[], Some(",,")), resolve_hosts(&[], None));
        assert_eq!(resolve_hosts(&[], None).len(), DEFAULT_CDNS.len());
    }

    #[test]
    fn resolve_hosts_trims_trailing_slashes() {
        assert_eq!(
            resolve_hosts(&["cdn.example.com/".to_string()], None),
            ["cdn.example.com"]
        );
        assert_eq!(
            resolve_hosts(&[], Some("cdn.example.com/")),
            ["cdn.example.com"]
        );
    }
}
