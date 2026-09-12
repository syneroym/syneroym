#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! A durable, owner-local work queue: an `outbox` of items awaiting
//! delivery, and a `dead_letters` table for the ones that never will be
//! ([ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md)).
//!
//! Generic over an opaque payload -- this crate never interprets what it
//! carries, only when to retry it and when to give up. Every queue belongs
//! to one process (ADR-0023 §6): there is no cross-process claim, no
//! distributed lock, and no compare-and-set. Correctness under
//! at-least-once delivery is the caller's own idempotent fence, not
//! anything this crate provides (ADR-0023 §1).
//!
//! **Every timestamp taken or returned by this crate is Unix
//! milliseconds**, not seconds like most of the rest of the tree -- the
//! reused [`RetryPolicy`] backoff curve is specified in milliseconds
//! (100 ms initial backoff), and converting it to second granularity would
//! make the first few retries indistinguishable from each other.
//!
//! **`queue_key` versus `group_key`.** `queue_key` is the caller's own
//! dedup/grouping key for one *item* -- opaque to this crate. `group_key`
//! is a second, coarser opaque string a caller may supply at [`Queue::enqueue`]
//! to scope the dead-letter cap ([`QueueConfig::dlq_max_rows`]) and its
//! pruning: the cap and the oldest-first eviction it triggers apply *within*
//! one `group_key`, not across the whole table. The supervisor outbox
//! groups by app instance, so one noisy instance cannot evict another's
//! operator-visible dead letters -- but this crate never parses either
//! string, so any caller-chosen grouping works.

pub mod dedup;
pub mod queue;
pub mod saga;

pub use crate::{
    dedup::{
        CALL_ALREADY_RUNNING_RPC_CODE, CALL_RESULT_NOT_RETAINED_RPC_CODE, ClaimToken, DedupConfig,
        DedupDecision, DedupStore, FirstOutcome,
    },
    queue::{
        DEFAULT_MAX_PENDING_ROWS, DeadLetter, FailOutcome, Queue, QueueConfig, QueueItem, TxQueue,
        backoff_before_wait,
    },
    saga::{
        CompensationOutcome, MAX_SAGA_PAYLOAD_BYTES, MIN_STEP_CALL_BUDGET_MS, SagaConfig, SagaHead,
        SagaInfo, SagaLog, SagaState, StepIntent, StepRow, StepState,
    },
};
