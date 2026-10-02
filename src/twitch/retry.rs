use std::time::Duration;

use anyhow::Result;

/// Whether a failed operation is worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// May succeed later (timeout, 429, 5xx, reset connection).
    Transient,
    /// Rate limited, but the wait was already served: retry immediately.
    RateLimited,
    /// Will never succeed (request could not even be built, redirect loop).
    Permanent,
}

/// Spread for the random backoff jitter, in milliseconds.
const JITTER_MILLIS: u64 = 100;

/// Classify a request error for retries.
///
/// Only errors proving the request itself is broken are permanent; transport
/// failures (timeouts, resets, refused or dropped connections) may succeed
/// on a later attempt.
///
/// # Arguments
///
/// * `error` - Request error from `reqwest`.
pub fn classify_request(error: reqwest::Error) -> (Failure, anyhow::Error) {
    if error.is_builder() || error.is_redirect() {
        (Failure::Permanent, error.into())
    } else {
        (Failure::Transient, error.into())
    }
}

/// Classify an HTTP status for retries.
///
/// # Arguments
///
/// * `status` - Response status code.
///
/// # Returns
///
/// `Ok` for 2xx, retryable transient error for 429/5xx, permanent error
/// otherwise.
///
/// Note: 429s from `Prober` arrive as [`Failure::RateLimited`] instead,
/// since the `Retry-After` wait is served before classifying.
pub fn check_http_status(status: reqwest::StatusCode) -> Result<(), (Failure, anyhow::Error)> {
    if status.is_success() {
        Ok(())
    } else if status.as_u16() == 429 || status.is_server_error() {
        Err((
            Failure::Transient,
            anyhow::anyhow!("Request failed with status {status}"),
        ))
    } else {
        Err((
            Failure::Permanent,
            anyhow::anyhow!("Request failed with status {status}"),
        ))
    }
}

/// Backoff between retries: exponential base plus jitter.
///
/// The jitter spreads tasks that failed together so their retries do not
/// re-collide on the same instant.
///
/// # Arguments
///
/// * `attempt` - Zero-based attempt that just failed.
fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_millis(200 * 4_u64.pow(attempt.min(8)));
    base + Duration::from_millis(fastrand::u64(0..=JITTER_MILLIS))
}

/// Honor a `Retry-After` delay on a 429 response, capped at one minute.
///
/// Only delta-seconds values are supported; HTTP dates fall back to the
/// regular backoff.
///
/// # Arguments
///
/// * `response` - Response carrying the 429 status.
///
/// # Returns
///
/// Waited seconds, or `None` when the header is missing or unparsable.
pub(crate) fn retry_after_secs(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|&seconds| seconds > 0)
        .map(|seconds| seconds.min(60))
}

/// Run an operation with retries for transient failures.
///
/// The operation runs at least once. Permanent failures return immediately.
///
/// # Arguments
///
/// * `operation` - Fallible operation classifying its own errors.
/// * `attempts` - Maximum total attempts.
///
/// # Returns
///
/// Operation value, or the last error.
///
/// # Errors
///
/// Returns the permanent error, or the last transient error when attempts
/// run out.
pub async fn with_retry<T, F, Fut>(mut operation: F, attempts: u32) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, (Failure, anyhow::Error)>>,
{
    let attempts = attempts.max(1);
    let mut last_error = None;
    for attempt in 0..attempts {
        match operation().await {
            Ok(value) => return Ok(value),
            Err((Failure::Permanent, error)) => return Err(error),
            Err((Failure::RateLimited, error)) => {
                // The wait was already served (e.g. `Retry-After`); retry
                // at once instead of stacking another backoff on top.
                last_error = Some(error);
            }
            Err((Failure::Transient, error)) => {
                last_error = Some(error);
                if attempt + 1 < attempts {
                    tokio::time::sleep(backoff(attempt)).await;
                }
            }
        }
    }
    Err(last_error.expect("at least one attempt runs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn permanent_failure_skips_retries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker = calls.clone();
        let result: Result<()> = with_retry(
            move || {
                let worker = worker.clone();
                async move {
                    worker.fetch_add(1, Ordering::SeqCst);
                    Err((Failure::Permanent, anyhow::anyhow!("gone")))
                }
            },
            3,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rate_limited_retries_without_backoff() {
        // No sleeps are involved, so this runs in real time.
        let calls = Arc::new(AtomicUsize::new(0));
        let worker = calls.clone();
        let result = with_retry(
            move || {
                let worker = worker.clone();
                async move {
                    let call = worker.fetch_add(1, Ordering::SeqCst);
                    if call < 2 {
                        Err((Failure::RateLimited, anyhow::anyhow!("slow down")))
                    } else {
                        Ok(42)
                    }
                }
            },
            3,
        )
        .await;
        assert_eq!(result.expect("succeeds on third try"), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn transient_failure_retries_then_succeeds() {
        tokio::time::pause();
        let worker = Arc::new(AtomicUsize::new(0));
        let task = worker.clone();
        let handle = tokio::spawn(async move {
            with_retry(
                move || {
                    let task = task.clone();
                    async move {
                        let call = task.fetch_add(1, Ordering::SeqCst);
                        if call < 2 {
                            Err((Failure::Transient, anyhow::anyhow!("busy")))
                        } else {
                            Ok(42)
                        }
                    }
                },
                3,
            )
            .await
        });
        tokio::time::advance(Duration::from_secs(120)).await;
        let result = handle.await.expect("retries complete");
        assert_eq!(result.expect("succeeds on third try"), 42);
        assert_eq!(worker.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn backoff_grows_with_jitter_bounds() {
        for (attempt, base) in [(0, 200), (1, 800), (2, 3200)] {
            let delay = backoff(attempt);
            assert!(delay >= Duration::from_millis(base), "attempt {attempt}");
            assert!(
                delay <= Duration::from_millis(base + JITTER_MILLIS),
                "attempt {attempt}"
            );
        }
    }

    #[tokio::test]
    async fn refused_connection_is_transient() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind loopback")
            .local_addr()
            .expect("read local addr")
            .port();
        let client = reqwest::Client::new();
        let error = client
            .get(format!("http://127.0.0.1:{port}/x"))
            .send()
            .await
            .expect_err("refused port errors");
        assert!(!error.is_builder() && !error.is_redirect());
        assert_eq!(classify_request(error).0, Failure::Transient);
    }

    #[tokio::test]
    async fn unbuildable_request_is_permanent() {
        let client = reqwest::Client::new();
        let error = client
            .get("ht tp://not a url")
            .send()
            .await
            .expect_err("bad URL errors");
        assert!(error.is_builder());
        assert_eq!(classify_request(error).0, Failure::Permanent);
    }
}
