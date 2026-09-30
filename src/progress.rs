use std::io::IsTerminal;

use indicatif::{ProgressBar, ProgressStyle};

/// Build a progress bar for a scanning operation.
///
/// Returns `None` for minimal output. Otherwise the bar shows when
/// explicitly requested or when stdout is interactive.
///
/// # Arguments
///
/// * `total` - Expected number of steps.
/// * `message` - Initial status message.
/// * `simple` - Minimal-output flag suppressing interactive widgets.
/// * `forced` - Explicit `--progressbar` flag.
///
/// # Returns
///
/// Configured progress bar, or `None` when output must stay minimal.
pub fn scanning_progress(
    total: u64,
    message: &str,
    simple: bool,
    forced: bool,
) -> Option<ProgressBar> {
    if simple || !(forced || std::io::stdout().is_terminal()) {
        return None;
    }
    let bar = ProgressBar::new(total);
    let style = ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("##-");
    bar.set_style(style);
    bar.set_message(message.to_string());
    Some(bar)
}

/// Print one message without breaking an active progress bar.
///
/// # Arguments
///
/// * `bar` - Active progress bar, if any.
/// * `message` - Complete message printed as one block.
pub fn emit(bar: Option<&ProgressBar>, message: String) {
    match bar {
        Some(bar) => bar.println(message),
        None => println!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_bar_shows() {
        assert!(scanning_progress(10, "msg", false, true).is_some());
    }

    #[test]
    fn simple_output_hides_bar() {
        assert!(scanning_progress(10, "msg", true, true).is_none());
    }
}
