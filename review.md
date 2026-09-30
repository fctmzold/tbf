I read all 23 files. I haven't compiled or run anything.

## Since last review

Fixed and looking good:
- Persistent 429/5xx now counts as `Failed`, not `Miss`.
- There's a typed retry layer (`Transient`/`Permanent`), API calls are capped at 8 concurrent, and `vods` prints newest-first with 403 treated as "restricted".
- Cursor loops are guarded and `--type` works.
- `--force` uses `create_new`, and `fix` prints a swap summary.
- Hits print above the progress bar without garbling it, and URLs are built with `Url`.
- `aha.py` is gone.

## Still open

1. **`fix` learns the output file exists only after all probing finishes.** `create_output` runs last, so an existing file costs you minutes first. Check up front (snippet below) and keep `create_new` for the race.
2. **`link` may have regressed.** The doc comment says TwitchTracker has no per-stream pages and that things were "verified against live pages." I believe TwitchTracker does have `/<user>/streams/<id>` pages, and the old code supported them. If you removed it because it's blocked (likely Cloudflare), say so in the error message instead. The `div[data-requests]` and `time[datetime]` selectors were written for the old site, so check that they exist on StreamsCharts.
3. **`cdns.rs` still cuts the list from 8 to 4 hosts** with a dated "verified" comment I can't confirm. Dead CloudFront hosts still answer 403/404, so a hit-only probe can't prove a host is retired.
4. **Bruteforce is still slow.** It sends 28 probes per timestamp, keeps going after a hit, and a one-day range means about 2.4M requests with no warning.
5. **`tracing` is effectively dead code.** Nothing emits events except that one `error!`, which prints a timestamped ERROR line with a debug dump of the error chain.
6. **Interactive mode feels unfinished.**
   - It's an alt-screen menu followed by plain `stdin` prompts, and the menu vanishes once you pick something.
   - Nothing is validated until every question is answered.
   - `read_line` treats EOF as an empty answer.
   - `vods` is hardcoded to `all`, and `fix` is hardcoded to no-force.
   - One error ends the session.
7. **Times are silently UTC.** Someone typing local time gets a wrong answer with no hint, and "not found" gives no next step.
8. **Results are bare lines.** They have no colors, no summary, and no "how do I play this" guidance. Status messages also share stdout with results, which breaks piping.

## What I did about UX

The changes are in the files below:
- **Guided menu.** It uses `dialoguer`: arrow-key select with descriptions, per-field validation with re-prompt, and prompts that accept `@name`, a twitch.tv link, or `2026-09-29 18:00` (no seconds). It loops back to the menu after each command and doesn't exit on errors. Because it's not an alt-screen, results stay in your scrollback. This replaces `ratatui`/`crossterm` and shrinks the build.
- **Clean errors.** Errors print as `error: … / caused by: …`, and the exit code is non-zero.
- **Stdout/stderr split.** Status goes to stderr and results go to stdout. When stdout is piped or `-s` is set, only URLs print.
- **Readable results.** Hits show as an aligned quality table plus copy-paste `mpv` / `yt-dlp` commands. "Not found" suggests the next command to try.
- **Clearer input.** `--help` has examples, and usernames and timestamps are validated at parse time with an error listing the accepted formats. Times are echoed back as UTC.
- **Faster bruteforce.** It scans only `chunked` (4 probes per timestamp instead of 28), expands to all qualities on a hit, and stops at the first hit unless `--all`. It asks for confirmation above 20k requests, or takes `--yes`.

These assumptions are unverified:
- `chunked` exists for essentially every video VOD (audio-only broadcasts would be missed by the fast scan).
- The `dialoguer` 0.11 and `console` 0.15 calls are written from memory.