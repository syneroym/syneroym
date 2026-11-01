use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use syneroym_core::config::{SubstrateConfig, sandbox};
use tokio::sync::Semaphore;

/// Minimum remaining wait allowance required to begin a batch.
pub const MIN_BATCH_ALLOWANCE: Duration = Duration::from_secs(1);

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
            max_calls: sandbox::default_call_all_max_calls(),
            max_concurrency: sandbox::default_call_all_max_concurrency(),
            default_call_timeout: Duration::from_millis(
                sandbox::default_call_all_default_call_timeout_ms(),
            ),
            max_call_timeout: Duration::from_millis(sandbox::default_call_all_max_call_timeout_ms()),
            total_timeout: Duration::from_millis(sandbox::default_call_all_total_timeout_ms()),
        }
    }
}

impl CallAllLimits {
    /// Computes the effective timeout for an individual call within a batch.
    #[must_use]
    pub fn call_timeout(&self, requested: Option<u32>) -> Duration {
        match requested {
            Some(ms) => Duration::from_millis(u64::from(ms))
                .clamp(Duration::from_millis(1), self.max_call_timeout),
            None => self.default_call_timeout,
        }
    }
}

static DEFAULT_WAITERS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn default_waiters_semaphore() -> Arc<Semaphore> {
    DEFAULT_WAITERS
        .get_or_init(|| Arc::new(Semaphore::new(sandbox::default_call_all_max_waiting_guests())))
        .clone()
}

/// Node-wide state and configuration for `call-all` batch execution.
#[derive(Clone, Debug)]
pub struct CallAll {
    /// Resolved limits for batched calls.
    pub limits: CallAllLimits,
    /// Semaphore bounding the number of guests waiting in `call-all`
    /// concurrently.
    pub(crate) waiters: Arc<Semaphore>,
}

impl Default for CallAll {
    fn default() -> Self {
        Self { limits: CallAllLimits::default(), waiters: default_waiters_semaphore() }
    }
}

impl CallAll {
    /// Resolves `call-all` limits and semaphore capacity from the substrate
    /// configuration, clamping invalid values (zeros and out-of-order
    /// bounds) with warnings.
    #[must_use]
    pub fn from_config(config: &SubstrateConfig) -> Self {
        let Some(sandbox) = &config.roles.app_sandbox else {
            return Self::default();
        };

        let mut max_calls = sandbox.call_all_max_calls;
        if max_calls == 0 {
            tracing::warn!("call_all_max_calls is 0; clamping to 1");
            max_calls = 1;
        }

        let mut max_concurrency = sandbox.call_all_max_concurrency;
        if max_concurrency == 0 {
            tracing::warn!("call_all_max_concurrency is 0; clamping to 1");
            max_concurrency = 1;
        }

        let mut total_timeout = Duration::from_millis(sandbox.call_all_total_timeout_ms);
        if total_timeout < MIN_BATCH_ALLOWANCE {
            tracing::warn!(
                "call_all_total_timeout_ms ({:?}) is below minimum batch allowance ({:?}); \
                 clamping to minimum batch allowance",
                total_timeout,
                MIN_BATCH_ALLOWANCE
            );
            total_timeout = MIN_BATCH_ALLOWANCE;
        }

        let mut max_call_timeout =
            Duration::from_millis(sandbox.call_all_max_call_timeout_ms.max(1));
        if sandbox.call_all_max_call_timeout_ms == 0 {
            tracing::warn!("call_all_max_call_timeout_ms is 0; clamping to 1 ms");
        }
        if max_call_timeout > total_timeout {
            tracing::warn!(
                "call_all_max_call_timeout_ms ({:?}) exceeds total timeout ({:?}); clamping to \
                 total timeout",
                max_call_timeout,
                total_timeout
            );
            max_call_timeout = total_timeout;
        }

        let mut default_call_timeout =
            Duration::from_millis(sandbox.call_all_default_call_timeout_ms.max(1));
        if sandbox.call_all_default_call_timeout_ms == 0 {
            tracing::warn!("call_all_default_call_timeout_ms is 0; clamping to 1 ms");
        }
        if default_call_timeout > max_call_timeout {
            tracing::warn!(
                "call_all_default_call_timeout_ms ({:?}) exceeds max call timeout ({:?}); \
                 clamping to max call timeout",
                default_call_timeout,
                max_call_timeout
            );
            default_call_timeout = max_call_timeout;
        }

        let max_allowed_waiters =
            (sandbox.max_concurrent_instances.saturating_sub(1) as usize).max(1);
        let mut max_waiting_guests = sandbox.call_all_max_waiting_guests;
        if max_waiting_guests == 0 {
            tracing::warn!("call_all_max_waiting_guests is 0; clamping to 1");
            max_waiting_guests = 1;
        }
        if max_waiting_guests > max_allowed_waiters {
            tracing::warn!(
                "call_all_max_waiting_guests ({max_waiting_guests}) exceeds max_instances - 1; \
                 clamping to {max_allowed_waiters}"
            );
            max_waiting_guests = max_allowed_waiters;
        }

        let limits = CallAllLimits {
            max_calls,
            max_concurrency,
            default_call_timeout,
            max_call_timeout,
            total_timeout,
        };

        Self::new(limits, max_waiting_guests)
    }

    /// Creates a new `CallAll` with custom limits and waiter capacity.
    #[must_use]
    pub fn new(limits: CallAllLimits, max_waiting_guests: usize) -> Self {
        Self { limits, waiters: Arc::new(Semaphore::new(max_waiting_guests)) }
    }
}
