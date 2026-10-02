use crate::twitch::check::VodInfo;

/// Print one stdout line, quitting quietly on a broken pipe.
///
/// `println!` panics on EPIPE, and with `panic = "abort"` that becomes
/// SIGABRT — ugly for a pipe-oriented tool (`tbf vods foo | head -1`).
///
/// # Arguments
///
/// * `line` - Complete line to print.
///
/// # Examples
///
/// ```
/// tbf_new::report::print_line("https://cdn.example.com/index.m3u8");
/// ```
pub fn print_line(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if let Err(error) = writeln!(out, "{line}") {
        if error.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        panic!("failed writing to stdout: {error}");
    }
}

/// Print one stdout line, suspending an active progress bar first.
///
/// # Arguments
///
/// * `bar` - Active progress bar, if any.
/// * `line` - Complete line to print.
///
/// # Examples
///
/// ```
/// tbf_new::report::stdout_line(None, "https://cdn.example.com/index.m3u8");
/// ```
pub fn stdout_line(bar: Option<&indicatif::ProgressBar>, line: &str) {
    match bar {
        Some(bar) => bar.suspend(|| print_line(line)),
        None => print_line(line),
    }
}

/// Print one stderr note without tearing an active progress bar.
///
/// Progress bars draw on stderr, so plain `eprintln!` while one is live
/// produces torn lines and duplicated bars.
///
/// # Arguments
///
/// * `bar` - Active progress bar, if any.
/// * `message` - Message without trailing newline.
///
/// # Examples
///
/// ```
/// tbf_new::report::note(None, "Scanning timestamps...");
/// ```
pub fn note(bar: Option<&indicatif::ProgressBar>, message: impl AsRef<str>) {
    match bar {
        Some(bar) => bar.println(message.as_ref()),
        None => eprintln!("{}", message.as_ref()),
    }
}

/// Human-readable output goes to the terminal; piped or minimal output
/// carries only URLs on stdout so results stay scriptable.
fn human_output(simple: bool) -> bool {
    !simple && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Render one playlist hit for stdout.
///
/// Interactive terminals get the aligned quality label; piped or minimal
/// output carries the bare URL.
///
/// # Arguments
///
/// * `info` - Found playlist.
/// * `simple` - Minimal-output flag.
pub fn format_hit(info: &VodInfo, simple: bool) -> String {
    if human_output(simple) {
        let label = console::style(format!("[{:<8}]", info.quality))
            .cyan()
            .bold();
        format!("{label} {}", info.playlist_url)
    } else {
        info.playlist_url.clone()
    }
}

/// Render one playlist hit as a JSON line.
///
/// The timestamp is included so `bruteforce --all --json` lines can be
/// attributed without parsing the URL.
///
/// # Arguments
///
/// * `info` - Found playlist.
/// * `timestamp` - Unix epoch seconds the hit was found at, if known.
pub fn format_hit_json(info: &VodInfo, timestamp: Option<i64>) -> String {
    serde_json::json!({
        "playlist_url": info.playlist_url,
        "quality": info.quality,
        "timestamp": timestamp,
    })
    .to_string()
}

/// Print one playlist hit.
///
/// # Arguments
///
/// * `bar` - Active progress bar, if any.
/// * `info` - Found playlist.
/// * `simple` - Minimal-output flag.
/// * `json` - Print a JSON line instead of the human format.
/// * `timestamp` - Unix epoch seconds for the JSON line, if known.
pub fn emit_hit(
    bar: Option<&indicatif::ProgressBar>,
    info: &VodInfo,
    simple: bool,
    json: bool,
    timestamp: Option<i64>,
) {
    if json {
        stdout_line(bar, &format_hit_json(info, timestamp));
    } else {
        stdout_line(bar, &format_hit(info, simple));
    }
}

/// Render player guidance lines for the first hit.
///
/// # Arguments
///
/// * `url` - Playlist URL to play or download.
pub fn format_player_hint(url: &str) -> String {
    format!("Play with:    mpv \"{url}\"\nDownload with: yt-dlp \"{url}\"")
}

/// Print player guidance for the first hit.
///
/// # Arguments
///
/// * `url` - Playlist URL to play or download.
/// * `simple` - Minimal-output flag; guidance only shows interactively.
pub fn suggest_player(url: &str, simple: bool) {
    if !human_output(simple) {
        return;
    }
    eprintln!("{}", format_player_hint(url));
}

/// Print a one-line hint to stderr.
///
/// # Arguments
///
/// * `message` - Hint text without the `hint:` prefix.
pub fn hint(message: &str) {
    eprintln!("hint: {message}");
}

/// Print an error chain as `error:` / `caused by:` lines on stderr.
///
/// # Arguments
///
/// * `report` - Error to display.
pub fn print_error(report: &anyhow::Error) {
    eprintln!("error: {report}");
    for cause in report.chain().skip(1) {
        eprintln!("caused by: {cause}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> VodInfo {
        VodInfo {
            playlist_url: "https://cdn.example.com/index.m3u8".to_string(),
            quality: "chunked".to_string(),
        }
    }

    #[test]
    fn hit_formats_bare_url_for_simple_output() {
        assert_eq!(
            format_hit(&hit(), true),
            "https://cdn.example.com/index.m3u8"
        );
    }

    #[test]
    fn hit_json_carries_quality_url_and_timestamp() {
        let line = format_hit_json(&hit(), Some(1_790_752_835));
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(parsed["quality"], "chunked");
        assert_eq!(parsed["playlist_url"], "https://cdn.example.com/index.m3u8");
        assert_eq!(parsed["timestamp"], 1_790_752_835);
    }

    #[test]
    fn hit_json_without_timestamp_is_null() {
        let parsed: serde_json::Value =
            serde_json::from_str(&format_hit_json(&hit(), None)).expect("valid JSON");
        assert!(parsed["timestamp"].is_null());
    }

    #[test]
    fn player_hint_names_player_and_downloader() {
        let hint = format_player_hint("https://cdn.example.com/index.m3u8");
        assert!(hint.contains("mpv"));
        assert!(hint.contains("yt-dlp"));
        assert!(hint.contains("https://cdn.example.com/index.m3u8"));
    }

    #[test]
    fn guidance_and_hints_print_without_panicking() {
        suggest_player("https://cdn.example.com/index.m3u8", true);
        suggest_player("https://cdn.example.com/index.m3u8", false);
        hint("try a wider range");
    }
}
