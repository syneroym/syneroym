use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use tokio::sync::Semaphore;

/// Limits applied to batched outbound calls (`call-all`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallAllLimits {
    /// Maximum number of calls allowed in a single batch.
    pub max_calls: usize,
    /// Maximum number of calls executing concurrently for one instance.
    pub max_concurrency: usize,
    /// Default per-call timeout when unspecified by caller.
    pub default_call_timeout: Duration,
    /// Maximum allowed timeout for an individual call.
    pub max_call_timeout: Duration,
    /// Total timeout for the entire batch.
    pub total_timeout: Duration,
}

impl Default for CallAllLimits {
    fn default() -> Self {
        Self {
            max_calls: 16,
            max_concurrency: 8,
            default_call_timeout: Duration::from_millis(5000),
            max_call_timeout: Duration::from_millis(10000),
            total_timeout: Duration::from_millis(15000),
        }
    }
}

impl CallAllLimits {
    /// Computes the effective timeout for an individual call within a batch.
    #[must_use]
    pub fn call_timeout(&self, requested: Option<u64>) -> Duration {
        match requested {
            Some(ms) => {
                Duration::from_millis(ms).clamp(Duration::from_millis(1), self.max_call_timeout)
            }
            None => self.default_call_timeout,
        }
    }
}

static DEFAULT_WAITERS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn default_waiters_semaphore() -> Arc<Semaphore> {
    DEFAULT_WAITERS.get_or_init(|| Arc::new(Semaphore::new(4))).clone()
}

/// Node-wide state and configuration for `call-all` batch execution.
#[derive(Clone, Debug)]
pub struct CallAll {
    /// Resolved limits for batched calls.
    pub limits: CallAllLimits,
    /// Semaphore bounding the number of guests waiting in `call-all`
    /// concurrently.
    #[allow(dead_code)]
    pub(crate) waiters: Arc<Semaphore>,
}

impl Default for CallAll {
    fn default() -> Self {
        Self { limits: CallAllLimits::default(), waiters: default_waiters_semaphore() }
    }
}

impl CallAll {
    /// Creates a new `CallAll` with custom limits and waiter capacity.
    #[must_use]
    pub fn new(limits: CallAllLimits, max_waiting_guests: usize) -> Self {
        Self { limits, waiters: Arc::new(Semaphore::new(max_waiting_guests)) }
    }
}
