//! Generic retry with exponential backoff utility.

use std::{fmt::Display, time::Duration};

use tokio::time::sleep;
use tracing::warn;

use crate::config::RetryPolicy;

/// Calculates the next backoff with jitter applied (between -10% and +10%).
#[must_use]
pub fn calculate_jittered_backoff(base_backoff: u64) -> u64 {
    let jitter_factor = 1.0 + (rand::random::<f64>() * 0.2 - 0.1);
    (base_backoff as f64 * jitter_factor) as u64
}

/// Retries an asynchronous operation with exponential backoff.
///
/// Jitter of ±10% is applied to the backoff duration to prevent thundering herd
/// problems.
pub async fn retry_with_backoff<F, Fut, T, E>(
    policy: &RetryPolicy,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: Display,
{
    let mut attempts = 0;
    let mut current_backoff = policy.initial_backoff_ms;

    loop {
        attempts += 1;
        match operation().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempts >= policy.max_attempts {
                    return Err(e);
                }

                warn!("Operation failed (attempt {}/{}): {}", attempts, policy.max_attempts, e);

                let sleep_ms = calculate_jittered_backoff(current_backoff);
                sleep(Duration::from_millis(sleep_ms)).await;

                let next_backoff = (current_backoff as f64 * policy.backoff_multiplier) as u64;
                current_backoff = next_backoff.min(policy.max_backoff_ms);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };

    use super::*;

    #[tokio::test]
    async fn test_retry_success_first_try() {
        let policy = RetryPolicy::default();
        let counter = Arc::new(AtomicU8::new(0));
        let c = counter.clone();

        let result = retry_with_backoff(&policy, || async {
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>("success")
        })
        .await;

        assert_eq!(result, Ok("success"));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_second_try() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1, // very fast for tests
            backoff_multiplier: 2.0,
            max_backoff_ms: 10,
        };
        let counter = Arc::new(AtomicU8::new(0));
        let c = counter.clone();

        let result = retry_with_backoff(&policy, || async {
            let attempt = c.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt < 2 { Err::<&str, _>("fail") } else { Ok::<_, &str>("success") }
        })
        .await;

        assert_eq!(result, Ok("success"));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_max_attempts_exhausted() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1,
            backoff_multiplier: 2.0,
            max_backoff_ms: 10,
        };
        let counter = Arc::new(AtomicU8::new(0));
        let c = counter.clone();

        let result = retry_with_backoff(&policy, || async {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<&str, _>("persistent fail")
        })
        .await;

        assert_eq!(result, Err("persistent fail"));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_jitter_distribution() {
        let base_backoff = 1000;
        let mut samples = Vec::new();
        for _ in 0..1000 {
            let val = calculate_jittered_backoff(base_backoff);
            assert!(val >= 900 && val <= 1100, "Jittered value {} out of range", val);
            samples.push(val);
        }

        // Check that we have a decent distribution (i.e. not all same values, which would happen with SystemTime thundering herd)
        samples.sort();
        let min = samples[0];
        let max = samples[samples.len() - 1];
        assert!(max - min > 150, "Insufficient spread in jitter distribution (min: {}, max: {})", min, max);

        // Check average is close to the base_backoff
        let sum: u64 = samples.iter().sum();
        let avg = sum as f64 / samples.len() as f64;
        assert!((avg - 1000.0).abs() < 10.0, "Average jittered backoff {} is too far from base 1000", avg);
    }
}
