I've read all 18 files, though I haven't compiled or run anything, so the "bugs" below come from reading the code. The structure is good: there's a lib/bin split, doc comments everywhere, no stray `.unwrap()` in production paths, and known-answer tests for the hash and URL. The main problems are silent failure modes, a few real bugs, and wasted requests.

## P0: Real bugs

1. **`link` probably does nothing for TwitchTracker URLs** (`commands/link.rs`).
   - For `https://twitchtracker.com/<user>/streams/<id>`, `split('/')` puts `"streams"` at `parts[4]`. The `parse::<i64>().unwrap_or(0)` then gives `id = 0`, and the command silently returns `Ok(())`. This assumes the usual TwitchTracker URL shape.
   - The StreamsCharts branch doesn't exist, even though the CLI help advertises it.
   - `url.contains("twitchtracker.com")` is a substring check, so a URL with that text in a query string would also match.
   - Fix: parse with the `url` crate (already in `Cargo.toml` but unused) and match on host and path segments (snippet below). Print a clear error when the ID or timestamp can't be found.

2. **Usernames aren't lowercased.** The hash is `SHA1("{username}_{id}_{ts}")`, so `Destiny` gives a wrong hash and a silent "not found". Normalize to lowercase at the CLI boundary.

3. **`--threads 0` can hang.** `check.rs` guards with `.max(1)`, but `bruteforce.rs` and `clipforce.rs` pass `flags.threads` straight to `buffer_unordered`, which I believe never makes progress at 0. Validate it in clap instead. The same change fixes another issue: `-t`, `-s` and `-p` aren't `global = true`, so `tbf exact -t 50 ...` is rejected and they only work before the subcommand.

4. **No HTTP timeouts, and every error looks like "not found."** The client has no `timeout` or `connect_timeout`, and every `.send().await...unwrap_or(false)` treats timeouts, 429s and DNS failures as "VOD doesn't exist". A rate-limited scan can end with "Could not find any available VODs" and no hint why.
   - Set timeouts, retry 429/5xx/timeouts with backoff, and count failed probes.
   - Report "N requests failed" at the end, and return an error if most probes failed.

5. **TUI issues** (`tui.rs`):
   - Ctrl+C does nothing, because in raw mode it arrives as a key event and only `q` is handled. Esc isn't handled either.
   - You don't check `key.kind == KeyEventKind::Press`. On Windows crossterm also emits release events, so arrow keys can move twice.
   - No panic hook, so a panic leaves the terminal in raw mode. If `EnterAlternateScreen` fails after `enable_raw_mode`, raw mode also stays on. `ratatui::init()`/`restore()` (0.28.1+) handles both.
   - Clicking the border rows selects an item, and a click only selects without activating.

6. **`fix` is very slow and fragile** (`commands/fix.rs`):
   - It does one sequential `HEAD` per segment, so a multi-hour VOD means thousands of round trips. Use `buffered(flags.threads)`; `_flags` is currently unused.
   - It never checks the HTTP status, so a 403 page gets parsed as an m3u8 and gives a confusing error.
   - `absolute.replace("unmuted", "muted")` is applied to the whole URL, not just the filename.
   - `rfind('/').unwrap_or(0)` and manual string concatenation should be `Url::join`.
   - `playlist.segments.clone()` is unnecessary, and the output file is overwritten without warning.

## P1: Performance

- **Probe `chunked` only during bruteforce.** You test 8 CDNs × 7 qualities = 56 requests per timestamp. Source quality (`chunked`) should exist for essentially any VOD, so probe 8 per timestamp and fetch the other qualities only on a hit. That is about 7× fewer requests, and `exact` benefits too. I believe upstream does the same.
- **Stop at the first hit.** Bruteforce keeps scanning the whole range after finding the VOD. Stop by default and add an `--all` flag to keep going. Dropping the stream cancels the remaining futures.
- **Print hits as they're found.** Right now nothing shows until the end.
- **Fix the progress bar.** It doesn't `inc(1)` on hits, so it never reaches 100%. Enable it automatically when stdout is a TTY.
- **Compute the hash once per timestamp.** `playlist_url` recomputes it for each CDN and quality. This is minor, since the network dominates.
- **`exact` runs its 21 offsets sequentially.** That is fine, but it could run them concurrently.

## P2: Structure and testability

- **Commands print directly.** `println!` inside `commands/*` makes them untestable and blocks `--json`. Have them return data (`Vec<VodInfo>`, and so on) and let `main` do the printing. `--simple` only affects `exact` today.
- **Pass an `Options { threads, simple, progress }` struct** instead of the whole `&Cli`.
- **Make base URLs injectable.** The CDN hosts, GQL endpoint, and clip host are hardcoded, so nothing can be tested with `wiremock`, which `AGENTS.md` requires. Put them in a small `Endpoints` struct with production defaults.
- **Fix the error types.** `AppError::{NetworkError, IoError, M3u8Error}` are never constructed. Either use typed errors in `twitch::*` (for example in `fix`, instead of `anyhow!("{error:?}")`) or delete them.
- **Replace the menu's string parsing** (`COMMANDS[selected].split(" -")...`) with an enum. Use `ListState` instead of the manual offset math, and `.min()` instead of the paired `Down` arms.
- **Deduplicate.** The progress-bar setup is copy-pasted in `bruteforce` and `clipforce`. Candidate generation is duplicated between `check_availability` and `bruteforce`.
- **Decide what the TUI is for.** It's currently just a menu followed by plain `stdin` prompts. Either build real input forms or drop `ratatui`/`crossterm` for something like `inquire`.
- **Make the GQL Client-ID overridable** by env var, and check `errors` in the response.
- **Send logs to stderr.** `tracing_subscriber::fmt()` writes to stdout by default, which mixes with results when piping. Use `.with_writer(std::io::stderr)`.
- **Use a non-zero exit code when nothing is found**, so scripts can tell.
- **`parse_timestamp` polish:** document that naive datetimes are treated as UTC, accept millisecond epochs, accept `HH:MM` without seconds, and add a test for the `Z` suffix.
- **Clip URLs:** check whether newer clip URLs use a `vod-` prefix. I can't verify this from here.

## P3: Repo hygiene

From my first plan, these are still all missing:
- README, LICENSE, CI, and `Cargo.toml` metadata
- `[profile.release]`, `rustls` for `reqwest`, and Rust edition 2024
- `url` is unused, and the `chrono` `serde` feature isn't needed
- The user-agent is hardcoded as `"tbf-new/0.1.0"`, so use `concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))`
- `AGENTS.md` is still mostly irrelevant to this project

## Suggested order

1. **Correctness (about a day):** items 1–4 above, plus the TUI key handling.
2. **Efficiency:** `chunked`-first probing, stop at first hit, and concurrent `fix`.
3. **Refactor and tests:** return data instead of printing, injectable endpoints, wiremock tests for `exact`/`bruteforce`/`fix`/`link`, and enum-based menu.
4. **Ship it:** README, CI, license, release binaries.

## Snippets for the top fixes

**Global flags with validation (`cli.rs`):**
```rust
#[arg(short, long, default_value_t = 100, global = true,
      value_parser = clap::value_parser!(u16).range(1..=1000))]
pub threads: u16,
#[arg(short, long, global = true)]
pub simple: bool,
#[arg(short, long, global = true)]
pub progressbar: bool,
```
Use `usize::from(flags.threads)` where you call `buffer_unordered`.

**Client with timeouts (`main.rs`):**
```rust
let client = Client::builder()
    .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(Duration::from_secs(5))
    .timeout(Duration::from_secs(15))
    .build()
    .context("Failed to build HTTP client")?;
```

**Link parsing with the `url` crate (`link.rs`):**
```rust
let parsed = url::Url::parse(url).context("Invalid URL")?;
let host = parsed.host_str().unwrap_or_default().trim_start_matches("www.");
let segments: Vec<&str> = parsed
    .path_segments()
    .map(|s| s.filter(|p| !p.is_empty()).collect())
    .unwrap_or_default();

let target = match (host, segments.as_slice()) {
    ("twitchtracker.com", [user, "streams", id]) => Some((*user, *id)),
    ("streamscharts.com", ["channels", user, "streams", id]) => Some((*user, *id)),
    _ => None,
};
```

**TUI key handling (`tui.rs`):**
```rust
Event::Key(key) if key.kind == KeyEventKind::Press => match (key.code, key.modifiers) {
    (KeyCode::Char('c'), KeyModifiers::CONTROL)
    | (KeyCode::Esc | KeyCode::Char('q'), _) => return Ok(None),
    // ...existing Enter / Up / Down arms