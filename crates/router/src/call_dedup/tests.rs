use std::sync::Arc;

use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;

use super::*;

struct Node {
    guard: Arc<CallDedupGuard>,
    dir: tempfile::TempDir,
}

fn config() -> DedupConfig {
    DedupConfig { ttl_ms: 600_000, claim_window_ms: 60_000, max_rows: 100, max_result_bytes: 64 }
}

/// A node whose registry knows `services` -- which is what makes them
/// deployed services as far as the guard is concerned. Deliberately
/// *no* `state.db` is created: a guest that has never touched its own
/// data layer has none, and the guard must still fence calls to it.
async fn node(encryption: bool, services: &[&str]) -> Node {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(SqliteStorageProvider::new(dir.path(), encryption).expect("provider"));
    let key_store = Arc::new(KeyStore::new());
    if encryption {
        key_store.inject_kek([9u8; 32]).expect("kek");
    }
    let registry = EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
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
    for interface in ["orchestrator", "security", SUPERVISOR_RESERVED_SERVICE_ID] {
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
    let registry = EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
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
    assert!(
        matches!(outcome, GuardOutcome::Refuse(ProxyError::PermissionDenied(_))),
        "an unresolvable DEK must fail closed, got {outcome:?}"
    );
}

/// A storage provider whose DEK resolves fine but whose on-disk
/// per-service directory cannot be found -- `service_db_dir`'s own
/// trait default, which returns `Err`. Every other method is stubbed:
/// `store_for` only ever calls `load_service_dek` and `service_db_dir`,
/// so nothing else may legitimately be reached by this test.
struct DekOnlyStorageProvider {
    inner: Arc<dyn StorageProvider>,
}

#[async_trait::async_trait]
impl StorageProvider for DekOnlyStorageProvider {
    async fn open_service_db(
        &self,
        _service_id: &str,
        _key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn syneroym_data_db::ServiceStore>> {
        unreachable!("store_for never opens a ServiceStore")
    }

    async fn rotate_kek(
        &self,
        _key_store: &Arc<KeyStore>,
        _new_kek: [u8; 32],
    ) -> anyhow::Result<()> {
        unreachable!("store_for never rotates a KEK")
    }

    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
        self.inner.load_service_dek(service_id, key_store).await
    }

    async fn service_exists(&self, _service_id: &str) -> anyhow::Result<bool> {
        unreachable!("store_for never checks service_exists")
    }

    // `service_db_dir` is left at the trait default, which returns
    // `Err` -- exactly the "no on-disk per-service directory" case
    // this test needs, with no override required.

    async fn save_config_generation(
        &self,
        _service_id: &str,
        _config_blob: &str,
    ) -> anyhow::Result<u64> {
        unreachable!("store_for never saves a config generation")
    }

    async fn delete_config_generation(
        &self,
        _service_id: &str,
        _generation: u64,
    ) -> anyhow::Result<()> {
        unreachable!("store_for never deletes a config generation")
    }

    async fn get_config_generation(
        &self,
        _service_id: &str,
        _generation: u64,
    ) -> anyhow::Result<Option<String>> {
        unreachable!("store_for never reads a config generation")
    }

    async fn get_latest_config_generation(
        &self,
        _service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>> {
        unreachable!("store_for never reads a config generation")
    }

    async fn save_messaging_subscription(
        &self,
        _service_id: &str,
        _topic: &str,
    ) -> anyhow::Result<()> {
        unreachable!("store_for never touches messaging subscriptions")
    }

    async fn delete_messaging_subscription(
        &self,
        _service_id: &str,
        _topic: &str,
    ) -> anyhow::Result<()> {
        unreachable!("store_for never touches messaging subscriptions")
    }

    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        _service_id: &str,
    ) -> anyhow::Result<()> {
        unreachable!("store_for never touches messaging subscriptions")
    }

    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
        unreachable!("store_for never touches messaging subscriptions")
    }

    async fn save_fdae_policy(&self, _service_id: &str, _policy_json: &str) -> anyhow::Result<()> {
        unreachable!("store_for never touches an FDAE policy")
    }

    async fn load_fdae_policy(&self, _service_id: &str) -> anyhow::Result<Option<String>> {
        unreachable!("store_for never touches an FDAE policy")
    }

    async fn delete_fdae_policy(&self, _service_id: &str) -> anyhow::Result<()> {
        unreachable!("store_for never touches an FDAE policy")
    }
}

/// The regression test for the `async_db_location` extraction (2026-08):
/// before it existed, `store_for` mapped `load_service_dek` failing to
/// `PermissionDenied` (fail-closed, by design) but `service_db_dir`
/// failing to `Internal` -- a path/IO problem has no security meaning
/// and must stay retryable. The extraction briefly collapsed both into
/// one `anyhow::Result` mapped entirely to `PermissionDenied`, which
/// would have made a transient directory error dead-letter the
/// *sender's* call instead of retrying it (`disposition_of` reads
/// `PermissionDenied` as `Terminal`, `Internal` as `Retry`). Pinned here
/// so the next refactor of `async_db_location` cannot re-flatten it
/// silently.
#[tokio::test]
async fn a_directory_resolution_failure_is_internal_not_permission_denied() {
    let dir = tempfile::tempdir().expect("tempdir");
    let real_provider = Arc::new(SqliteStorageProvider::new(dir.path(), true).expect("provider"));
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([9u8; 32]).expect("kek");
    let provider: Arc<dyn StorageProvider> =
        Arc::new(DekOnlyStorageProvider { inner: real_provider });
    let registry = EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
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
    let guard = CallDedupGuard::new(provider, key_store, registry, config());

    let outcome = guard.begin("svc-a", "greeter", Some("did:key:zC"), Some("k1")).await;
    assert!(
        matches!(outcome, GuardOutcome::Refuse(ProxyError::Internal(_))),
        "a directory-resolution failure with a resolvable DEK must be `Internal` (retryable), not \
         `PermissionDenied` (terminal) -- got {outcome:?}"
    );
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

    let executing =
        [&first, &second].iter().filter(|o| matches!(o, GuardOutcome::Execute(Some(_)))).count();
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
