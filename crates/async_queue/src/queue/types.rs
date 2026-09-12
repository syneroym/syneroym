use syneroym_core::config::{AppSandboxRole, RetryPolicy, SupervisorRole};

use super::backoff_before_wait;

/// The retry curve ([`RetryPolicy`], reused for its struct
/// and `calculate_jittered_backoff`, not `retry_with_backoff`, which sleeps
/// in-process where a durable queue must compute a timestamp and forget the
/// item until a worker tick finds it due) plus this queue's own knobs.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub retry: RetryPolicy,
    /// How long a claimed item stays invisible to a second claim before a
    /// crashed worker's hold on it is assumed gone.
    pub visibility_timeout_ms: u64,
    /// Dead letters are pruned oldest-first, *within one `group_key`*, on
    /// every write past this count -- a bound and a trigger, not
    /// an adjective.
    pub dlq_max_rows: u32,
    /// Ceiling on items waiting for delivery. Unlike the dead-letter cap
    /// this one **refuses** rather than evicting: a pending item is work
    /// somebody is still expecting to happen, so dropping the oldest
    /// silently would be exactly the loss the queue exists to prevent.
    pub max_pending_rows: u32,
}

/// The supervisor's five `queue_*` fields, converted:
/// initial backoff and multiplier stay `RetryPolicy`'s own defaults (100 ms,
/// x2) since `SupervisorRole` configures only the attempt budget and the
/// ceiling, not the shape of the early curve.
impl From<&SupervisorRole> for QueueConfig {
    fn from(role: &SupervisorRole) -> Self {
        let defaults = RetryPolicy::default();
        // A configured 0 would dead-letter every item on its first failure
        // with no warning -- the queue crate has no `tracing` dependency of
        // its own to log through, so it clamps silently and the one caller
        // that constructs this from operator config (`SupervisorService::new`)
        // is the one that warns, mirroring `max_renewals_per_pass`'s
        // existing clamp.
        let max_attempts = role.queue_max_attempts.max(1);
        Self {
            retry: RetryPolicy {
                max_attempts,
                initial_backoff_ms: defaults.initial_backoff_ms,
                backoff_multiplier: defaults.backoff_multiplier,
                max_backoff_ms: role.queue_max_backoff_secs.saturating_mul(1000),
            },
            visibility_timeout_ms: role.queue_visibility_timeout_secs.saturating_mul(1000),
            dlq_max_rows: role.queue_dlq_max_rows,
            max_pending_rows: DEFAULT_MAX_PENDING_ROWS,
        }
    }
}

/// The sandbox role's five `queue_*` fields, converted -- the guest proxy
/// outbox's own budget. Same shape and same clamp as the supervisor's:
/// initial backoff and multiplier stay `RetryPolicy`'s defaults, since the
/// role configures the attempt budget and the ceiling, not the shape of the
/// early curve.
impl From<&AppSandboxRole> for QueueConfig {
    fn from(role: &AppSandboxRole) -> Self {
        let defaults = RetryPolicy::default();
        Self {
            retry: RetryPolicy {
                max_attempts: role.queue_max_attempts.max(1),
                initial_backoff_ms: defaults.initial_backoff_ms,
                backoff_multiplier: defaults.backoff_multiplier,
                max_backoff_ms: role.queue_max_backoff_secs.saturating_mul(1000),
            },
            visibility_timeout_ms: role.queue_visibility_timeout_secs.saturating_mul(1000),
            dlq_max_rows: role.queue_dlq_max_rows,
            max_pending_rows: DEFAULT_MAX_PENDING_ROWS,
        }
    }
}

impl QueueConfig {
    /// The nominal, unjittered time this budget spends retrying one item
    /// before it dead-letters: the sum of every wait the backoff curve
    /// produces over the attempt budget.
    #[must_use]
    pub fn total_retry_window_ms(&self) -> u64 {
        (1..u32::from(self.retry.max_attempts))
            .map(|wait| backoff_before_wait(&self.retry, wait))
            .sum()
    }
}

/// How many items one queue may hold waiting for delivery before it
/// refuses more. Derived beside the other budgets rather than configured,
/// for the same reason the dedup bounds are: the only interesting setting
/// is one that breaks the guarantee.
pub const DEFAULT_MAX_PENDING_ROWS: u32 = 10_000;

/// One item due for delivery, as [`Queue::claim_due`] hands it to a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub id: i64,
    /// The caller's own grouping/dedup key -- opaque to this crate. The
    /// supervisor outbox encodes `(instance, logical_ref, substrate)` into
    /// it, since that is the row the DLQ's standing alert groups by; this
    /// crate does not need to know that shape.
    pub queue_key: String,
    pub payload: Vec<u8>,
    /// How many delivery attempts this item has already used, including
    /// the one that is about to happen -- 0 for one never yet claimed.
    /// Advanced only by [`Queue::fail`]: a claim that never calls `fail`
    /// (a panic, a crashed worker) does not, by itself, count as a used
    /// attempt against this budget -- [`Self::claim_count`] is what bounds
    /// that case instead.
    pub attempts: u32,
    /// How many times this item has been claimed, including this claim --
    /// distinct from `attempts`, which only [`Queue::fail`] advances. A
    /// worker that panics or crashes on every delivery attempt never
    /// reaches `fail`, so `attempts` alone cannot bound it; the caller is
    /// expected to dead-letter (via `Queue::fail(..., terminal: true)`) an
    /// item whose `claim_count` reaches the same attempt budget, closing
    /// the poison-pill gap that `attempts` alone leaves open.
    ///
    /// **Also counts a claim a caller abandoned on purpose**, not only a
    /// crash: the supervisor's own worker (`app_supervisor::service::
    /// queue_worker_tick`) races cancellation into a delivery via
    /// `tokio::select!` so shutdown does not wait for one in flight -- a
    /// restart caught at exactly that instant drops the delivery after this
    /// count was already advanced, with no `attempts` to show for it.
    /// Indistinguishable from a real poison pill from inside this crate, and
    /// deliberately left that way: the failure mode is one claim spent for
    /// no attempt, bounded by the same budget either way, and it takes the
    /// same number of consecutive occurrences (the configured `max_attempts`,
    /// 54 by default) to matter -- an operator sees a real, replayable dead
    /// letter either way, never silent loss.
    pub claim_count: u32,
}

/// A terminally failed item, as [`Queue::dead_letters`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub id: i64,
    pub queue_key: String,
    pub payload: Vec<u8>,
    /// Attempts made before this item was dead-lettered.
    pub attempts: u32,
    pub last_error: String,
    pub created_at: i64,
}

/// What [`Queue::fail`] did with an item that did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailOutcome {
    /// Still under budget; back on the outbox, due again at this time.
    Retrying { next_attempt_at: i64 },
    /// Attempts exhausted, or the caller marked this failure terminal;
    /// moved to `dead_letters`. `pruned_keys` carries the `queue_key` of
    /// every *other* dead letter this same write evicted past
    /// `dlq_max_rows` -- a caller with a standing alert keyed by
    /// `queue_key` needs this to clear it, since a prune is otherwise
    /// silent.
    DeadLettered { pruned_keys: Vec<String> },
}
