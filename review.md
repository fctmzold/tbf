I read all 23 files but couldn't compile or run anything, so treat compile-level details as unverified. This is a solid step forward, but a few new problems came in with the new features.

## What's fixed from last time
- Global flags now work after the subcommand, and `--threads 0` is rejected.
- The client has timeouts, and probes are three-state (`Hit`/`Miss`/`Failed`) with retries and failure counts.
- Usernames are lowercased.
- `link` uses proper URL parsing, with tests.
- `fix` runs concurrently, checks HTTP status, uses `Url::join`, and refuses to overwrite the output file.
- The TUI handles Ctrl+C, Esc and key-release events, ignores frame clicks, and installs a panic hook via `ratatui::init()`.
- Logs go to stderr, and there is a shared `progress` module.
- Hashing is now done once per timestamp.

## Bugs and risks to fix

1. **Rate limits are still reported as "not found."**
   - `probe_head` returns `Miss` after retries on 429/5xx, and the test `persistent_server_error_is_miss` locks that in. That is the same failure-masking problem as before, just moved.
   - `vods.rs::resolve_video` has it too: any non-2xx manifest response becomes "No playable playlist found."
   - Fix: after the last retry, return `Failed` for 429 and 5xx. In `resolve_video`, treat 403 as "restricted" and other non-2xx statuses as errors.
   ```rust
   let transient = status.as_u16() == 429 || status.is_server_error();
   if transient {
       if last { return Probe::Failed; }
       tokio::time::sleep(backoff(attempt)).await;
       continue;
   }
   return Probe::Miss;
   ```

2. **The CDN list was cut from 8 to 4 with an unverifiable comment.** The comment in `cdns.rs` says four hosts "stopped answering (verified 2026-09-30)." I can't confirm that. A dead CloudFront hostname still resolves and returns 403 or 404, so a hit-only probe can't prove a host is retired. If that claim is wrong, valid VODs are now unfindable. Check it against a known-good VOD, or keep the old hosts in a `LEGACY_CDNS` list behind a flag. Drop the date from the comment unless you have evidence.

3. **The `Z` branch in `parse_timestamp` is dead code.** chrono's `parse_from_rfc3339` already accepts a `Z` suffix, so the comment is wrong and the new branch never runs. Delete it and keep the test.

4. **Bruteforce prints hits with `println!` while the progress bar is drawing,** which garbles the display. Use `bar.println(...)` when a bar exists, as `vods.rs` already does.

5. **The default of 100 concurrent requests is wrong for the API commands.** `vods` fires up to 100 concurrent GQL and Usher requests with no retry, which invites 429s. Give API-backed commands their own low cap (about 8), and add retries for the POST/GET calls.

6. **`vods` has three smaller problems.**
   - `buffer_unordered` prints videos in random order. Use `.buffered(n)` to keep newest-first.
   - `fetch_all_videos` retries permanent errors (unknown channel, bad persisted-query hash) three times, and it has no guard against a repeating cursor. Use a typed error to skip retries, and add a max-pages or cursor-changed check.
   - Highlights and uploads are mixed in with broadcasts. Add a `--type` flag, as `aha.py` has.

7. **Rust may be blocked where Python wasn't.** `aha.py` uses `curl_cffi` with `impersonate="chrome120"` for the same video-listing query. Plain `reqwest` has a different TLS fingerprint. Test `vods` and `live` against the real API before trusting them. If they fail, this is a likely cause, and you should also expect the persisted-query hash to rot.

8. **`fix` has a race and gives no summary.** `resolve_output` checks `exists()` and then `File::create` runs, which is a race. Use `OpenOptions::new().write(true).create_new(true)` and add a `--force` flag. Also print how many segments were swapped and how many probes failed, since failures currently fall back silently.

9. **`link` may be blocked by the tracker sites.** Sites like TwitchTracker often reject non-browser user agents, and the current status check will now fail cleanly with that. I also can't verify that the StreamsCharts `/channels/<user>/streams/<id>` ID is the Twitch broadcast ID. Test both against live pages.

10. **Small TUI issues.**
    - `ratatui::init()` panics on failure, so use `try_init`.
    - `ratatui::init()` needs ratatui ≥ 0.28.1, so pin it as `"0.28.1"`.
    - The panic hook doesn't disable mouse capture.

## `aha.py` doesn't belong in this repo
It is a different tool: Twitch → JSON → yt-dlp → faster-whisper. It references `transcribe_vod_fasterwhisper.py`, which isn't here, and has no declared dependencies (`curl_cffi`, `yt_dlp`). It also duplicates the Rust video-listing logic, including the same rotating hash and client ID. Move it to its own repo, or delete it. If you keep it, these are real bugs:
- **`is_expired` is applied to every video type.** Twitch retention isn't a flat 60 days. As I understand it, highlights are kept until deleted, and non-partner broadcasts expire much sooner. Verify this, store `broadcastType` from the node, and apply retention per type.
- **`expired` is only computed for freshly fetched videos.** Videos that dropped out of the listing keep a stale `False`, so the script tries to download them. Recompute it for all merged records:
  ```python
  for record in existing_by_id.values():
      record["expired"] = is_expired(record)
  ```
- **`save()` isn't atomic, and a corrupt file means data loss.** Write to a temp file and use `os.replace`. If loading fails, back the file up instead of "Starting fresh," which then overwrites it.
- **Permanent errors are retried for about 100 seconds.** A nonexistent channel raises `ValueError`, but the code still sleeps 10+20+30+40s across retries.
- **There's a history-leaking comment in `start_download`** ("Previously this shelled out..."), which your own `AGENTS.md` forbids.

## Still open from last review
- Still probes 4 CDNs × 7 qualities = 28 requests per timestamp. Probe `chunked` first (4 per timestamp, about 7× fewer), and check other qualities only for timestamps that hit. `chunked` should exist for nearly any video, but audio-only broadcasts may be an exception.
- Bruteforce doesn't stop at the first hit, and `exact` still runs its 21 offsets sequentially.
- Commands print directly instead of returning data. Because of this, and because base URLs are hardcoded, none of `execute` is tested, only pure functions. `wiremock` plus an `Endpoints` struct would fix both.
- The exit code is still 0 when nothing is found.
- `AppError` still has three variants that are never constructed.
- Repo hygiene is unchanged: no README, LICENSE, CI, Cargo metadata or `[profile.release]`, and `AGENTS.md` still describes a different project.
- **New:** a one-day bruteforce range is about 86,400 timestamps × 28 probes ≈ 2.4M requests at defaults. Add a request-count preview and require `--yes` above a threshold.

## Small cleanups
- The GQL endpoint and client ID are duplicated in three places, and `videos.rs` uses a different client ID from `gql.rs`. Consolidate them.
- `urlencoding` is redundant with the `url` crate you already depend on. Use `Url::parse_with_params` or `query_pairs_mut`.
- Parse the master manifest with `m3u8-rs`, which is already a dependency. You then get labelled qualities instead of bare URLs.
- `clipforce` scans `start..end`, so the end is exclusive even though the help text implies inclusive. `start == end` reports "no clips found" after scanning nothing.
- The `Video` fields `duration_seconds`, `view_count` and `game_name` are parsed but never shown.
- `out(&Option<ProgressBar>, …)` should take `Option<&ProgressBar>`.
- `chrono`'s `serde` feature is still unused.

## Suggested order
1. Fix the `Miss`/`Failed` classification (item 1) and verify the CDN list (item 2).
2. Add API-command concurrency caps and retries, then test `vods` and `live` against the real API (items 5 and 7).
3. Do `chunked`-first probing, stop-at-first-hit, and the bruteforce request cap.
4. Move `aha.py` out.
5. Refactor to return data and add wiremock tests, then the README, LICENSE, CI and release setup.
