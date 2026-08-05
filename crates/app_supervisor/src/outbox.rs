//! The supervisor's own binding-write outbox: a
//! [`syneroym_sdk::deploy::WriteBindingsOutbox`] backed by
//! `syneroym_async_queue::Queue` (M05B Slice B1, D-B1-1/D-B1-5). The queue
//! itself stores an opaque key and payload; this module is where those
//! become the supervisor's `(instance, logical_ref, substrate)` grouping
//! (D-B1-6) and a serialized `BindingWrite`.

use std::{
    fmt,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use syneroym_async_queue::Queue;
use syneroym_sdk::{BindingWrite, deploy::WriteBindingsOutbox};

/// The unit separator (`\u{1}`) joins the three parts rather than a visible
/// character like `/` or `|`: an app instance id, a logical ref, and a DID
/// can all legitimately contain those, but never a control character.
const FIELD_SEPARATOR: char = '\u{1}';

/// The supervisor's grouping key for one queued binding write --
/// `(instance, logical_ref, substrate)`, the same triple the DLQ's standing
/// alert groups by (D-B1-6). Opaque to `syneroym-async-queue`, which only
/// stores and compares its `Display` form as a string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueueKey {
    pub app_instance_id: String,
    pub logical_ref: String,
    pub substrate_did: String,
}

impl fmt::Display for QueueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{FIELD_SEPARATOR}{}{FIELD_SEPARATOR}{}",
            self.app_instance_id, self.logical_ref, self.substrate_did
        )
    }
}

impl FromStr for QueueKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut parts = s.split(FIELD_SEPARATOR);
        let app_instance_id = parts.next().ok_or_else(|| anyhow!("empty queue key"))?;
        let logical_ref =
            parts.next().ok_or_else(|| anyhow!("queue key '{s}' is missing its logical ref"))?;
        let substrate_did =
            parts.next().ok_or_else(|| anyhow!("queue key '{s}' is missing its substrate DID"))?;
        if parts.next().is_some() {
            return Err(anyhow!("queue key '{s}' carries more than three fields"));
        }
        Ok(Self {
            app_instance_id: app_instance_id.to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_did: substrate_did.to_string(),
        })
    }
}

/// What actually rides in the outbox's payload column: the write itself,
/// plus the substrate it targets -- `BindingWrite` alone does not carry a
/// destination, and the worker (which reconnects long after the client
/// that first attempted this write is gone) needs one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct QueuedBindingWrite {
    pub substrate_did: String,
    pub write: BindingWrite,
}

#[must_use]
pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The `WriteBindingsOutbox` `deploy::build_durable_actor` enqueues onto.
/// `Queue` is cheap to clone (an `Arc`-backed connection), so a fresh
/// `SupervisorOutbox` is built at each call site that constructs a durable
/// actor rather than threaded through as shared state.
#[derive(Debug, Clone)]
pub struct SupervisorOutbox {
    queue: Queue,
}

impl SupervisorOutbox {
    #[must_use]
    pub fn new(queue: Queue) -> Self {
        Self { queue }
    }
}

#[async_trait::async_trait]
impl WriteBindingsOutbox for SupervisorOutbox {
    async fn enqueue(&self, queue_key: &str, substrate_did: &str, write: &BindingWrite) {
        // A5e's resident loop reclassifies a member as a push candidate on
        // every pass until its push actually lands (the downgrade-to-
        // Degraded comment on the loop's own write phase explains why:
        // `compute_diff` deliberately falls back to the previous baseline
        // so the next pass retries the same push). Left unguarded, every
        // one of those passes would enqueue its own row for the identical
        // logical write while a substrate stays offline, flooding the
        // outbox with duplicates that each start their own attempt budget
        // from zero -- the queue would never actually exhaust a budget,
        // since a newer, immediately-due duplicate keeps winning the next
        // claim over an older one waiting out its backoff. One pending row
        // per key is already this intent's durable record; a pass that
        // finds one is a no-op here; the row's own retry schedule is what
        // answers it.
        if self.already_pending(queue_key) {
            return;
        }
        let payload =
            QueuedBindingWrite { substrate_did: substrate_did.to_string(), write: write.clone() };
        let bytes = match serde_json::to_vec(&payload) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to serialize a binding write for the outbox; dropping it"
                );
                return;
            }
        };
        if let Err(e) = self.queue.enqueue(queue_key, &bytes, now_ms()) {
            tracing::warn!(
                error = %e,
                "failed to enqueue a binding write onto the durable outbox"
            );
        }
    }
}

impl SupervisorOutbox {
    /// Whether `queue_key` already has a row in the outbox -- pending or
    /// claimed, either way already durable and already on its own retry
    /// schedule. A linear scan over `Queue::all()`, not an indexed lookup:
    /// this is the supervisor's own per-instance outbox, not a shared
    /// multi-tenant one, so its size is bounded by how many distinct
    /// members are simultaneously mid-retry, not by anything unbounded.
    #[must_use]
    pub fn already_pending(&self, queue_key: &str) -> bool {
        self.queue.all().is_ok_and(|items| items.iter().any(|item| item.queue_key == queue_key))
    }
}

#[cfg(test)]
mod tests {
    use syneroym_async_queue::QueueConfig;
    use syneroym_core::config::SupervisorRole;

    use super::*;

    #[test]
    fn a_queue_key_round_trips_through_display_and_from_str() {
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let round_tripped: QueueKey = key.to_string().parse().unwrap();
        assert_eq!(round_tripped, key);
    }

    #[tokio::test]
    async fn enqueue_writes_a_row_the_queue_key_can_be_parsed_back_out_of() {
        let queue = Queue::open_in_memory(QueueConfig::from(&SupervisorRole::default())).unwrap();
        let outbox = SupervisorOutbox::new(queue.clone());
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let write = BindingWrite {
            service_id: "did:key:zSvc".to_string(),
            app_instance_id: "inst-1".to_string(),
            bindings: vec![],
            generation: 3,
        };

        outbox.enqueue(&key.to_string(), "did:key:zB", &write).await;

        let items = queue.all().unwrap();
        assert_eq!(items.len(), 1);
        let parsed_key: QueueKey = items[0].queue_key.parse().unwrap();
        assert_eq!(parsed_key, key);
        let payload: QueuedBindingWrite = serde_json::from_slice(&items[0].payload).unwrap();
        assert_eq!(payload.substrate_did, "did:key:zB");
        assert_eq!(payload.write.generation, 3);
    }

    /// M05B B1 review: A5e's resident loop reclassifies an unlanded push as
    /// a candidate every pass until it lands, so `enqueue_unreachable_push`
    /// (`service.rs`) calls this repeatedly for the identical key while a
    /// substrate stays offline. Left unguarded that floods the outbox with
    /// duplicates, each starting its own attempt budget at zero -- so an
    /// item never actually exhausts one, since a newer, immediately-due
    /// duplicate keeps winning the next claim over an older one waiting out
    /// its backoff.
    #[tokio::test]
    async fn a_second_enqueue_for_the_same_key_while_one_is_still_pending_is_a_no_op() {
        let queue = Queue::open_in_memory(QueueConfig::from(&SupervisorRole::default())).unwrap();
        let outbox = SupervisorOutbox::new(queue.clone());
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let first_write = BindingWrite {
            service_id: "did:key:zSvc".to_string(),
            app_instance_id: "inst-1".to_string(),
            bindings: vec![],
            generation: 3,
        };
        let later_write = BindingWrite { generation: 9, ..first_write.clone() };

        outbox.enqueue(&key.to_string(), "did:key:zB", &first_write).await;
        assert!(outbox.already_pending(&key.to_string()));
        outbox.enqueue(&key.to_string(), "did:key:zB", &later_write).await;

        let items = queue.all().unwrap();
        assert_eq!(items.len(), 1, "a pending row for the key must not be duplicated");
        let payload: QueuedBindingWrite = serde_json::from_slice(&items[0].payload).unwrap();
        assert_eq!(
            payload.write.generation, 3,
            "the first pending write is untouched, not replaced by the later one"
        );
    }

    #[tokio::test]
    async fn already_pending_is_false_for_a_key_never_enqueued() {
        let queue = Queue::open_in_memory(QueueConfig::from(&SupervisorRole::default())).unwrap();
        let outbox = SupervisorOutbox::new(queue);
        assert!(!outbox.already_pending("inst-1\u{1}inst-1/backend\u{1}did:key:zB"));
    }
}
