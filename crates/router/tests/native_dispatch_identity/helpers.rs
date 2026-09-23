pub(crate) use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub(crate) use dashmap::DashMap;
pub(crate) use serde_json::{Value, json};
pub(crate) use syneroym_control_plane::SynSvcNativeService;
pub(crate) use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    dht_registry::RegistryClient,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
};
pub(crate) use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
pub(crate) use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider, host_store, sqlite::MAX_BATCH_SIZE,
};
pub(crate) use syneroym_data_keystore::KeyStore;
pub(crate) use syneroym_fdae::{MAX_FETCH_IDS, Mode, Policy, parse_and_validate};
pub(crate) use syneroym_identity::substrate;
pub(crate) use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
pub(crate) use syneroym_router::{
    AdaptationStage, EncryptionStage, IrohHop, ProxyRouter, RouteHandler, RouteHandlerDeps,
    RoutePipeline, RoutePreamble, RouteProtocol, RouteTransport, ServiceStage, TransportStage,
};
pub(crate) use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, FetchError, NativeDispatchRegistry,
    NativeInvocation, NativeResponse, NativeService, ProxyError, ProxyRequest, ResourceUri,
    RpcResult, ServiceProxy, SessionContext,
};
pub(crate) use syneroym_sandbox_wasm::AppSandboxEngine;

#[derive(Debug, Default)]
pub(crate) struct RecordingNativeService {
    pub(crate) invoked: AtomicBool,
}

impl RecordingNativeService {
    pub(crate) fn was_invoked(&self) -> bool {
        self.invoked.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl NativeService for RecordingNativeService {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        self.invoked.store(true, Ordering::SeqCst);
        Ok(NativeResponse { payload: Value::Null })
    }
}

pub(crate) fn test_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        // `subject_did` mirrors `caller_did`, exactly as production's
        // `build_caller` (`route_handler/io.rs:169`) always sets it --
        // `write_attribution` reads the verified session identity,
        // not the raw `caller_did` field, so a fixture leaving this at
        // `SessionContext::default()`'s empty string would attribute writes
        // to nobody instead of this caller.
        session: SessionContext { subject_did: did.to_string(), ..Default::default() },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// Seeds one fixture row directly against a service's store, `auth: None`
/// -- bypasses the write-side FDAE gate entirely, which these
/// read-side tests are not about: their fixture policies declare no
/// `data-layer/write` permission at all, so seeding through the gated
/// `put`/`create-collection` JSON-RPC path would deny closed regardless of
/// which caller presents it.
pub(crate) async fn seed_via_store(
    storage_provider: &Arc<dyn StorageProvider>,
    key_store: &Arc<KeyStore>,
    service_id: &str,
    collection: &str,
    id: &str,
    payload: &Value,
) {
    let store = storage_provider.open_service_db(service_id, key_store).await.unwrap();
    store
        .create_collection(&host_store::CollectionSchema {
            name: collection.to_string(),
            indexes: vec![],
        })
        .await
        .unwrap();
    store
        .put(
            collection,
            &host_store::RecordWriteValue {
                id: id.to_string(),
                payload: payload.to_string().into_bytes(),
            },
            service_id,
            None,
        )
        .await
        .unwrap();
}

/// The same `[iam].admin_ucan_root` grant `build_caller`
/// (`crates/router/src/route_handler/io.rs`) constructs for a caller whose
/// master DID matches the configured admin root: `substrate/admin` on
/// `substrate:<did>`.
pub(crate) fn admin_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: did.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate(did),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// Builds a minimal `RouteHandler` with empty `native_dispatch`/
/// `http_routes` tables the test populates itself.
pub(crate) async fn test_route_handler() -> (RouteHandler, HttpRouteRegistry) {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            registry.clone(),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());

    let deps = RouteHandlerDeps {
        logical_resolver: syneroym_app_orchestration::empty_resolver(),
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch: NativeDispatchRegistry::default(),
        native_http: Arc::new(DashMap::new()),
        websocket_senders: syneroym_rpc::WebSocketSenders::new(),
        http_routes: http_routes.clone(),
        assets: Arc::new(DashMap::new()),
        sse_permits: Arc::new(DashMap::new()),
        control_plane_service: Arc::new(RecordingNativeService::default()),
        control_plane: None,
        session_revocation: None,
    };

    let route_handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [7u8; 32],
        None,
        deps,
    )
    .await
    .unwrap();

    (route_handler, http_routes)
}

pub(crate) fn raw_pipeline(service_id: &str) -> RoutePipeline {
    RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::None,
        service: ServiceStage::NativeService { service_id: service_id.to_string() },
    }
}

pub(crate) fn preamble_for(service_id: &str, interface: &str) -> RoutePreamble {
    RoutePreamble {
        transport: RouteTransport::Binary,
        protocol: RouteProtocol::JsonRpc,
        interface: interface.to_string(),
        service_id: service_id.to_string(),
        enc: None,
        pubkey: None,
        delegation: None,
        ucan: None,
        dir: None,
    }
}

pub(crate) fn json_rpc_body(method: &str, params: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}))
        .unwrap()
}

/// `document` --creator--> `user`, `view` permission reachable only via the
/// creator relation, plus `fields.deny: ["ssn"]` -- mirrors
/// `sandbox_wasm::host_capabilities`'s `fdae_cls_policy`, the WASM-host-path
/// analog of this same policy shape.
pub(crate) fn native_fdae_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

pub(crate) fn native_fdae_resource(service_id: &str, collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(service_id, service_id).0
    ))
}

/// A verified caller entitled to `data-layer/read` on `documents` -- the
/// router-threaded `CallerContext` shape a real external caller carries,
/// distinct from `test_caller` (no capabilities) and `admin_caller`
/// (node-wide authority).
pub(crate) fn fdae_reader_caller(subject_did: &str, service_id: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "documents"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A single `employee` definition supporting both authorization
/// models: `view_self`'s zero-hop `paths: [["caller"]]` is what A1 (the
/// existing capability-gated sieve) evaluates, and
/// `resolvable_without_capability: true` is the explicit per-definition
/// opt-in A2 (bare `principal_column` match) requires.
pub(crate) fn resolvable_employee_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "employee": {
                    "table": "employees",
                    "principal_column": "did",
                    "resolvable_without_capability": true,
                    "permissions": {
                        "view_self": {"allows": ["data-layer/read"], "paths": [["caller"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// A verified caller entitled to `data-layer/read` on the `employee`
/// resource -- the A1 path.
pub(crate) fn employee_reader_caller(subject_did: &str, service_id: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                // Scoped to the physical table ("employees"), not the
                // policy definition key ("employee") -- `compile_read`
                // builds its resource URI from whatever `collection` string
                // actually reaches it, which is the resolved table
                // (`definition_table`), matching every other capability
                // scoping convention in this file (e.g. `fdae_reader_caller`
                // above uses "documents", not "document").
                with: native_fdae_resource(service_id, "employees"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A verified caller holding a capability scoped to a **different**
/// resource entirely -- B3-07: the A1/A2 fork must key on capabilities
/// scoped to *this* resource, not "holds any capability at all", so this
/// caller correctly routes to A2 (as if capability-less for `employees`),
/// not to a real-but-unrelated A1 grant check.
pub(crate) fn unrelated_resource_capability_caller(
    subject_did: &str,
    service_id: &str,
) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "some_other_collection"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A verified caller holding a capability scoped to the **right**
/// resource (`employees`) but for an ability `view_self`'s `allows:
/// ["data-layer/read"]` doesn't cover -- routes to A1 (a capability *is*
/// scoped here), which then genuinely denies via the grant∩policy
/// intersection. Exercises the real A1 deny, not one A2 rescued.
///
/// Must be an ability data-layer/read doesn't entail: the `data-layer`
/// namespace is a *tiered* hierarchy (`admin ⊇ write ⊇ read`,
/// `Ability::entails`), so `data-layer/write` would actually cover
/// `data-layer/read` here -- a flat, unrelated ability (`blob/read`) is
/// what genuinely fails to entail it.
pub(crate) fn wrong_ability_on_the_right_resource_caller(
    subject_did: &str,
    service_id: &str,
) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "employees"),
                can: Ability(Ability::BLOB_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A verified caller holding **zero** capabilities -- a real
/// `session.subject_did` (unlike `test_caller`, whose `SessionContext`
/// default leaves it empty), the shape `build_caller`
/// (`crates/router/src/route_handler/io.rs`) always produces for any
/// verified connection, capability-laden or not.
pub(crate) fn zero_capability_caller(subject_did: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext { subject_did: subject_did.to_string(), ..Default::default() },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

pub(crate) type ResolveRelationHarness = (
    RouteHandler,
    RoutePipeline,
    RoutePreamble,
    tempfile::TempDir,
    Arc<dyn StorageProvider>,
    Arc<KeyStore>,
);

pub(crate) async fn resolve_relation_service_and_pipeline(
    service_id: &str,
    policy: Option<Policy>,
) -> ResolveRelationHarness {
    resolve_relation_service_and_pipeline_with(
        service_id,
        policy,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
    )
    .await
}

/// Same as [`resolve_relation_service_and_pipeline`], but lets the caller
/// pin `node_identity`/`owner_did` explicitly -- needed to construct two
/// services that share one or the other while varying just the dimension
/// under test (e.g. same node, different owner).
pub(crate) async fn resolve_relation_service_and_pipeline_with(
    service_id: &str,
    policy: Option<Policy>,
    node_identity: Arc<syneroym_identity::Identity>,
    owner_did: &str,
) -> ResolveRelationHarness {
    let (route_handler, _http_routes) = test_route_handler().await;
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.to_string(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        policy.map(Arc::new),
        node_identity,
        owner_did,
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.to_string(), data_service);

    let pipeline = raw_pipeline(service_id);
    let preamble = preamble_for(service_id, "data-layer");

    // Seeded directly against the store, `auth: None` -- `resolvable_
    // employee_policy` (and every other fixture policy used here) declares
    // no `data-layer/write` permission at all, so seeding through the
    // gated `put`/`batch-mutate` JSON-RPC path would deny closed regardless
    // of which caller presents it.
    let store = storage_provider.open_service_db(service_id, &key_store).await.unwrap();
    store
        .create_collection(&host_store::CollectionSchema {
            name: "employees".to_string(),
            indexes: vec![],
        })
        .await
        .unwrap();
    for (id, did) in [("emp-alice", "did:key:alice"), ("emp-bob", "did:key:bob")] {
        store
            .put(
                "employees",
                &host_store::RecordWriteValue {
                    id: id.to_string(),
                    payload: json!({"did": did}).to_string().into_bytes(),
                },
                service_id,
                None,
            )
            .await
            .unwrap();
    }
    drop(store);

    (route_handler, pipeline, preamble, temp_dir, storage_provider, key_store)
}

/// Seeds `count` employee rows, all sharing `did` (so a single principal's
/// `view_self`/structural lookup reaches all of them), directly against the
/// store (`resolvable_employee_policy` declares no `data-layer/write`
/// permission, so the gated `batch-mutate` JSON-RPC path would deny closed
/// regardless of caller, same reasoning as the fixed-row seeding above) --
/// needed to construct an id-set bigger than `MAX_FETCH_IDS` (1000) for the
/// overflow tests below.
pub(crate) async fn seed_many_employees(
    storage_provider: &Arc<dyn StorageProvider>,
    key_store: &Arc<KeyStore>,
    service_id: &str,
    did: &str,
    count: usize,
) {
    let store = storage_provider.open_service_db(service_id, key_store).await.unwrap();
    let mutations: Vec<host_store::Mutation> = (0..count)
        .map(|i| {
            host_store::Mutation::Put(host_store::RecordWriteValue {
                id: format!("emp-bulk-{i}"),
                payload: json!({"did": did}).to_string().into_bytes(),
            })
        })
        .collect();
    for chunk in mutations.chunks(MAX_BATCH_SIZE) {
        store.batch_mutate("employees", chunk, service_id, None).await.unwrap();
    }
}

pub(crate) fn resolve_relation_body(relation: &str, principal: &str) -> Vec<u8> {
    json_rpc_body("resolve-relation", json!({"relation": relation, "principal": principal}))
}

/// Same shape as `resolvable_employee_policy`, but `view_self` opts into
/// the stage-4 after-step. Neither A1 nor A2 has a compiled sieve in hand
/// (`synsvc_native.rs::resolve_relation`'s own doc comment), so
/// `definition_has_abac` denies both branches at the definition level
/// before either would otherwise run.
pub(crate) fn resolvable_employee_policy_with_stage4() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "employee": {
                    "table": "employees",
                    "principal_column": "did",
                    "resolvable_without_capability": true,
                    "permissions": {
                        "view_self": {
                            "allows": ["data-layer/read"],
                            "paths": [["caller"]],
                            "authorize_rows": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// Constructs hr-svc's `SynSvcNativeService` directly
/// (not via `resolve_relation_service_and_pipeline_with`/`RouteHandler`,
/// which hides its registry/native_dispatch -- this test needs a real
/// `ProxyRouter` it can hand to `syneroym_rpc::resolve_fetches`), registers
/// it as a `NativeHostChannel` in a fresh `EndpointRegistry`, and seeds two
/// employees. Returns the router (as `ServiceProxy`), the DID a caller's
/// policy must declare as `expected_asserter_did` to trust this instance
/// (computed the same way a real policy author would: independently, from
/// the `(owner_did, service_id)` pair they were told about -- never read
/// back off a proof), and the owned `NativeDispatchRegistry`/`TempDir` the
/// caller must keep alive for the test's duration.
///
/// **Must not leak these.** `SqliteStorageProvider` spawns a
/// `spawn_blocking` writer-loop task on the *ambient* runtime (here,
/// `#[tokio::test]`'s own), which only exits once its channel `Sender`
/// (owned transitively by `hr_service`, kept alive by `native_dispatch`) is
/// dropped. Leaking `native_dispatch`/`temp_dir` (an earlier version of this
/// helper did, mirroring a pattern used elsewhere in this file for values
/// that genuinely need `'static`) keeps that writer thread running forever,
/// which deadlocks the test runtime's shutdown (`BlockingPool::shutdown`
/// waits for every spawned blocking task to finish) -- confirmed by `sample`
/// on the hung process, which showed the main thread parked in
/// `Runtime::drop` -> `BlockingPool::shutdown` while a `tokio-rt-worker`
/// thread sat in `run_writer_loop`'s `blocking_recv()`. Returning owned
/// handles the test function holds until it returns (normal drop order)
/// fixes this.
pub(crate) async fn build_hr_svc_proxy_router(
    node_identity: Arc<syneroym_identity::Identity>,
    owner_did: &str,
) -> (Arc<ProxyRouter>, String, NativeDispatchRegistry, tempfile::TempDir) {
    let hr_service_id = "hr-svc";
    let expected_asserter_did = substrate::derive_did_key(
        &node_identity.derive_service_identity(owner_did, hr_service_id).public_key(),
    );

    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let hr_service = Arc::new(SynSvcNativeService::new(
        hr_service_id.to_string(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(Arc::new(resolvable_employee_policy())),
        node_identity.clone(),
        owner_did,
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    // Seeded directly against the store, `auth: None` -- `resolvable_
    // employee_policy` declares no `data-layer/write` permission at all, so
    // seeding through the gated native `"put"` dispatch would deny closed
    // regardless of caller.
    for (id, did) in [("emp-alice", "did:key:alice"), ("emp-bob", "did:key:bob")] {
        seed_via_store(
            &storage_provider,
            &key_store,
            hr_service_id,
            "employees",
            id,
            &json!({"did": did}),
        )
        .await;
    }

    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    registry
        .register(
            hr_service_id.to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: hr_service_id.to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    native_dispatch.insert(hr_service_id.to_string(), hr_service as Arc<dyn NativeService>);

    let router = Arc::new(ProxyRouter::new(
        registry,
        Arc::new(RegistryClient::new(false, None)),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(IrohHop::new(None, RetryPolicy::default())),
        node_identity,
        RetryPolicy::default(),
    ));

    (router, expected_asserter_did, native_dispatch, temp_dir)
}
