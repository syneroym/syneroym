//! The receiver-side idempotency fence, shared by both dispatch entry
//! points ([ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md)
//! §4).
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
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use syneroym_async_queue::{
    CALL_ALREADY_RUNNING_RPC_CODE, CALL_RESULT_NOT_RETAINED_RPC_CODE, DedupConfig, DedupDecision,
    DedupStore, FirstOutcome,
};
use syneroym_core::local_registry::{EndpointRegistry, NODE_NATIVE_INTERFACES};
use syneroym_data_db::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::ProxyError;
use tokio::task;
use tracing::warn;

/// The database a service's outbox, dead letters, and dedup records all
/// live in, beside its own `state.db` and protected by the same key.
pub const ASYNC_DB_NAME: &str = "async.db";

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
                self.store.finish(&self.caller, &self.key, &outcome, now)
            }
            Settlement::Release => self.store.release(&self.caller, &self.key),
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

/// Whether `interface` is one of the node's own interfaces rather than a
/// deployed service's.
///
/// These have no DEK and no directory, and asking for one anyway *works*:
/// the id passes validation and the key layer generates a DEK on first
/// use, so a naive caller would mint a key and a database for a service
/// that does not exist. Matched by short hash as well as by name, since
/// the hash is an unsalted prefix a guest can compute for itself.
pub(crate) fn is_node_level_interface(interface: &str) -> bool {
    NODE_NATIVE_INTERFACES
        .iter()
        .any(|name| *name == interface || syneroym_core::util::short_hash(name) == interface)
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

    /// Whether a keyed call to `(target_service, interface)` can be fenced
    /// at all.
    ///
    /// `orchestrator`, `security`, and the supervisor's dispatch id are
    /// node-level interfaces, not deployed services: they have no DEK and
    /// no directory, and asking for one anyway *works* -- the id passes
    /// validation and the key layer generates a DEK on first use -- so a
    /// naive guard would mint a key and a database for a service that does
    /// not exist before refusing the call. Refused up front instead. No
    /// caller sends a key to one today: the supervisor's own traffic is
    /// fenced by generations and epochs, not by keys.
    fn is_node_level_interface(interface: &str) -> bool {
        is_node_level_interface(interface)
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

        let dek =
            self.storage_provider.load_service_dek(service_id, &self.key_store).await.map_err(
                |e| ProxyError::PermissionDenied(format!("dedup store unavailable: {e}")),
            )?;
        let dir: PathBuf = self
            .storage_provider
            .service_db_dir(service_id)
            .map_err(|e| ProxyError::Internal(format!("dedup store unavailable: {e}")))?;

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

        if Self::is_node_level_interface(interface) {
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
        let probe = {
            let (store, caller, key) = (store.clone(), caller.clone(), key.clone());
            // `DedupStore` is synchronous over a file lock. Run inline and
            // it parks a runtime worker thread on that lock, on the hot
            // path -- a worse version of the problem this budget exists to
            // prevent.
            task::spawn_blocking(move || {
                #[cfg(test)]
                record_probe_thread();
                store.begin(&caller, &key, now_ms())
            })
            .await
        };
        match probe {
            Ok(Ok(DedupDecision::Execute)) => {
                GuardOutcome::Execute(Some(DedupClaim { store, caller, key }))
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
mod tests {
    use std::sync::Arc;

    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    use super::*;

    struct Node {
        guard: Arc<CallDedupGuard>,
        dir: tempfile::TempDir,
    }

    fn config() -> DedupConfig {
        DedupConfig {
            ttl_ms: 600_000,
            claim_window_ms: 60_000,
            max_rows: 100,
            max_result_bytes: 64,
        }
    }

    /// A node whose registry knows `services` -- which is what makes them
    /// deployed services as far as the guard is concerned. Deliberately
    /// *no* `state.db` is created: a guest that has never touched its own
    /// data layer has none, and the guard must still fence calls to it.
    async fn node(encryption: bool, services: &[&str]) -> Node {
        let dir = tempfile::tempdir().expect("tempdir");
        let provider =
            Arc::new(SqliteStorageProvider::new(dir.path(), encryption).expect("provider"));
        let key_store = Arc::new(KeyStore::new());
        if encryption {
            key_store.inject_kek([9u8; 32]).expect("kek");
        }
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
        for service in services {
            registry
                .register(
                    (*service).to_string(),
                    "greeter".to_string(),
                    syneroym_core::local_registry::SubstrateEndpoint::WasmChannel {
                        service_id: (*service).to_string(),
                    },
                )
                .await
                .expect("register");
        }
        Node { guard: Arc::new(CallDedupGuard::new(provider, key_store, registry, config())), dir }
    }

    fn async_db_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let services = root.join("services");
        let Ok(entries) = std::fs::read_dir(&services) else { return Vec::new() };
        entries
            .filter_map(Result::ok)
            .map(|e| e.path().join(ASYNC_DB_NAME))
            .filter(|p| p.exists())
            .collect()
    }

    /// The anonymous namespace is shared, so two different callers using
    /// the same key string would read each other's stored results. There
    /// is no safe place to file this call.
    #[tokio::test]
    async fn a_keyed_call_from_an_unidentified_caller_is_refused() {
        let node = node(false, &["svc-a"]).await;
        let outcome = node.guard.begin("svc-a", "greeter", None, Some("k1")).await;
        assert!(
            matches!(outcome, GuardOutcome::Refuse(ProxyError::PermissionDenied(_))),
            "got {outcome:?}"
        );
        assert!(async_db_files(node.dir.path()).is_empty());
    }

    /// The dangerous half of this case is the file that would be written
    /// before the refusal: the key layer generates a DEK on first use, and
    /// these ids pass validation, so asking about them at all mints a key
    /// and a database for a service that does not exist.
    #[tokio::test]
    async fn a_keyed_call_to_a_node_level_interface_is_refused_and_creates_no_database() {
        let node = node(true, &[]).await;
        for interface in ["orchestrator", "security"] {
            let outcome = node
                .guard
                .begin("did:key:zNodeItself", interface, Some("did:key:zC"), Some("k1"))
                .await;
            assert!(
                matches!(outcome, GuardOutcome::Refuse(ProxyError::PermissionDenied(_))),
                "interface '{interface}' must be refused, got {outcome:?}"
            );
        }
        assert!(
            !node.dir.path().join("services").join("did:key:zNodeItself").exists(),
            "no directory may be created for a service that does not exist"
        );
    }

    /// The store opened is the *target's*, so one service's records never
    /// answer for another's.
    #[tokio::test]
    async fn a_key_is_scoped_to_its_target_service() {
        let node = node(false, &["svc-a", "svc-b"]).await;
        let first = node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await;
        let GuardOutcome::Execute(Some(claim)) = first else { panic!("expected a claim") };
        claim.settle(&Ok(serde_json::json!("from-a"))).await;

        let other = node.guard.begin("svc-b", "greeter", Some("did:key:zC"), Some("k1")).await;
        assert!(
            matches!(other, GuardOutcome::Execute(_)),
            "the same key against a different target is a different call, got {other:?}"
        );
    }

    /// Executing without a dedup check is executing an at-least-once
    /// delivery with no fence at all, which is the one thing this guard
    /// exists to prevent -- so a store it cannot open refuses the call.
    #[tokio::test]
    async fn a_keyed_call_is_refused_when_the_dedup_store_cannot_be_opened() {
        let dir = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(SqliteStorageProvider::new(dir.path(), true).expect("provider"));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
        registry
            .register(
                "svc-a".to_string(),
                "greeter".to_string(),
                syneroym_core::local_registry::SubstrateEndpoint::WasmChannel {
                    service_id: "svc-a".to_string(),
                },
            )
            .await
            .expect("register");
        // No KEK injected: the vault is locked, exactly as it is after
        // every substrate restart until an operator injects one.
        let guard = CallDedupGuard::new(provider, Arc::new(KeyStore::new()), registry, config());

        let outcome = guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await;
        assert!(matches!(outcome, GuardOutcome::Refuse(_)), "got {outcome:?}");
    }

    /// Not one of the refusing cases: with encryption off for the whole
    /// deployment the queue file is plain SQLite, exactly as `state.db` is
    /// then. Matching the surrounding data's protection is all that was
    /// ever asked for.
    #[tokio::test]
    async fn a_keyed_call_works_with_encryption_disabled_for_the_deployment() {
        let node = node(false, &["svc-a"]).await;
        let outcome = node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await;
        assert!(matches!(outcome, GuardOutcome::Execute(Some(_))), "got {outcome:?}");
    }

    /// The load-bearing budget assertion, stated as "untouched" rather
    /// than as a timing so it cannot pass by being fast on a quick machine.
    #[tokio::test]
    async fn a_call_with_no_idempotency_key_never_opens_a_dedup_store() {
        let node = node(true, &["svc-a"]).await;
        for _ in 0..5 {
            let outcome = node.guard.begin("svc-a", "greeter", Some("did:key:zC"), None).await;
            assert!(matches!(outcome, GuardOutcome::Execute(None)));
        }
        assert!(
            async_db_files(node.dir.path()).is_empty(),
            "an unkeyed call must not open, or create, any dedup store"
        );
        assert_eq!(node.guard.cached_store_count(), 0);
    }

    /// Without the cache every keyed call pays a SQLCipher key
    /// derivation, which is the cost that makes a timed test meaningless
    /// and which no other assertion here would notice.
    #[tokio::test]
    async fn a_second_keyed_call_to_one_target_opens_no_second_connection() {
        let node = node(true, &["svc-a"]).await;
        node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await;
        node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k2")).await;
        assert_eq!(
            node.guard.cached_store_count(),
            1,
            "the second keyed call to one target must reuse the first call's connection"
        );
    }

    /// Two duplicates arriving at once through a *cold* cache. The
    /// dangerous shape: both miss the store cache, and without
    /// single-flighting they each build an independent handle to the same
    /// file, so both read "no row" before either writes and both are told
    /// to execute. Exactly one may win.
    #[tokio::test]
    async fn two_concurrent_duplicates_through_a_cold_cache_produce_one_claim() {
        let node = node(false, &["svc-a"]).await;
        assert_eq!(node.guard.cached_store_count(), 0, "the cache must start cold");

        let (first, second) = tokio::join!(
            node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")),
            node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")),
        );

        let executing = [&first, &second]
            .iter()
            .filter(|o| matches!(o, GuardOutcome::Execute(Some(_))))
            .count();
        assert_eq!(
            executing, 1,
            "exactly one of two concurrent duplicates may execute, got {first:?} and {second:?}"
        );
        assert_eq!(node.guard.cached_store_count(), 1, "and only one store may have been opened");
    }

    /// A timeout fires *around* a dispatch that may still be running
    /// inside the target, so it must not release the claim -- releasing it
    /// lets the retry run the target a second time, on the failure mode
    /// most likely to cause a retry.
    #[tokio::test]
    async fn a_timed_out_call_keeps_its_claim_so_a_retry_does_not_re_execute() {
        let node = node(false, &["svc-a"]).await;
        let GuardOutcome::Execute(Some(claim)) =
            node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await
        else {
            panic!("expected a claim");
        };
        claim.settle(&Err(ProxyError::Timeout(std::time::Duration::from_secs(30)))).await;

        assert!(
            matches!(
                node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await,
                GuardOutcome::Refuse(ProxyError::Callee { code, .. })
                    if code == syneroym_async_queue::CALL_ALREADY_RUNNING_RPC_CODE
            ),
            "a retry after a timeout must be told the call is still running, not handed the key"
        );
    }

    /// The target definitely ran -- it produced a frame we could not read
    /// -- so this is the same rule as the timeout.
    #[tokio::test]
    async fn an_unreadable_response_keeps_its_claim() {
        let node = node(false, &["svc-a"]).await;
        let GuardOutcome::Execute(Some(claim)) =
            node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await
        else {
            panic!("expected a claim");
        };
        claim.settle(&Err(ProxyError::Transport("unreadable response frame".to_string()))).await;

        assert!(
            matches!(
                node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await,
                GuardOutcome::Refuse(_)
            ),
            "an unreadable response must not free the key for a second execution"
        );
    }

    /// The boundary: a refusal raised before anything was dispatched must
    /// still release, or a corrected retry is blocked for a whole window.
    #[tokio::test]
    async fn a_pre_dispatch_refusal_still_releases_its_claim() {
        let node = node(false, &["svc-a"]).await;
        let GuardOutcome::Execute(Some(claim)) =
            node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await
        else {
            panic!("expected a claim");
        };
        claim.settle(&Err(ProxyError::PermissionDenied("denied".to_string()))).await;

        assert!(
            matches!(
                node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await,
                GuardOutcome::Execute(Some(_))
            ),
            "a corrected retry must not be blocked by a claim nothing executed under"
        );
    }

    /// SQLite here is synchronous over a file lock. Run inline on the
    /// async worker and it parks that worker on the lock, on the hot path
    /// -- invisible to every other assertion in this file, so it gets its
    /// own. A `current_thread` runtime has exactly one async worker, so a
    /// differing thread id is proof the work moved to the blocking pool.
    #[test]
    fn the_guard_never_runs_sqlite_on_the_async_worker_thread() {
        let runtime =
            tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        runtime.block_on(async {
            let node = node(false, &["svc-a"]).await;
            node.guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await;
            let probe = PROBE_THREAD.lock().expect("probe record").take();
            assert!(
                probe.is_some_and(|id| id != std::thread::current().id()),
                "the store probe must not run on the async worker thread"
            );
        });
    }
}
