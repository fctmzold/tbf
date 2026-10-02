# AGENTS.md

Guidelines for AI coding agents and contributors working on `tbf-new`, a CLI that finds
hidden Twitch VOD playlists by probing timestamp-derived URLs.

Prioritize clarity, readability, and maintainability over cleverness. Leave no technical debt:
no code beyond what the problem needs.

## Project Invariants

These are the rules most likely to be broken by well-meaning changes. Read them first.

### Output contract

- stdout carries results only: URLs, or one JSON object per line with `--json`.
- stderr carries everything else: progress text, warnings, hints, errors.
- `--json` output must never contain human-readable text on stdout, including error lines.
- Use `eprintln!`/`println!` and `report::print_error` in this binary. Do NOT replace them with
  `tracing`/`log`; the stdout/stderr split is the interface.
- While an `indicatif` bar is active, print through `bar.println(..)` or `bar.suspend(..)`.
  Never use bare `println!`/`eprintln!`, which tears the bar.
- Result blocks (anything a user would pipe) print via `stdout_line`; `note` is for stderr
  diagnostics only.

### Exit codes

- `0` found, `1` clean miss, `2` error (`Outcome::Found`, `Outcome::NotFound`, `Err`).
- A network outage is an error (2), never a miss (1). If every probe failed, return an error.
  This applies to `exact`, `bruteforce`, `clipforce`, and `vods`.
- New commands must return `Result<Outcome>` and follow the same rule.

### Concurrency

- `Prober` (`twitch/check.rs`) owns the single global request budget via its semaphore. It is the
  only thing that bounds in-flight requests, so `--threads` is honored.
- NEVER add a `concurrency` parameter to a scan function or nest `buffer_unordered` windows to
  control request rate. Outer `buffered(n)` windows only bound task count.
- Never hold a semaphore permit across a sleep (`Retry-After`, backoff).

### Ordering

- Wherever order is observable (hits, listings), use `buffered`, not `buffer_unordered`, or sort
  afterward.
- Sort numerically, never by string (`offset-10` must come after `offset-9`).
- Early-exit scans (`scan_first`, `exact`) rely on ordered `buffered` and drop the stream on the
  first hit to cancel in-flight work.

### Input validation

- Validate at the clap boundary with a `value_parser` (`parse_login`, `parse_cli_timestamp`,
  range-limited integers). New numeric flags need an explicit range.
- Any scan size is computed with `range_len` BEFORE iterating. Never `collect()` a user-sized
  range, and never compute `to - from + 1` in `i64`.
- Any command whose request count depends on a user-supplied number (range, window) computes
  an estimate first and refuses above `CONFIRM_THRESHOLD` unless confirmed.
- Scans above `CONFIRM_THRESHOLD` need `--yes` on the CLI or a confirm prompt in interactive mode.

### Tests

- Tests NEVER contact real Twitch, CloudFront, or StreamsCharts. Use loopback `TcpListener`
  servers (see `serve` in `twitch/check.rs`) and `Prober::with_hosts` to point at them.
- Use `tempfile` for filesystem tests. Do not use fixed filenames in the temp dir.
- Use `tokio::time::pause()` for anything that sleeps for retries.
- Do not add a mocking framework. Loopback servers are the project's mocking strategy.
- Tests may use `unwrap`/`expect`.
- NEVER run `bruteforce`, `clipforce`, or `vods` against live endpoints as a check. Use tests.

### Operational notes

- `DEFAULT_CDNS` goes stale. When a host stops answering, remove it and update the verification
  date in the doc comment. Users can override hosts with `--cdn` or `TBF_CDNS`.
- `VIDEO_TOWER_HASH` is a Twitch persisted-query hash that rotates. A `PersistedQueryNotFound`
  error means it needs updating.
- The two client IDs (`GQL_CLIENT_ID`, `PERSISTED_QUERY_CLIENT_ID`) are public and intentionally
  different. They are not secrets and must not be merged or moved to `.env`.
- Interactive mode uses `dialoguer` prompts, not a TUI. Do not introduce `ratatui`/`crossterm`.
- Every global flag's help text must say which commands it affects (for example, `--cdn` does not
  apply to `clipforce`, `vods`, or `fix`).

## Core Principles

- Prefer simple, straightforward solutions over clever or abstract ones.
- Use meaningful names and small, single-purpose functions. Follow DRY.
- Avoid premature optimization. Add parallelism or new crates only for a measured need.
- Keep dependencies minimal: prefer `std` unless a small, well-maintained crate clearly reduces
  complexity. Declare every Tokio feature you use explicitly, and keep test-only features
  (`test-util`) in `[dev-dependencies]`.

## Preferred Tools

- `cargo` for building, testing, and dependency management.
- `clap` (derive) for the CLI, `dialoguer` for interactive prompts, `console` for styling.
- `indicatif` for progress bars. Messages must be contextual (for example "Scanning timestamps...").
- `serde` with `serde_json` for JSON.
- `tokio` for async, `reqwest` for HTTP, `futures` streams for fan-out.
- `thiserror` for error types in library code, `anyhow` for application-level errors with
  `.context()`.

## Code Style and Formatting

- Follow the Rust API Guidelines and idiomatic Rust. Use `rustfmt` defaults (4 spaces, 100 columns).
- snake_case for functions, variables, modules; PascalCase for types and traits;
  SCREAMING_SNAKE_CASE for constants.
- Use meaningful, descriptive names.
- NEVER use emoji or emoji-like unicode (checkmarks, crosses) in code, output, or docs. The only
  exception is tests that verify multibyte handling.
- Assume the reader is a Python expert and a Rust novice. Comment Rust-specific nuance where it is
  not obvious to that reader: ownership and borrowing decisions, `Arc`/`clone` costs, lifetimes,
  `?` propagation, trait bounds, and async cancellation on drop.
- Do NOT comment what a line plainly does, restate a function name, describe what the file
  contains, or reference the original prompt or task.
- Keep comments up to date with code changes.

## Documentation

- Every public function, struct, enum, and method needs a doc comment documenting arguments,
  return value, and errors.
- Include a runnable example for non-trivial public functions. Examples must compile as doctests
  (no `?` outside a `Result` function, no undefined items).

Example:

````rust
/// Count timestamps in an inclusive range without overflow.
///
/// # Arguments
///
/// * `from` - Range start.
/// * `to` - Range end.
///
/// # Returns
///
/// Timestamp count, saturating at `u64::MAX`; `0` when the range is reversed.
///
/// # Examples
///
/// ```
/// use tbf_new::util::range_len;
///
/// assert_eq!(range_len(5, 10), 6);
/// assert_eq!(range_len(10, 5), 0);
/// ```
pub fn range_len(from: i64, to: i64) -> u64 {
    (to as i128 - from as i128 + 1).clamp(0, u64::MAX as i128) as u64
}
````

## Type System and Error Handling

- Leverage the type system: prefer enums (`VideoType`, `Menu`, `Outcome`) over strings and
  sentinel values; prefer `Option<T>` over magic values.
- Use newtypes to separate semantically different values of the same underlying type when
  mix-ups are plausible.
- NEVER use `.unwrap()` in production code paths. Use `.expect("reason")` only for invariant
  violations (for example static selectors).
- Use `Result<T, E>` for fallible operations and propagate with `?`.
- Add context with `.context()`. Error messages should say what failed and, where possible,
  what to do next.
- Classify retryable failures with `Failure::{Transient, Permanent}`; do not retry permanent ones.
- A 429 without `Retry-After` is `Failure::Transient` (backoff applies); only a served wait
  is `Failure::RateLimited` (retry at once).

## Function and Type Design

- Keep functions and types focused on a single responsibility.
- Prefer borrowing (`&T`, `&mut T`) over ownership.
- Limit function parameters to 5. Beyond that, use a struct (see `BruteforceTarget`, `ScanCtx`).
  `clipforce::execute` currently exceeds this and should get a small target struct.
- Return early to reduce nesting. Prefer iterators and combinators where clearer than loops.
- Derive `Debug`, `Clone`, `PartialEq` where appropriate; use `#[derive(Default)]` or an
  `impl Default` when a sensible default exists.
- Fields are private by default with accessors, EXCEPT plain data carriers (`GlobalOpts`, `VodInfo`,
  `ScanOutcome`, `Video`, `*Target` structs), which keep public fields.

## Rust Best Practices

- NEVER use `unsafe` unless absolutely necessary; document safety invariants if used.
- Call `.clone()` explicitly; avoid hidden clones in closures and iterator chains.
- Match exhaustively; avoid catch-all `_` arms when the variants are known.
- Avoid wildcard imports except `use super::*` in test modules and preludes.
- Organize imports: standard library, external crates, local modules.
- Use `enumerate()` instead of manual counters; prefer `if let` / `while let` for single patterns.
- Avoid unnecessary allocations: prefer `&str` over `String`, and `Cow<'_, str>` when ownership
  is conditional. Use `Vec::with_capacity` when the size is known.

## Testing

- Write unit tests for all new functions and types, following Arrange-Act-Assert.
- Prefer testing pure functions (`resolve_hosts`, `format_hit`, `parse_videos_page`) over
  asserting on printed output. Refactor printing code into a `format_*` function plus a thin
  print wrapper.
- Use `#[cfg(test)]` modules. Never commit commented-out tests.
- Reuse the project's test conventions listed under "Project Invariants".

## Security

- No secrets, API keys, or passwords in code. Real secrets go in `.env`, which must be in
  `.gitignore`. Public identifiers (the Twitch client IDs) are exempt, see "Operational notes".
- Never log sensitive information.

## Version Control

- Write clear, descriptive commit messages.
- Never commit commented-out code, `dbg!`, or debug `println!` statements.
- Never commit credentials.

## Tools and Checks

- `rustfmt` for formatting; `clippy` for linting. Code must compile with no warnings. Use
  `-D warnings` in CI, not `#![deny(warnings)]` in source.
- Do not read `Cargo.lock` unless it is directly relevant; it is large.

## Before Committing

- [ ] `cargo test` passes
- [ ] `cargo build` has no warnings
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] All public items have doc comments; doc examples compile
- [ ] stdout carries only results; exit codes follow 0 / 1 / 2
- [ ] New scans use `range_len` guards and the shared `Prober`
- [ ] No test touches the real network
- [ ] No commented-out code, debug statements, or hardcoded credentials

---

**Remember:** Prioritize clarity and maintainability over cleverness. When a generic rule
conflicts with a Project Invariant, the invariant wins.
