use std::time::Duration;

use anyhow::Result;

/// Whether a failed operation is worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// May succeed later (timeout, 429, 5xx, connection reset).
    Transient,
    /// Will never succeed (4xx, bad data, unknown channel).
    Permanent,
}

/// Classify a request error for retries.
pub fn classify_request(error: reqwest::Error) -> (Failure, anyhow::Error) {
    if error.is_timeout() {
        (Failure::Transient, error.into())
    } else {
        (Failure::Permanent, error.into())
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

/// Backoff between retries.
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(200 * 4_u64.pow(attempt))
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
    async fn transient_failure_retries_then_succeeds() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker = calls.clone();
        let result = with_retry(
            move || {
                let worker = worker.clone();
                async move {
                    let call = worker.fetch_add(1, Ordering::SeqCst);
                    if call < 2 {
                        Err((Failure::Transient, anyhow::anyhow!("busy")))
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
}
