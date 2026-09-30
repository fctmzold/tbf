use crate::twitch::check::VodInfo;

/// Human-readable output goes to the terminal; piped or minimal output
/// carries only URLs on stdout so results stay scriptable.
fn human_output(simple: bool) -> bool {
    !simple && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Print one playlist hit.
///
/// Piped or minimal output prints the bare URL; interactive output aligns
/// the quality label, which stays uncolored for safe copy-paste.
///
/// # Arguments
///
/// * `info` - Found playlist.
/// * `simple` - Minimal-output flag.
pub fn emit_hit(info: &VodInfo, simple: bool) {
    if human_output(simple) {
        let label = console::style(format!("[{:<8}]", info.quality))
            .cyan()
            .bold();
        println!("{label} {}", info.playlist_url);
    } else {
        println!("{}", info.playlist_url);
    }
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
    eprintln!("Play with:    mpv \"{url}\"");
    eprintln!("Download with: yt-dlp \"{url}\"");
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
    fn hit_prints_without_panicking() {
        emit_hit(&hit(), true);
        emit_hit(&hit(), false);
    }

    #[test]
    fn guidance_and_hints_print_without_panicking() {
        suggest_player("https://cdn.example.com/index.m3u8", true);
        suggest_player("https://cdn.example.com/index.m3u8", false);
        hint("try a wider range");
    }
}
