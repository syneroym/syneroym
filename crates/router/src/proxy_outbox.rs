//! The guest-facing durable outbox: fire-and-forget calls that survive an
//! unreachable target and a process restart
//! ([ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md)
//! §2, §4, §5).
//!
//! **One queue per calling service, in that service's own encrypted
//! database.** A queued call's payload is the caller's own data -- the same
//! data that lives in its `state.db` -- so it waits in a sibling file under
//! the same key, never in the node-wide substrate database, which is not
//! encrypted. The file is host-owned rather than part of `state.db` itself,
//! so a guest's own read surface cannot reach it.
//!
//! **Intent, not a resolved target.** The stored item names the dependency
//! the caller asked for, and resolution happens again on every delivery
//! attempt. Storing the resolved DID would snapshot a binding for hours,
//! which is precisely what the no-snapshot rule (ADR-0021 §2) exists to
//! prevent, and for far longer than a caller could manage on its own.
//!
//! **A queued failure is not a queued success.** The synchronous rule --
//! "the callee answered, so the answer is definitive" -- is right when a
//! caller is holding the error. A fire-and-forget item has no such reader,
//! so completing it on a callee error would be silent loss. Here every
//! callee error dead-letters, with exactly one exception: the reserved code
//! meaning "this same item is running on the receiver right now", which is
//! definitive-looking and temporary in fact.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use syneroym_app_orchestration::{
    AppInstanceId, LogicalResolver, LogicalServiceName, LogicalServiceRef,
};
use syneroym_async_queue::{CALL_ALREADY_RUNNING_RPC_CODE, Queue, QueueConfig};
use syneroym_core::local_registry::EndpointRegistry;
use syneroym_data_db::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{ProxyError, QueuedCall, QueuedTarget};
use tokio::task;
use tracing::warn;

use crate::call_dedup::ASYNC_DB_NAME;

/// How many items one service may have claimed in a single worker tick.
/// One service with a permanently unreachable target must not spend the
/// whole node's tick budget on its own backlog.
pub(crate) const CLAIM_LIMIT_PER_TICK: u32 = 16;

/// Whether a delivery failure should be retried, or is settled for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Retry,
    Terminal,
}

/// Classifies a queued delivery's failure.
///
/// This is deliberately **not** the synchronous rule. There, a callee
/// answer is definitive and the caller reads it; here the caller is long
/// gone, so a callee error that merely completed the item would be silent
/// loss. Every callee error is terminal instead, and the one reserved code
/// that means "the receiver is running this exact item right now" retries
/// -- definitive-looking, temporary in fact.
///
/// `Internal` retries: it covers "sandbox engine unavailable" and "native
/// dispatch registry gone", which are shutdown-window states rather than
/// defects in the item. The same attempt budget still bounds it, so a
/// genuine host defect reaches the dead-letter table instead of looping.
///
/// `PermissionDenied` is terminal on purpose: the gate is deterministic, so
/// retrying re-asks a question that is already settled.
#[must_use]
pub fn disposition_of(error: &ProxyError) -> Disposition {
    match error {
        ProxyError::Callee { code, .. } if *code == CALL_ALREADY_RUNNING_RPC_CODE => {
            Disposition::Retry
        }
        ProxyError::Callee { .. }
        | ProxyError::ServiceNotFound(_)
        | ProxyError::UnsupportedTarget(_)
        | ProxyError::PermissionDenied(_)
        | ProxyError::UnsupportedProtocol(_) => Disposition::Terminal,
        ProxyError::Transport(_) | ProxyError::Timeout(_) | ProxyError::Internal(_) => {
            Disposition::Retry
        }
    }
}

/// One node's guest outboxes: a queue per calling service, opened once.
pub struct ProxyOutbox {
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    resolver: Arc<LogicalResolver>,
    config: QueueConfig,
    queues: Mutex<HashMap<String, Queue>>,
}

impl std::fmt::Debug for ProxyOutbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyOutbox").finish_non_exhaustive()
    }
}

impl ProxyOutbox {
    #[must_use]
    pub fn new(
        storage_provider: Arc<dyn StorageProvider>,
        key_store: Arc<KeyStore>,
        resolver: Arc<LogicalResolver>,
        config: QueueConfig,
    ) -> Self {
        Self { storage_provider, key_store, resolver, config, queues: Mutex::new(HashMap::new()) }
    }

    #[must_use]
    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    /// Opens (once) the outbox for `service_id`, on that service's own
    /// database with that service's own key.
    pub async fn queue_for(&self, service_id: &str) -> Result<Queue, ProxyError> {
        if let Some(queue) = self
            .queues
            .lock()
            .map_err(|_| ProxyError::Internal("outbox cache poisoned".to_string()))?
            .get(service_id)
            .cloned()
        {
            return Ok(queue);
        }
        if !self
            .storage_provider
            .service_exists(service_id)
            .await
            .map_err(|e| ProxyError::Internal(format!("cannot check calling service: {e}")))?
        {
            return Err(ProxyError::ServiceNotFound(service_id.to_string()));
        }
        let dek = self
            .storage_provider
            .load_service_dek(service_id, &self.key_store)
            .await
            .map_err(|e| ProxyError::Internal(format!("outbox unavailable: {e}")))?;
        let dir = self
            .storage_provider
            .service_db_dir(service_id)
            .map_err(|e| ProxyError::Internal(format!("outbox unavailable: {e}")))?;
        let config = self.config.clone();
        let queue = task::spawn_blocking(move || -> anyhow::Result<Queue> {
            std::fs::create_dir_all(&dir)?;
            Queue::open_encrypted(&dir, ASYNC_DB_NAME, dek.as_deref(), config)
        })
        .await
        .map_err(|e| ProxyError::Internal(format!("outbox task failed: {e}")))?
        .map_err(|e| ProxyError::Internal(format!("outbox unavailable: {e}")))?;

        self.queues
            .lock()
            .map_err(|_| ProxyError::Internal("outbox cache poisoned".to_string()))?
            .insert(service_id.to_string(), queue.clone());
        Ok(queue)
    }

    /// Resolves a queued item's stored intent into the DID to deliver to,
    /// now -- so a binding re-pushed while the item waited takes effect.
    /// A dependency that no longer resolves is terminal.
    pub fn resolve_target(&self, call: &QueuedCall) -> Result<String, ProxyError> {
        match &call.target {
            QueuedTarget::Service(did) => Ok(did.clone()),
            QueuedTarget::Dependency(name) => {
                let app_instance_id = call.app_instance_id.as_deref().ok_or_else(|| {
                    ProxyError::ServiceNotFound(format!(
                        "queued dependency '{name}' has no app instance to resolve against"
                    ))
                })?;
                let logical_ref = LogicalServiceRef {
                    app_instance_id: AppInstanceId::try_new(app_instance_id).map_err(|e| {
                        ProxyError::Internal(format!("stored app context is unreadable: {e}"))
                    })?,
                    service_name: LogicalServiceName::try_new(name).map_err(|e| {
                        ProxyError::ServiceNotFound(format!("invalid dependency name: {e}"))
                    })?,
                };
                self.resolver
                    .resolve(&logical_ref, call.routing_key.as_deref().map(str::as_bytes))
                    .map(|id| id.to_string())
                    .map_err(|e| {
                        ProxyError::ServiceNotFound(format!(
                            "dependency '{name}' of '{}' is no longer bound: {e}",
                            call.caller_service_id
                        ))
                    })
            }
        }
    }

    /// Writes an item to the calling service's outbox, unless one is
    /// already pending for the same key.
    ///
    /// The queue key *is* the idempotency key, so a caller that re-enqueues
    /// the same logical operation after a crash gets one queued item rather
    /// than two -- deduplicated at the sender, before any wire traffic. The
    /// group key is the *target*, so one permanently broken recipient
    /// cannot evict every other conversation's dead letters.
    pub async fn store(&self, call: &QueuedCall) -> Result<(), ProxyError> {
        let queue = self.queue_for(&call.caller_service_id).await?;
        let payload = serde_json::to_vec(call)
            .map_err(|e| ProxyError::Internal(format!("cannot store queued call: {e}")))?;
        let (group, key) = (group_key_of(call), call.idempotency_key.clone());
        let now = now_ms();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            if queue.has_pending(&key)? {
                return Ok(());
            }
            queue.enqueue(&group, &key, &payload, now)?;
            Ok(())
        })
        .await
        .map_err(|e| ProxyError::Internal(format!("outbox task failed: {e}")))?
        .map_err(|e| ProxyError::Internal(format!("cannot write to the outbox: {e}")))?;
        metrics::counter!("substrate.proxy.outbox.enqueued").increment(1);
        Ok(())
    }

    /// Which services the worker should drain this tick: every one whose
    /// outbox is already open, filtered to those the endpoint registry
    /// still knows.
    ///
    /// Nothing removes a service's data directory on undeploy, so an
    /// orphaned `async.db` can outlive the service that wrote it. Its items
    /// are completed silently rather than delivered -- delivering would
    /// resurrect intent an operator withdrew -- and rather than
    /// dead-lettered, which would raise noise about a service nobody is
    /// going to act on.
    #[must_use]
    pub fn open_services(&self) -> Vec<String> {
        self.queues.lock().map(|q| q.keys().cloned().collect()).unwrap_or_default()
    }

    /// Whether `service_id` already has a queue file on disk.
    ///
    /// Checked before opening, so a worker tick does not create an empty
    /// `async.db` for every deployed service on the node just to discover
    /// it has nothing queued. This is also what lets a restart find items
    /// written before it, when nothing is in the cache yet.
    #[must_use]
    pub fn queue_file_exists(&self, service_id: &str) -> bool {
        self.storage_provider
            .service_db_dir(service_id)
            .is_ok_and(|dir| dir.join(ASYNC_DB_NAME).exists())
    }

    /// Whether the endpoint registry still knows `service_id`.
    pub fn is_still_deployed(registry: &EndpointRegistry, service_id: &str) -> bool {
        registry.owner_of(service_id).is_some() || registry.instance_cert(service_id).is_some()
    }
}

/// The dead-letter cap is scoped by this, so one permanently broken
/// recipient cannot evict another conversation's dead letters. Grouping by
/// *caller* would be useless: the file is already per-caller.
#[must_use]
pub fn group_key_of(call: &QueuedCall) -> String {
    match &call.target {
        QueuedTarget::Dependency(name) => format!("dep:{name}"),
        QueuedTarget::Service(did) => format!("did:{did}"),
    }
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn log_delivery_failure(key: &str, error: &ProxyError) {
    warn!(idempotency_key = key, error = %error, "queued proxy call failed");
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn call(target: QueuedTarget) -> QueuedCall {
        QueuedCall {
            app_instance_id: Some("app-1".to_string()),
            caller_service_id: "svc-caller".to_string(),
            target,
            routing_key: None,
            interface: "greeter".to_string(),
            method: "greet".to_string(),
            params: serde_json::Value::Null,
            idempotency_key: "msg-7".to_string(),
            protocol: None,
            timeout_ms: None,
        }
    }

    /// The row a reader coming from the synchronous rule will expect to
    /// behave the other way round: there, a callee answer completes the
    /// call; here, nobody is left to read it, so it must be recorded.
    #[test]
    fn a_callee_error_on_a_queued_item_is_terminal_rather_than_completing() {
        let error = ProxyError::Callee { code: -32010, message: "no".into(), data: None };
        assert_eq!(disposition_of(&error), Disposition::Terminal);
    }

    /// The one documented exception, and the reason it exists: the
    /// receiver is running this exact item right now.
    #[test]
    fn an_in_flight_dedup_refusal_retries_rather_than_dead_lettering() {
        let error = ProxyError::Callee {
            code: CALL_ALREADY_RUNNING_RPC_CODE,
            message: "already running".into(),
            data: None,
        };
        assert_eq!(disposition_of(&error), Disposition::Retry);
    }

    /// The pair most easily classified the wrong way round, asserted
    /// together: a host-side internal error is a shutdown-window state, a
    /// permission denial is a settled question.
    #[test]
    fn a_host_side_internal_error_retries_and_a_permission_denial_does_not() {
        assert_eq!(
            disposition_of(&ProxyError::Internal("sandbox engine unavailable".into())),
            Disposition::Retry
        );
        assert_eq!(
            disposition_of(&ProxyError::PermissionDenied("denied".into())),
            Disposition::Terminal
        );
    }

    #[test]
    fn a_transport_failure_retries_and_an_unknown_service_is_terminal() {
        assert_eq!(disposition_of(&ProxyError::Transport("down".into())), Disposition::Retry);
        assert_eq!(
            disposition_of(&ProxyError::ServiceNotFound("gone".into())),
            Disposition::Terminal
        );
    }

    /// One broken recipient must not be able to evict another
    /// conversation's dead letters, which is what scoping the cap by
    /// target buys.
    #[test]
    fn the_group_key_is_the_target_not_the_caller() {
        assert_eq!(group_key_of(&call(QueuedTarget::Dependency("chat".into()))), "dep:chat");
        assert_eq!(
            group_key_of(&call(QueuedTarget::Service("did:key:zB".into()))),
            "did:did:key:zB"
        );
        assert_ne!(
            group_key_of(&call(QueuedTarget::Dependency("a".into()))),
            group_key_of(&call(QueuedTarget::Dependency("b".into())))
        );
    }

    /// The payload has to survive a restart as bytes, so it has to
    /// round-trip through the encoding the queue stores it in.
    #[test]
    fn a_queued_call_round_trips_through_its_stored_encoding() {
        let original = call(QueuedTarget::Dependency("chat".into()));
        let bytes = serde_json::to_vec(&original).unwrap();
        let parsed: QueuedCall = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.idempotency_key, "msg-7");
        assert_eq!(parsed.target, QueuedTarget::Dependency("chat".into()));
        assert_eq!(parsed.caller_service_id, "svc-caller");
    }

    /// ADR-0021 §2's no-snapshot rule: the item stores the name the caller
    /// asked for, never the DID it happened to resolve to.
    #[test]
    fn the_queued_payload_stores_the_dependency_name_not_a_resolved_did() {
        let stored = call(QueuedTarget::Dependency("chat".into()));
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("chat"));
        assert!(
            !json.contains("did:key:"),
            "a resolved DID must never reach the stored payload: {json}"
        );
    }
}
