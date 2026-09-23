#![allow(clippy::cognitive_complexity)]

use std::{env, fs, sync::atomic::AtomicBool};

use serde_json::Number;
use syneroym_core::{storage::MockStorage, test_constants};
use syneroym_data_db::{ServiceStore, SqliteStorageProvider};
use syneroym_mqtt_broker::MqttBrokerConfig;
use syneroym_rpc::{Ability, AuthLevel, Capability, ResourceUri, SessionContext};
use tokio::sync::Notify;
use wasmtime::component::Component;

use super::*;
use crate::host_capabilities::tests::{
    test_blob_provider, test_messaging_context, test_service_proxy, test_streaming_context,
};

/// Wraps a real `StorageProvider`, pausing `load_fdae_policy` on
/// `release` before delegating -- lets a test deterministically land a
/// `bump_fdae_policy_generation` call inside `resolve_fdae_policy`'s
/// cross-await race window, rather than relying on incidental thread
/// scheduling (which would make the test flaky in either direction).
/// Every other method delegates straight through; `resolve_fdae_policy`
/// only ever calls `load_fdae_policy`.
struct RacingStorageProvider {
    inner: Arc<dyn StorageProvider>,
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl StorageProvider for RacingStorageProvider {
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn ServiceStore>> {
        self.inner.open_service_db(service_id, key_store).await
    }
    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()> {
        self.inner.rotate_kek(key_store, new_kek).await
    }
    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
        self.inner.load_service_dek(service_id, key_store).await
    }
    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
        self.inner.service_exists(service_id).await
    }
    async fn save_config_generation(
        &self,
        service_id: &str,
        config_blob: &str,
    ) -> anyhow::Result<u64> {
        self.inner.save_config_generation(service_id, config_blob).await
    }
    async fn delete_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<()> {
        self.inner.delete_config_generation(service_id, generation).await
    }
    async fn get_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<String>> {
        self.inner.get_config_generation(service_id, generation).await
    }
    async fn get_latest_config_generation(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>> {
        self.inner.get_latest_config_generation(service_id).await
    }
    async fn save_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        self.inner.save_messaging_subscription(service_id, topic).await
    }
    async fn delete_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        self.inner.delete_messaging_subscription(service_id, topic).await
    }
    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        service_id: &str,
    ) -> anyhow::Result<()> {
        self.inner.delete_all_messaging_subscriptions_for_service(service_id).await
    }
    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.list_all_messaging_subscriptions().await
    }
    async fn save_fdae_policy(&self, service_id: &str, policy_json: &str) -> anyhow::Result<()> {
        self.inner.save_fdae_policy(service_id, policy_json).await
    }
    async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>> {
        self.release.notified().await;
        self.inner.load_fdae_policy(service_id).await
    }
    async fn delete_fdae_policy(&self, service_id: &str) -> anyhow::Result<()> {
        self.inner.delete_fdae_policy(service_id).await
    }
}

/// Reproduces the lost-invalidation race directly: a `resolve_fdae_policy`
/// load is paused (via `RacingStorageProvider`) after it has already
/// captured `generation_before`, a concurrent eviction (simulating a
/// redeploy) fires while it's still in flight, and only then is the load
/// allowed to complete. The in-flight call must still return the correct
/// (if now possibly stale) answer, but must **not** repopulate the cache
/// -- otherwise the eviction it raced would be silently undone, and the
/// stale policy would be served indefinitely until the next
/// `stop_wasm`/redeploy.
#[tokio::test]
async fn fdae_policy_resolution_racing_an_eviction_is_not_cached() {
    let temp_dir = tempfile::tempdir().unwrap();
    let real_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    real_provider
        .save_fdae_policy("svc-race", r#"{"version": "fdae/v1", "definitions": {}}"#)
        .await
        .unwrap();

    let release = Arc::new(Notify::new());
    let racing_provider: Arc<dyn StorageProvider> =
        Arc::new(RacingStorageProvider { inner: real_provider, release: release.clone() });
    let app_engine = Arc::new(test_app_engine(racing_provider));

    let resolver = {
        let app_engine = app_engine.clone();
        tokio::spawn(async move { app_engine.resolve_fdae_policy("svc-race").await })
    };

    // Let the spawned task run up through its `generation_before`
    // snapshot and into `load_fdae_policy`'s `release.notified().await`
    // suspension point, before the eviction below fires.
    task::yield_now().await;
    task::yield_now().await;

    // The eviction half of a concurrent redeploy, landing while the load
    // above is still paused.
    app_engine.fdae_policies.remove("svc-race");
    app_engine.bump_fdae_policy_generation("svc-race");

    release.notify_one();
    let resolved = resolver.await.unwrap();

    assert!(resolved.is_some(), "the in-flight call must still return the correct answer");
    assert!(
        app_engine.fdae_policies.get("svc-race").is_none(),
        "a load that raced a concurrent eviction must not repopulate the cache -- doing so would \
         silently undo the eviction and serve a possibly-stale policy indefinitely"
    );
}

/// Wraps a real `StorageProvider`, failing `load_fdae_policy` exactly
/// once (then delegating normally) -- simulates a transient storage
/// error, like a busy connection under load, that clears up on retry.
struct FlakyStorageProvider {
    inner: Arc<dyn StorageProvider>,
    fail_next: AtomicBool,
}

#[async_trait::async_trait]
impl StorageProvider for FlakyStorageProvider {
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn ServiceStore>> {
        self.inner.open_service_db(service_id, key_store).await
    }
    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()> {
        self.inner.rotate_kek(key_store, new_kek).await
    }
    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
        self.inner.load_service_dek(service_id, key_store).await
    }
    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
        self.inner.service_exists(service_id).await
    }
    async fn save_config_generation(
        &self,
        service_id: &str,
        config_blob: &str,
    ) -> anyhow::Result<u64> {
        self.inner.save_config_generation(service_id, config_blob).await
    }
    async fn delete_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<()> {
        self.inner.delete_config_generation(service_id, generation).await
    }
    async fn get_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<String>> {
        self.inner.get_config_generation(service_id, generation).await
    }
    async fn get_latest_config_generation(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>> {
        self.inner.get_latest_config_generation(service_id).await
    }
    async fn save_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        self.inner.save_messaging_subscription(service_id, topic).await
    }
    async fn delete_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        self.inner.delete_messaging_subscription(service_id, topic).await
    }
    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        service_id: &str,
    ) -> anyhow::Result<()> {
        self.inner.delete_all_messaging_subscriptions_for_service(service_id).await
    }
    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.list_all_messaging_subscriptions().await
    }
    async fn save_fdae_policy(&self, service_id: &str, policy_json: &str) -> anyhow::Result<()> {
        self.inner.save_fdae_policy(service_id, policy_json).await
    }
    async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            anyhow::bail!("simulated transient storage failure");
        }
        self.inner.load_fdae_policy(service_id).await
    }
    async fn delete_fdae_policy(&self, service_id: &str) -> anyhow::Result<()> {
        self.inner.delete_fdae_policy(service_id).await
    }
}

/// A transient storage error (e.g. one `SQLITE_BUSY`) must not be
/// remembered as "this service has no policy" -- unlike a genuinely
/// absent or malformed row, it says nothing about whether a policy
/// exists, and caching it as absent would silently disable FDAE for the
/// service until the next redeploy over what may be a one-off blip.
#[tokio::test]
async fn fdae_policy_transient_storage_error_is_not_cached() {
    let temp_dir = tempfile::tempdir().unwrap();
    let real_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    real_provider
        .save_fdae_policy("svc-flaky", r#"{"version": "fdae/v1", "definitions": {}}"#)
        .await
        .unwrap();
    let flaky_provider: Arc<dyn StorageProvider> =
        Arc::new(FlakyStorageProvider { inner: real_provider, fail_next: AtomicBool::new(true) });
    let app_engine = test_app_engine(flaky_provider);

    // First resolution hits the simulated transient failure.
    assert!(
        app_engine.resolve_fdae_policy("svc-flaky").await.is_none(),
        "a storage error must resolve to None for this call, same as a genuine absence"
    );
    assert!(
        app_engine.fdae_policies.get("svc-flaky").is_none(),
        "a transient storage error must not be cached as 'no policy'"
    );

    // The failure was one-shot; a retry reaches real storage and finds
    // the policy that was there all along.
    assert!(
        app_engine.resolve_fdae_policy("svc-flaky").await.is_some(),
        "a retry after the transient failure clears must resolve the real policy"
    );
    assert!(app_engine.fdae_policies.get("svc-flaky").is_some());
}

/// `prepare_wasm_execution` is the ordinary dispatch path reached from
/// wire-originated JSON-RPC (`dispatch.rs`) and guest-to-guest proxy
/// calls, both of which let the caller pick `method_name` freely.
/// Naming a request "init" or "migrate" must not synthesize
/// `CallerContext::local_elevated` -- the `data-layer/admin`-bearing
/// context `HostState::query_auth` exempts from the FDAE sieve entirely
/// -- or any caller could self-elevate by choosing that method name.
/// Only `invoke_lifecycle_hook` (called directly by the deploy path,
/// never through this function) may synthesize that context.
#[tokio::test]
async fn prepare_wasm_execution_grants_no_elevation_for_init_or_migrate_method_names() {
    let wat = r#"
(component
  (core module $m
(func (export "noop"))
  )
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface
(export "init" (func $noop))
(export "migrate" (func $noop))
  )
  (export "test-interface" (instance $interface))
)
"#;
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(tempfile::tempdir().unwrap().path(), false).unwrap());
    let app_engine = test_app_engine(storage_provider);
    app_engine.compile_and_cache_wasm("svc-n1", wat.as_bytes().to_vec(), None).await.unwrap();

    for method in ["init", "migrate"] {
        let (store, _func, _results_len, _item) = app_engine
            .prepare_wasm_execution(
                "svc-n1",
                "test-interface",
                method,
                None,
                InvocationOrigin::Local,
            )
            .await
            .unwrap();
        assert_eq!(
            store.data().caller.auth,
            AuthLevel::System,
            "a wire-dispatched call naming its method {method:?} must not be granted \
             LocalElevated -- only invoke_lifecycle_hook may synthesize that context"
        );
        assert!(
            !store.data().caller.caller_did.contains("local-elevated"),
            "caller_did leaked a local-elevated identity for method {method:?}"
        );
    }
}

/// A caller matching `[iam].admin_ucan_root` has always been meant to
/// reach a guest's `execute-ddl`/`query-raw` -- `build_caller`
/// (`crates/router/src/route_handler/io.rs`) issues it a bare
/// `substrate:<node_did>` grant of `substrate/admin`, which
/// `Ability::entails` defines as covering everything on the node
/// (including `data-layer/admin`), and a data-layer lifecycle-hook test
/// already pins that fact against a hand-built `HostState`
/// (ADR-0015/0016).
///
/// Before the wire path forwarded the real caller, that fact was true
/// but practically unreachable from the wire: `prepare_wasm_execution`
/// always synthesized `service_system` (no capabilities at all) for any
/// wire-dispatched call, so no admin-rooted caller's grant could ever
/// actually arrive at `HostState.caller` outside `invoke_lifecycle_hook`
/// (which never calls this function). Forwarding the real caller
/// (`dispatch.rs`'s `JsonRpcToWasm` branch) makes that
/// existing, ADR-accepted admission reachable end to end for the first
/// time -- this test pins it through the real `prepare_wasm_execution`
/// wiring, not a hand-built `HostState`, so a
/// regression in that wiring (or an accidental narrowing that
/// contradicts the ADR) shows up here.
#[tokio::test]
async fn prepare_wasm_execution_forwards_a_wire_admin_caller_that_reaches_guest_execute_ddl() {
    let wat = r#"
(component
  (core module $m
(func (export "noop"))
  )
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface
(export "init" (func $noop))
(export "migrate" (func $noop))
  )
  (export "test-interface" (instance $interface))
)
"#;
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(tempfile::tempdir().unwrap().path(), false).unwrap());
    let app_engine = test_app_engine(storage_provider);
    app_engine
        .compile_and_cache_wasm("svc-admin-ddl", wat.as_bytes().to_vec(), None)
        .await
        .unwrap();

    let admin_did = "did:key:z6MkAdminRootWire";
    let admin_caller = CallerContext {
        caller_did: admin_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: admin_did.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate(admin_did),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    let (mut store, _func, _results_len, _item) = app_engine
        .prepare_wasm_execution(
            "svc-admin-ddl",
            "test-interface",
            "init",
            Some(admin_caller),
            InvocationOrigin::Local,
        )
        .await
        .unwrap();

    store::Host::execute_ddl(store.data_mut(), "CREATE TABLE x (id TEXT)".to_string())
        .await
        .expect(
            "an admin-rooted caller forwarded through the real dispatch wiring must reach guest \
             execute-ddl, matching the ADR-0015/0016 admission model already pinned (against a \
             hand-built HostState) by \
             lifecycle_hooks::test_execute_ddl_allowed_for_admin_ucan_root_caller",
        );
}

#[tokio::test]
async fn test_list_interfaces() {
    // The trailing 0s are unused: `None, None` disables pooling entirely.
    let engine = AppSandboxEngine::build_wasm_engine(None, None, 0, 0, 0).unwrap();
    let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider =
        Arc::new(SqliteStorageProvider::new(tempfile::tempdir().unwrap().path(), false).unwrap());
    let host_state = HostState::new(
        "test_component".to_string(),
        None,
        key_store,
        storage_provider,
        test_blob_provider(),
        CallerContext::service_system("test_component"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let mut store = Store::new(&engine, host_state);

    let component_path = test_constants::greeter_wasm_path();
    let wasm_bytes = if let Ok(bytes) = fs::read(&component_path) {
        bytes
    } else {
        println!(
            "Skipping test_list_interfaces: WASM artifact not found at {}",
            component_path.display()
        );
        return;
    };

    let component: Component =
        Component::new(&engine, &wasm_bytes).expect("Failed to compile WASM component");
    for interface in component.component_type().exports(&engine) {
        println!("Listing interface: {interface:?}");
    }

    match linker.instantiate_async(&mut store, &component).await {
        Ok(instance) => {
            let interface_name = test_constants::GREETER_INTERFACE_NAME;
            let method_name = "greet";

            // Use the helper function to extract function and result size
            match AppSandboxEngine::get_wasm_func(
                &mut store,
                &instance,
                Some(interface_name),
                method_name,
            ) {
                Ok((func, results_len, _item)) => {
                    println!("Function export: {func:?}");
                    let mut wasm_results = vec![Val::Bool(false); results_len];

                    let result = func
                        .call_async(
                            &mut store,
                            &[Val::String("TestUser".to_string())],
                            &mut wasm_results,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to call function: {e}"));
                    println!("Function call result: {result:?} is {wasm_results:?}");
                }
                Err(e) => {
                    println!("Failed to get wasm func: {e}");
                }
            }
        }
        Err(err) => {
            println!("Error instantiating component: {err}");
        }
    }
}

#[tokio::test]
async fn test_wasm_quotas() {
    let wat = r#"
(component
  (core module $m
(func (export "loop_forever")
  (loop $l
    br $l
  )
)
(func (export "allocate_too_much") (param $pages i32) (result i32)
  (memory.grow (local.get $pages))
)
(memory (export "memory") 1)
  )
  (core instance $i (instantiate $m))
  (func $loop_forever (canon lift (core func $i "loop_forever")))
  (func $allocate_too_much (param "pages" u32) (result s32) (canon lift (core func $i "allocate_too_much")))
  (instance $interface
(export "loop-forever" (func $loop_forever))
(export "allocate-too-much" (func $allocate_too_much))
  )
  (export "test-interface" (instance $interface))
)
"#;
    let engine =
        AppSandboxEngine::build_wasm_engine(Some(10), Some(128 * 1024 * 1024), 4, 4, 4).unwrap();
    let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

    let app_engine = AppSandboxEngine {
        blobs_dir: env::temp_dir(),
        engine,
        linker,
        components: DashMap::new(),
        fdae_policies: DashMap::new(),
        fdae_policy_generation: DashMap::new(),
        default_max_instructions: Some(10_000),
        default_max_memory_bytes: Some(1024 * 1024), // 1MB
        guest_websocket_permits: Arc::new(DashMap::new()),
        max_concurrent_websockets_per_service: 10,
        max_sse_subscribers_per_service: 100,
        websocket_senders: {
            let ws = OnceLock::new();
            let _ = ws.set(WebSocketSenders::new());
            ws
        },
        _shutdown_tx: None,
        key_store: Arc::new(KeyStore::new()),
        storage_provider: Arc::new(SqliteStorageProvider::new(env::temp_dir(), false).unwrap()),
        blob_provider: test_blob_provider(),
        messaging_broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        self_weak: OnceLock::new(),
        service_proxy: OnceLock::new(),
        record_signer: OnceLock::new(),
        conversation: OnceLock::new(),
        subscriptions: DashMap::new(),
        endpoint_registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        logical_resolver: syneroym_app_orchestration::empty_resolver(),
        stream_registry: StreamRegistry::new(),
        max_concurrent_streams_per_service: 8,
        stream_instance_permits: Arc::new(Semaphore::new(8)),
        abac_instance_permits: Arc::new(Semaphore::new(1)),
        probe_instance_permits: Arc::new(Semaphore::new(2)),
        dispatch_epoch_ticks: ticks_for_secs(5),
        lifecycle_hook_epoch_ticks: ticks_for_secs(30),
        abac_epoch_ticks: ticks_for_secs(2),
        abac_max_instructions: 50_000_000,
        instantiations: AtomicU64::new(0),
        guest_http_permits: Arc::new(DashMap::new()),
        max_concurrent_guest_http_per_service: 4,
    };

    // Cache the test component
    app_engine.compile_and_cache_wasm("test_service", wat.as_bytes().to_vec(), None).await.unwrap();

    // 1. Test infinite loop (fuel limit)
    let request_loop = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "loop-forever".to_string(),
        params: Value::Array(vec![]),
        id: None,
        idempotency_key: None,
    };
    let res_loop = app_engine.execute_wasm("test_service", "test-interface", &request_loop).await;
    assert!(res_loop.is_err());
    let err_msg = res_loop.unwrap_err().to_string();
    assert!(err_msg.contains("QuotaExceeded"), "expected QuotaExceeded, got: {err_msg}");

    // 2. Test memory allocation limit
    // 1 page is 64KB. We try to allocate 100 pages (6.4MB), which exceeds the 1MB
    // limit.
    let request_mem = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "allocate-too-much".to_string(),
        params: Value::Array(vec![Value::Number(Number::from(100))]),
        id: None,
        idempotency_key: None,
    };
    let res_mem = app_engine.execute_wasm("test_service", "test-interface", &request_mem).await;
    assert!(res_mem.is_err());
    let err_msg = res_mem.unwrap_err().to_string();
    assert!(
        err_msg.contains("MemoryFault") || err_msg.contains("failed to grow memory"),
        "expected MemoryFault or failed to grow memory, got: {err_msg}"
    );
}

/// Reviewer-requested boundary test (PR #136 review): a component that
/// uses *exactly* `max_core_instances_per_component` core-module
/// instantiations -- and, since each declares its own memory and table,
/// exactly `max_memories_per_component`/`max_tables_per_component` too
/// -- must still instantiate successfully, proving `build_wasm_engine`'s
/// per-component pooling limits don't spuriously reject a component that
/// stays within its declared budget. One core module over that limit
/// must fail clearly instead (not silently, and not by exhausting the
/// pool's global totals -- `max_instances` here is large enough that the
/// global budget alone could never be the reason), proving the ceiling
/// is a real, enforced contract and not just documentation.
#[tokio::test]
async fn a_component_at_the_configured_per_component_resource_max_still_instantiates() {
    fn n_module_component_wat(n: u32) -> String {
        let mut wat = String::from("(component\n");
        for i in 0..n {
            wat += &format!(
                "  (core module $m{i}\n    (memory (export \"mem\") 1)\n    (table (export \
                 \"tbl\") 1 funcref)\n    (func (export \"noop\"))\n  )\n"
            );
        }
        for i in 0..n {
            wat += &format!("  (core instance $i{i} (instantiate $m{i}))\n");
        }
        wat += "  (func $noop (canon lift (core func $i0 \"noop\")))\n";
        wat += "  (instance $interface (export \"noop\" (func $noop)))\n";
        wat += "  (export \"test-interface\" (instance $interface))\n)";
        wat
    }

    const PER_COMPONENT_MAX: u32 = 4;

    // `max_instances: 10` keeps the pool's global totals (40 of each
    // resource) far above what either component below needs, so a
    // failure can only come from the per-component ceiling itself.
    let engine = AppSandboxEngine::build_wasm_engine(
        Some(10),
        Some(16 * 1024 * 1024),
        PER_COMPONENT_MAX,
        PER_COMPONENT_MAX,
        PER_COMPONENT_MAX,
    )
    .unwrap();
    let linker = Linker::<()>::new(&engine);

    let at_max = n_module_component_wat(PER_COMPONENT_MAX);
    let component = Component::new(&engine, &at_max)
        .expect("a component using exactly the declared per-component max must compile");
    let mut store = Store::new(&engine, ());
    linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("a component at the declared per-component max must still instantiate");

    let over_max = n_module_component_wat(PER_COMPONENT_MAX + 1);
    let msg = match Component::new(&engine, &over_max) {
        Ok(_) => panic!("a component exceeding the declared per-component max must not fit"),
        Err(err) => err.to_string(),
    };
    assert!(
        msg.contains("exceeds the configured maximum"),
        "expected a clear per-component-limit error, got: {msg}"
    );
}

fn test_app_engine(storage_provider: Arc<dyn StorageProvider>) -> AppSandboxEngine {
    // The trailing 0s are unused: `None, None` disables pooling entirely.
    let engine = AppSandboxEngine::build_wasm_engine(None, None, 0, 0, 0).unwrap();
    let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();
    AppSandboxEngine {
        blobs_dir: env::temp_dir(),
        engine,
        linker,
        components: DashMap::new(),
        fdae_policies: DashMap::new(),
        fdae_policy_generation: DashMap::new(),
        default_max_instructions: Some(10_000),
        default_max_memory_bytes: Some(1024 * 1024),
        guest_websocket_permits: Arc::new(DashMap::new()),
        max_concurrent_websockets_per_service: 10,
        max_sse_subscribers_per_service: 100,
        websocket_senders: {
            let ws = OnceLock::new();
            let _ = ws.set(WebSocketSenders::new());
            ws
        },
        _shutdown_tx: None,
        key_store: Arc::new(KeyStore::new()),
        storage_provider,
        blob_provider: test_blob_provider(),
        messaging_broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        self_weak: OnceLock::new(),
        service_proxy: OnceLock::new(),
        record_signer: OnceLock::new(),
        conversation: OnceLock::new(),
        subscriptions: DashMap::new(),
        endpoint_registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        logical_resolver: syneroym_app_orchestration::empty_resolver(),
        stream_registry: StreamRegistry::new(),
        max_concurrent_streams_per_service: 8,
        stream_instance_permits: Arc::new(Semaphore::new(8)),
        abac_instance_permits: Arc::new(Semaphore::new(1)),
        probe_instance_permits: Arc::new(Semaphore::new(2)),
        dispatch_epoch_ticks: ticks_for_secs(5),
        lifecycle_hook_epoch_ticks: ticks_for_secs(30),
        abac_epoch_ticks: ticks_for_secs(2),
        abac_max_instructions: 50_000_000,
        instantiations: AtomicU64::new(0),
        guest_http_permits: Arc::new(DashMap::new()),
        max_concurrent_guest_http_per_service: 4,
    }
}

/// A policy-absent service resolves `None` and caches it -- the common
/// case -- without re-querying `substrate.db` on a subsequent call.
#[tokio::test]
async fn fdae_policy_absent_resolves_none_and_caches() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let app_engine = test_app_engine(storage_provider);

    assert!(app_engine.fdae_policies.get("svc-none").is_none(), "nothing resolved yet");
    assert!(app_engine.resolve_fdae_policy("svc-none").await.is_none());
    assert!(
        app_engine.fdae_policies.get("svc-none").is_some(),
        "the absence itself must be cached, not just a miss"
    );
    assert!(app_engine.fdae_policies.get("svc-none").unwrap().is_none());
}

/// A persisted policy resolves to `Some`, is cached, and a cache hit does
/// not re-query storage (proven by mutating storage to an unparseable
/// document after the first resolution and confirming the second call
/// still returns the original, cached policy).
#[tokio::test]
async fn fdae_policy_present_resolves_some_and_cache_hit_skips_storage() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    storage_provider
        .save_fdae_policy("svc-some", r#"{"version": "fdae/v1", "definitions": {}}"#)
        .await
        .unwrap();
    let app_engine = test_app_engine(storage_provider.clone());

    let policy = app_engine.resolve_fdae_policy("svc-some").await;
    assert!(policy.is_some(), "a valid persisted policy must resolve to Some");
    assert!(app_engine.fdae_policies.get("svc-some").is_some());

    // Corrupt storage after the first resolution; a cache hit must not
    // observe this -- if it did, the second call would return None.
    storage_provider.save_fdae_policy("svc-some", "not valid json").await.unwrap();
    let cached = app_engine.resolve_fdae_policy("svc-some").await;
    assert!(cached.is_some(), "a cache hit must not re-query storage");
}

/// `stop_wasm` and `compile_and_cache_wasm` (a re-deploy) both evict the
/// resolved-policy cache, so the next instantiation re-resolves from
/// storage rather than serving a stale value.
#[tokio::test]
async fn fdae_policy_cache_evicted_on_stop_wasm_and_recompile() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    storage_provider
        .save_fdae_policy("svc-evict", r#"{"version": "fdae/v1", "definitions": {}}"#)
        .await
        .unwrap();
    let app_engine = test_app_engine(storage_provider);

    assert!(app_engine.resolve_fdae_policy("svc-evict").await.is_some());
    assert!(app_engine.fdae_policies.get("svc-evict").is_some());

    app_engine.stop_wasm("svc-evict").await.unwrap();
    assert!(
        app_engine.fdae_policies.get("svc-evict").is_none(),
        "stop_wasm must evict the cached policy"
    );

    assert!(app_engine.resolve_fdae_policy("svc-evict").await.is_some());
    assert!(app_engine.fdae_policies.get("svc-evict").is_some());

    let minimal_component = b"(component)";
    app_engine.compile_and_cache_wasm("svc-evict", minimal_component.to_vec(), None).await.unwrap();
    assert!(
        app_engine.fdae_policies.get("svc-evict").is_none(),
        "a re-deploy's recompile must evict the cached policy"
    );
}

/// A malformed persisted policy is fail-closed-*absent*:
/// `resolve_fdae_policy` logs and caches `None` rather than propagating
/// an error that would deny every read for the service (the deploy path
/// is what rejects a bad policy before it's ever persisted).
#[tokio::test]
async fn fdae_policy_unparseable_in_storage_resolves_none_not_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    storage_provider.save_fdae_policy("svc-bad", "not valid json").await.unwrap();
    let app_engine = test_app_engine(storage_provider);

    assert!(app_engine.resolve_fdae_policy("svc-bad").await.is_none());
    assert!(app_engine.fdae_policies.get("svc-bad").unwrap().is_none());
}

/// Pins `classify_call_failure`'s taxonomy: the downcast
/// `Trap::OutOfFuel` and the two fuel substrings both
/// classify as `OutOfFuel`, memory/epoch substrings classify as
/// expected, and an unrelated error falls through to `Other`. Without
/// this test the fuel-detail regression that prompted it went
/// unnoticed.
#[test]
fn classify_call_failure_matches_taxonomy() {
    let fuel_trap = wasmtime::Error::from(Trap::OutOfFuel);
    assert!(matches!(classify_call_failure(&fuel_trap), CallFailure::OutOfFuel));

    let fuel_string = wasmtime::Error::msg("all fuel consumed by WebAssembly code");
    assert!(matches!(classify_call_failure(&fuel_string), CallFailure::OutOfFuel));

    let fuel_string_alt = wasmtime::Error::msg("trap: out of fuel");
    assert!(matches!(classify_call_failure(&fuel_string_alt), CallFailure::OutOfFuel));

    let memory_string = wasmtime::Error::msg("instance exceeded its memory limits");
    assert!(matches!(classify_call_failure(&memory_string), CallFailure::MemoryFault));

    let memory_string_alt = wasmtime::Error::msg("host trap: MemoryFault");
    assert!(matches!(classify_call_failure(&memory_string_alt), CallFailure::MemoryFault));

    let epoch_string = wasmtime::Error::msg("epoch deadline reached while executing");
    assert!(matches!(classify_call_failure(&epoch_string), CallFailure::Deadline));

    let other = wasmtime::Error::msg("some unrelated wasm trap");
    assert!(matches!(classify_call_failure(&other), CallFailure::Other));
}
