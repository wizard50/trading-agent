use std::{error::Error, future::Future, time::Duration};
use tokio::time::sleep;
use tracing::warn;

pub async fn with_exponential_backoff<F, Fut, T>(
    max_attempts: u32,
    call: F,
) -> Result<T, Box<dyn Error>>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, Box<dyn Error>>>,
{
    let mut last_error: Option<Box<dyn Error>> = None;

    for attempt in 1..=max_attempts {
        match call().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);

                // Only retry on specific transient / network errors
                let error_str = last_error.as_ref().unwrap().to_string().to_lowercase();
                let is_retryable = error_str.contains("timeout")
                    || error_str.contains("connection")
                    || error_str.contains("network")
                    || error_str.contains("5xx")
                    || error_str.contains("internal server")
                    || error_str.contains("service unavailable");

                if !is_retryable || attempt == max_attempts {
                    break;
                }

                // exponential backoff
                let base = 2u64.pow(attempt - 1);
                let full_jitter: u64 = rand::random_range(0..=base);
                let backoff = Duration::from_secs(full_jitter);

                warn!(
                    event = "retry_attempt",
                    attempt = attempt,
                    max_attempts = max_attempts,
                    backoff_seconds = full_jitter,
                    "Request failed, retrying with exponential backoff"
                );

                sleep(backoff).await;
            }
        }
    }

    let error_msg = match last_error {
        Some(e) => format!(
            "Operation failed after {} attempts. Last error: {}",
            max_attempts, e
        ),
        None => format!(
            "Operation failed after {} attempts. Unknown error.",
            max_attempts
        ),
    };

    Err(error_msg.into())
}
