//! The receiver-side idempotency fence, shared by both dispatch entry
//! points (see
//! [ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md),
//! section 4).
//!
//! A call reaches a target service two ways -- `ProxyRouter::invoke_local`
//! for a service on this node, and the route handler's
//! `dispatch_json_rpc_once` for one arriving over the wire -- and the second
//! never passes through the proxy at all. A fence on only one of them is not
//! a fence: every caller that actually needed a durable queue is remote. So
//! both call this one guard, driven by what is in the request body, with no
//! per-ingress special case. Traffic arriving through the HTTP bridge is
//! covered for free, since both of its paths forward into the second entry
//! point.
//!
//! **Fail closed.** A keyed call whose dedup store cannot be opened is
//! refused, not executed. Executing it would be an at-least-once delivery
//! with no fence, which is the one thing this mechanism exists to prevent.
//! The four no-store cases are not the same, though, and only three of them
//! refuse: a *locked vault* and an *I/O error* and a *node with no storage
//! provider* all refuse, while *encryption disabled for the whole
//! deployment* opens the store unencrypted -- exactly as `state.db` is in
//! that mode. Matching the surrounding data's protection is the whole
//! requirement; exceeding it was never asked for.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use syneroym_async_queue::{
    CALL_ALREADY_RUNNING_RPC_CODE, CALL_RESULT_NOT_RETAINED_RPC_CODE, ClaimToken, DedupConfig,
    DedupDecision, DedupStore, DedupWindow, FirstOutcome,
};
use syneroym_core::{
    local_registry::{self, EndpointRegistry},
    mailbox::MAILBOX_SENDER_MAX_AGE_SECS,
};
use syneroym_data_db::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mailbox_format::MAX_CLOCK_SKEW_SECS;
use syneroym_rpc::{NODE_LOCKED_RPC_CODE, ProxyError};
use tokio::task;
use tracing::{info, warn};

use crate::service_async_db::{AsyncDbLocationError, async_db_location};

/// The database a service's outbox, dead letters, and dedup records all
/// live in, beside its own `state.db` and protected by the same key.
pub const ASYNC_DB_NAME: &str = "async.db";

/// The row cap per caller for a service with a mailbox. A message may reach
/// such a service directly and again from its mailbox, days apart.
const MAILBOX_DEDUP_MAX_ROWS: u32 = 100_000;

/// The live rows in the whole fence of a service with a mailbox at which the
/// long window closes. Callers cost nothing to make, so the per-caller cap
/// alone does not bound the store. Past this, new calls get the node's
/// default window, which is what every service had before it had a mailbox.
const MAILBOX_DEDUP_MAX_TOTAL_ROWS: u32 = 200_000;

/// What the guard decided about one incoming request.
#[derive(Debug)]
pub enum GuardOutcome {
    /// Nothing to fence, or this attempt owns the key. Run the call. A
    /// `Some` claim must be settled with [`DedupClaim::settle`] once the
    /// outcome is known.
    Execute(Option<DedupClaim>),
    /// The call already ran here. Answer with this; do not execute.
    Answer(FirstOutcome),
    /// This node cannot honour the guarantee the key promises, so it
    /// refuses rather than executing unfenced.
    Refuse(ProxyError),
}

/// A held claim on `(caller, key)`, owed a settlement.
#[derive(Debug)]
pub struct DedupClaim {
    store: DedupStore,
    caller: String,
    key: String,
    /// Identifies *this* attempt's hold, so a settlement that arrives
    /// after the claim was retaken cannot stamp over the newer one.
    token: ClaimToken,
}

impl DedupClaim {
    /// Records what the call answered, so a duplicate is served from the
    /// record instead of re-executed.
    ///
    /// A failure that **provably** never reached the target releases the
    /// claim instead: nothing ran, so nothing should be remembered, and
    /// holding it would block a corrected retry for a whole claim window.
    ///
    /// "Provably" is the load-bearing word, and the set is deliberately
    /// narrow. A timeout is *not* in it: the dispatch is wrapped in a
    /// timeout, so the deadline fires around a call that may still be
    /// running inside the target. Neither is an unreadable response frame,
    /// which the target produced by definition. Releasing on either would
    /// let the retry run the target a second time -- on the failure mode
    /// most likely to cause a retry in the first place.
    ///
    /// So anything ambiguous keeps its claim and lets the window expire on
    /// its own. The cost of holding a claim wrongly is one window of
    /// "already running here" answers, which a sender retries; the cost of
    /// releasing one wrongly is a double execution, which is the single
    /// thing this fence exists to prevent.
    pub async fn settle(self, outcome: &Result<serde_json::Value, ProxyError>) {
        enum Settlement {
            Record(FirstOutcome),
            Release,
            LeaveInFlight,
        }
        let settlement = match outcome {
            Ok(value) => match serde_json::to_vec(value) {
                Ok(bytes) => Settlement::Record(FirstOutcome::Success(bytes)),
                // The target answered, but its own value will not
                // round-trip. Recording it as done-without-body keeps
                // "not re-executed" true, which is the half that matters.
                Err(_) => Settlement::Record(FirstOutcome::SuccessNotRetained),
            },
            Err(ProxyError::Callee { code, message, .. }) => {
                Settlement::Record(FirstOutcome::CalleeError {
                    code: *code,
                    message: message.clone(),
                })
            }
            Err(e) if precedes_execution(e) => Settlement::Release,
            Err(_) => Settlement::LeaveInFlight,
        };
        let now = now_ms();
        let recorded = task::spawn_blocking(move || match settlement {
            Settlement::Record(outcome) => {
                self.store.finish(&self.caller, &self.key, self.token, &outcome, now)
            }
            Settlement::Release => self.store.release(&self.caller, &self.key, self.token),
            Settlement::LeaveInFlight => Ok(()),
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|inner| inner);
        if let Err(e) = recorded {
            warn!(error = %e, "failed to record a call's dedup outcome");
        }
    }
}

/// Whether `error` is one this node raised *before* the target could have
/// started running.
///
/// Everything here is decided by the router or the route handler on the
/// way in -- a lookup miss, a gate, an unusable target kind, a store that
/// would not open. Deliberately excludes `Transport` and `Timeout`: both
/// can be reported around a call that is already executing.
fn precedes_execution(error: &ProxyError) -> bool {
    matches!(
        error,
        ProxyError::ServiceNotFound(_)
            | ProxyError::PermissionDenied(_)
            | ProxyError::UnsupportedTarget(_)
            | ProxyError::UnsupportedProtocol(_)
            | ProxyError::Internal(_)
    )
}

/// Turns a stored first outcome back into the answer the duplicate gets.
pub fn replay_as_result(outcome: FirstOutcome) -> Result<serde_json::Value, ProxyError> {
    match outcome {
        FirstOutcome::Success(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            ProxyError::Internal(format!("stored result for a repeated call is unreadable: {e}"))
        }),
        FirstOutcome::CalleeError { code, message } => {
            Err(ProxyError::Callee { code, message, data: None })
        }
        FirstOutcome::SuccessNotRetained => Err(ProxyError::Callee {
            code: CALL_RESULT_NOT_RETAINED_RPC_CODE,
            message: "this call already ran here; its result was too large to retain".to_string(),
            data: None,
        }),
    }
}

/// The guard itself: one per node, holding a connection per target service.
pub struct CallDedupGuard {
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    /// What this node actually hosts. The deployed-service check reads
    /// this rather than the filesystem -- see `store_for`.
    registry: EndpointRegistry,
    config: DedupConfig,
    /// Opened once per service and reused. Without this every keyed call
    /// would pay a SQLCipher key derivation, which is a real cost on the
    /// hot path and one no structural test notices unless it asks.
    /// Bounded by the number of services on the node.
    stores: Mutex<HashMap<String, DedupStore>>,
    /// Services whose long window is closed, so the warning is made once for
    /// each time it closes and not for each call.
    closed_long_windows: Mutex<HashSet<String>>,
    /// The size of `closed_long_windows`, so a call can skip the lock when
    /// nothing is closed.
    closed_long_window_count: AtomicUsize,
    /// Serialises the *opening* of a store, so two callers that miss the
    /// cache at the same moment cannot each build an independent handle to
    /// the same file. Two handles are two connections, and the claim's
    /// atomicity would then rest on nothing. Opens are once-per-service,
    /// so a single node-wide lock costs nothing worth optimising.
    open_lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for CallDedupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallDedupGuard").finish_non_exhaustive()
    }
}

impl CallDedupGuard {
    #[must_use]
    pub fn new(
        storage_provider: Arc<dyn StorageProvider>,
        key_store: Arc<KeyStore>,
        registry: EndpointRegistry,
        config: DedupConfig,
    ) -> Self {
        Self {
            storage_provider,
            key_store,
            registry,
            config,
            stores: Mutex::new(HashMap::new()),
            closed_long_windows: Mutex::new(HashSet::new()),
            closed_long_window_count: AtomicUsize::new(0),
            open_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn cached_store(&self, service_id: &str) -> Result<Option<DedupStore>, ProxyError> {
        Ok(self
            .stores
            .lock()
            .map_err(|_| ProxyError::Internal("dedup store cache poisoned".to_string()))?
            .get(service_id)
            .cloned())
    }

    /// How many per-service connections this guard currently holds --
    /// the cache's own assertion, and the "an unkeyed call opens nothing"
    /// one.
    #[cfg(test)]
    fn cached_store_count(&self) -> usize {
        self.stores.lock().expect("dedup store cache poisoned").len()
    }

    /// The error for a service key that cannot be resolved. A node that holds
    /// no KEK yet is only waiting for its operator, so the caller is told to
    /// try again later (`NODE_LOCKED_RPC_CODE`); a durable outbox would
    /// otherwise dead-letter the call for good during the first seconds of a
    /// boot. Any other key failure fails closed.
    ///
    /// `was_locked` is read before the key is resolved. A KEK that loads
    /// after the failed attempt must not turn a locked answer into a final
    /// one, and a KEK cannot be unloaded, so a failure with a key present at
    /// the start is a real one.
    fn dek_failure(was_locked: bool, error: &anyhow::Error) -> ProxyError {
        let message = format!("dedup store unavailable: {error}");
        if was_locked {
            ProxyError::Callee { code: NODE_LOCKED_RPC_CODE, message, data: None }
        } else {
            ProxyError::PermissionDenied(message)
        }
    }

    /// Opens (once) the dedup store for `service_id`, on that service's own
    /// database with that service's own key.
    async fn store_for(&self, service_id: &str) -> Result<DedupStore, ProxyError> {
        if let Some(store) = self.cached_store(service_id)? {
            return Ok(store);
        }

        // Missed the cache. Serialise from here, and look again once the
        // lock is held: whoever went first has already published theirs,
        // and a second handle to the same file would undo the claim's
        // atomicity.
        let _opening = self.open_lock.lock().await;
        if let Some(store) = self.cached_store(service_id)? {
            return Ok(store);
        }

        // Before any DEK is resolved: the key layer generates one on first
        // use, so asking about a service that does not exist would create
        // the very thing the check is meant to establish is absent.
        //
        // The endpoint registry is the authority for "is a deployed
        // service on this node", *not* whether it has a `state.db`. A
        // guest that has never touched its own data layer has no
        // `state.db` at all, so keying this off storage refuses exactly
        // the ordinary service a keyed call is most likely aimed at.
        if self.registry.lookup_by_service(service_id).is_empty() {
            return Err(ProxyError::ServiceNotFound(service_id.to_string()));
        }

        let was_locked = !self.key_store.kek_is_loaded();
        let (dir, dek) = async_db_location(&self.storage_provider, &self.key_store, service_id)
            .await
            .map_err(|e| match e {
                // The DEK is unavailable (typically a locked vault): fail
                // closed by design, per this module's own doc comment --
                // a keyed call answered without being able to check the
                // fence is worse than one refused.
                AsyncDbLocationError::Dek(e) => Self::dek_failure(was_locked, &e),
                // A path/IO problem resolving the directory has no
                // security meaning and is not a settled answer -- terminal
                // here would dead-letter a transient failure at the
                // sender's outbox instead of retrying it.
                AsyncDbLocationError::Dir(e) => {
                    ProxyError::Internal(format!("dedup store unavailable: {e}"))
                }
            })?;

        let config = self.config.clone();
        let store = task::spawn_blocking(move || -> anyhow::Result<DedupStore> {
            std::fs::create_dir_all(&dir)?;
            DedupStore::open_encrypted(&dir, ASYNC_DB_NAME, dek.as_deref(), config)
        })
        .await
        .map_err(|e| ProxyError::Internal(format!("dedup store task failed: {e}")))?
        .map_err(|e| ProxyError::Internal(format!("dedup store unavailable: {e}")))?;

        self.stores
            .lock()
            .map_err(|_| ProxyError::Internal("dedup store cache poisoned".to_string()))?
            .insert(service_id.to_string(), store.clone());
        Ok(store)
    }

    /// Whether `target_service`'s fence already holds a settled record for
    /// `(caller, key)`. Test-only: reuses the same cached store handle
    /// `begin`/`store_for` do rather than opening a second connection to
    /// the file (two handles to one file is the exact hazard this module's
    /// own docs warn about), and a hit on an already-`Done` record returns
    /// before `begin`'s own claiming write, so the probe has no side
    /// effect.
    #[cfg(test)]
    pub(crate) async fn debug_has_settled_key(
        &self,
        target_service: &str,
        caller: &str,
        key: &str,
    ) -> bool {
        let Ok(store) = self.store_for(target_service).await else { return false };
        matches!(
            store.begin(caller, key, now_ms(), self.window_for(target_service)),
            Ok(DedupDecision::Replay(_))
        )
    }

    /// The fence window for `target_service`: the node's own default, or a
    /// longer one when the service has a mailbox. A sender may still try a
    /// message to it, directly or by deposit, for its whole age limit.
    fn window_for(&self, target_service: &str) -> DedupWindow {
        let base = self.config.window();
        let Some(mailbox) = self.registry.mailbox_of(target_service) else { return base };
        let wait_secs = mailbox.max_ttl_secs.max(MAILBOX_SENDER_MAX_AGE_SECS);
        let ttl_ms = wait_secs.saturating_add(MAX_CLOCK_SKEW_SECS).saturating_mul(1000);
        DedupWindow {
            ttl_ms: ttl_ms.max(base.ttl_ms),
            max_rows: MAILBOX_DEDUP_MAX_ROWS.max(base.max_rows),
            max_total_rows: MAILBOX_DEDUP_MAX_TOTAL_ROWS,
        }
    }

    /// Makes a closed long window visible. While it is closed, a service
    /// with a mailbox has only the default window, and nothing else says so.
    /// The lock is taken only when the state may change: when the window is
    /// closed, or when some service is noted as closed.
    fn report_long_window(&self, target_service: &str, store: &DedupStore) {
        let closed_now = store.long_window_closed();
        if !closed_now && self.closed_long_window_count.load(Ordering::Relaxed) == 0 {
            return;
        }
        // Only a report: a poisoned lock just skips it.
        let Ok(mut closed) = self.closed_long_windows.lock() else { return };
        if closed_now {
            metrics::counter!("substrate.proxy.dedup.long_window_closed").increment(1);
            if closed.insert(target_service.to_string()) {
                warn!(
                    target_service,
                    "the receiver fence holds too many records for a long window; new calls get \
                     the default window until records expire"
                );
            }
        } else if closed.remove(target_service) {
            info!(target_service, "the receiver fence has room for a long window again");
        }
        self.closed_long_window_count.store(closed.len(), Ordering::Relaxed);
    }

    /// The one entry point both dispatch sites call.
    ///
    /// `caller` is the identity this node already verified: a DID for a
    /// remote caller presenting an instance certificate, or
    /// `system:<service id>` for a same-node one. `None` means the caller
    /// is anonymous, which has no namespace at all -- two different callers
    /// sharing it would read each other's stored results, so a keyed call
    /// from one is refused rather than filed under a shared name.
    pub async fn begin(
        &self,
        target_service: &str,
        interface: &str,
        caller: Option<&str>,
        key: Option<&str>,
    ) -> GuardOutcome {
        // The load-bearing budget property: a call with no key never opens
        // a store, never resolves a DEK, and never touches the filesystem.
        // Every call on the hot path today is one of these.
        let Some(key) = key else { return GuardOutcome::Execute(None) };

        let Some(caller) = caller else {
            return GuardOutcome::Refuse(ProxyError::PermissionDenied(
                "an idempotency key needs a verified caller to be scoped to; this call is \
                 anonymous"
                    .to_string(),
            ));
        };

        // `orchestrator`, `security`, the supervisor and the mailbox are
        // node-level interfaces, not deployed services: refused up front, so
        // no key and database get minted for the node's own id. No caller
        // sends a key to one today: the supervisor's own traffic is fenced
        // by generations and epochs, not by keys.
        if local_registry::is_node_level_interface(interface) {
            return GuardOutcome::Refuse(ProxyError::PermissionDenied(format!(
                "node-level interface '{interface}' cannot fence an idempotency key: it is not a \
                 deployed service and has no store to remember one in"
            )));
        }

        let store = match self.store_for(target_service).await {
            Ok(store) => store,
            Err(e) => {
                warn!(target_service, error = %e, "refusing a keyed call: no dedup store");
                metrics::counter!("substrate.proxy.dedup.refused").increment(1);
                return GuardOutcome::Refuse(e);
            }
        };

        let (caller, key) = (caller.to_string(), key.to_string());
        let window = self.window_for(target_service);
        let probe = {
            let (store, caller, key) = (store.clone(), caller.clone(), key.clone());
            // `DedupStore` is synchronous over a file lock. Run inline and
            // it parks a runtime worker thread on that lock, on the hot
            // path -- a worse version of the problem this budget exists to
            // prevent.
            task::spawn_blocking(move || {
                #[cfg(test)]
                record_probe_thread();
                store.begin(&caller, &key, now_ms(), window)
            })
            .await
        };
        if window.ttl_ms > self.config.ttl_ms {
            self.report_long_window(target_service, &store);
        }
        match probe {
            Ok(Ok(DedupDecision::Execute(token))) => {
                GuardOutcome::Execute(Some(DedupClaim { store, caller, key, token }))
            }
            Ok(Ok(DedupDecision::Replay(outcome))) => {
                metrics::counter!("substrate.proxy.dedup.replayed").increment(1);
                GuardOutcome::Answer(outcome)
            }
            Ok(Ok(DedupDecision::AlreadyRunning)) => {
                metrics::counter!("substrate.proxy.dedup.in_flight").increment(1);
                GuardOutcome::Refuse(ProxyError::Callee {
                    code: CALL_ALREADY_RUNNING_RPC_CODE,
                    message: "a call with this idempotency key is already running here".to_string(),
                    data: None,
                })
            }
            Ok(Err(e)) => {
                GuardOutcome::Refuse(ProxyError::Internal(format!("dedup lookup failed: {e}")))
            }
            Err(e) => {
                GuardOutcome::Refuse(ProxyError::Internal(format!("dedup lookup task failed: {e}")))
            }
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Records which thread the store probe actually ran on, so a test can
/// assert it was not the async worker's -- the only way to tell an
/// off-thread SQLite call from an inline one from the outside.
#[cfg(test)]
fn record_probe_thread() {
    *PROBE_THREAD.lock().expect("probe thread record poisoned") = Some(std::thread::current().id());
}

#[cfg(test)]
static PROBE_THREAD: Mutex<Option<std::thread::ThreadId>> = Mutex::new(None);

#[cfg(test)]
mod tests;
