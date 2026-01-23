//! Retry utilities with exponential backoff.

use std::future::Future;
use std::time::Duration;

use crate::{CelestiaError, Result};

/// Default maximum number of retry attempts.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Default base delay between retries in milliseconds.
pub const DEFAULT_RETRY_DELAY_MS: u64 = 1000;

/// Default delay before reconnecting in milliseconds.
pub const DEFAULT_RECONNECT_DELAY_MS: u64 = 5000;

/// Execute an operation with linear backoff retry logic.
///
/// The delay between attempts increases linearly: `base_delay_ms * attempt_number`.
///
/// # Arguments
///
/// * `operation_name` - Name used in log messages
/// * `max_retries` - Maximum number of attempts before giving up
/// * `base_delay_ms` - Base delay in milliseconds (multiplied by attempt number)
/// * `operation` - Async closure that performs the operation
///
/// # Returns
///
/// The result of the operation if successful within max_retries attempts,
/// or a `CelestiaError::SubmissionFailed` if all attempts fail.
///
/// # Example
///
/// ```ignore
/// let result = with_retry(
///     "blob submission",
///     3,
///     1000,
///     || async {
///         client.submit_blob(&blob).await
///     },
/// ).await?;
/// ```
pub async fn with_retry<T, E, F, Fut>(
    operation_name: &str,
    max_retries: u32,
    base_delay_ms: u64,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempts = 0u32;

    loop {
        attempts += 1;

        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempts >= max_retries {
                    return Err(CelestiaError::SubmissionFailed(format!(
                        "{operation_name} failed after {attempts} attempts: {e}"
                    )));
                }

                tracing::warn!(
                    attempt = attempts,
                    max_retries,
                    error = %e,
                    "{} failed, retrying...",
                    operation_name
                );

                tokio::time::sleep(Duration::from_millis(
                    base_delay_ms * u64::from(attempts),
                ))
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_with_retry_succeeds_first_try() {
        let result = with_retry("test op", 3, 10, || async { Ok::<_, &str>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_with_retry_succeeds_after_failures() {
        let attempts = AtomicU32::new(0);

        let result = with_retry("test op", 3, 10, || {
            let count = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if count < 2 {
                    Err("temporary failure")
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_with_retry_fails_after_max_retries() {
        let attempts = AtomicU32::new(0);

        let result = with_retry("test op", 3, 10, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err::<i32, _>("permanent failure") }
        })
        .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("test op failed after 3 attempts"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
