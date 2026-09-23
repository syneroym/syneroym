use std::sync::{Arc, Mutex, Weak};

use dashmap::DashMap;
use syneroym_core::{
    config::SubstrateConfig,
    local_registry::EndpointRegistry,
    storage::{EndpointStorage, MockStorage},
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{NativeDispatchRegistry, ProxyError, ServiceProxy};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    NetworkEndpoint, ServiceConfig,
};

use super::super::*;
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

/// A fake `ServiceProxy` that records the last request it
/// received and answers with whatever `response` was primed with, so
/// `run_scheduled`'s dispatch is assertable without a real deployed
/// target on the other end.
#[derive(Debug, Default)]
pub(super) struct RecordingProxy {
    pub(super) last_request: Mutex<Option<ProxyRequest>>,
    pub(super) response: Mutex<Option<Result<Value, ProxyError>>>,
}

#[async_trait::async_trait]
impl ServiceProxy for RecordingProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        *self.last_request.lock().unwrap() = Some(request);
        self.response.lock().unwrap().take().expect("response not set for test")
    }
}

/// Wires `proxy` into `service.service_proxy` the way `RouteHandler::init`
/// does post-construction -- `service_proxy` is a `Weak`,
/// so the caller must keep `proxy` alive for as long as the service is
/// used.
pub(super) fn wire_service_proxy(service: &ControlPlaneService, proxy: &Arc<RecordingProxy>) {
    let dynamic: Arc<dyn ServiceProxy> = proxy.clone();
    let weak: Weak<dyn ServiceProxy> = Arc::downgrade(&dynamic);
    service.service_proxy.set(weak).expect("service_proxy already set");
}

/// A caller holding node-wide orchestrator authority on
/// `"did:key:zTestNode"` (every test in this module inits
/// `ControlPlaneService` with that node DID) -- the shape `build_caller`
/// issues for a verified `ControllerAgreement` controller. Deploy/undeploy
/// gate on an explicit `orchestrator/{deploy,undeploy}` capability,
/// so every test below that exercises `deploy`/`deploy_plan`/
/// `undeploy` and expects to get *past* that gate (to reach a
/// path-traversal/schema/rollback/ownership assertion further in) needs
/// a caller that holds it -- `CallerContext::service_system` (zero
/// capabilities) no longer suffices on its own.
pub(super) fn node_wide_caller(caller_did: &str) -> CallerContext {
    use syneroym_rpc::{AuthLevel, Capability, SessionContext};

    let resource = ResourceUri::substrate("did:key:zTestNode");
    CallerContext {
        caller_did: caller_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: caller_did.to_string(),
            capabilities: vec![
                Capability {
                    with: resource.clone(),
                    can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                    caveats: None,
                },
                Capability {
                    with: resource,
                    can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                    caveats: None,
                },
            ],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A caller holding an app-scoped `orchestrator/deploy`
/// grant for exactly `service_id` (`substrate:<node>/app/<service_id>`
/// selector) rather than `node_wide_caller`'s bare, node-wide form.
/// `has_node_wide_ability` returns `false` for this caller -- needed for
/// tests that must reach *past* the admission gate to exercise a
/// takeover/ownership rejection, which a node-wide caller always
/// bypasses.
pub(super) fn scoped_deploy_caller(caller_did: &str, service_id: &str) -> CallerContext {
    use syneroym_rpc::{AuthLevel, Capability, SessionContext};

    let resource = ResourceUri(format!("substrate:did:key:zTestNode/app/{service_id}"));
    CallerContext {
        caller_did: caller_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: caller_did.to_string(),
            capabilities: vec![Capability {
                with: resource,
                can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

pub(super) fn app_context(
    app_instance_id: &str,
    service_name: &str,
    bindings: Vec<DependencyBinding>,
) -> AppContext {
    AppContext {
        app_instance_id: app_instance_id.to_string(),
        service_name: service_name.to_string(),
        bindings,
        // Unmanaged: every existing test here is an
        // ordinary operator-style deploy, unaffected by the
        // generation gate. Tests that need a specific generation
        // override it with `AppContext { generation: N, ..app_context(...) }`.
        generation: 0,
    }
}

pub(super) fn dependency_binding(name: &str, members: Vec<&str>) -> DependencyBinding {
    DependencyBinding {
        dependency_name: name.to_string(),
        app_instance_id: "app-1".to_string(),
        mode: WitTopologyMode::Singleton,
        members: members.into_iter().map(str::to_string).collect(),
        epoch: 0,
        cache_ttl_ms: 60_000,
    }
}

/// Registers a local endpoint for `service_id`/`interface` -- the fact
/// `run_scheduled` requires before it dispatches, since a target the
/// endpoint registry does not know would be resolved through the
/// community registry and called on another node.
pub(super) async fn register_local_endpoint(
    service: &ControlPlaneService,
    service_id: &str,
    interface: &str,
) {
    service
        .registry
        .register(
            service_id.to_string(),
            interface.to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();
}

/// A minimal, real component that compiles successfully but exports
/// nothing under `syneroym:data-layer/authorizer` -- the same
/// `wat` shape
/// `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
/// uses to get a real, cheap-to-build component without a
/// `cargo-component`-built fixture.
pub(super) const WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT: &str = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "greet" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;

/// A policy opting a single permission into the stage-4 after-step
/// (`authorize_rows: true`, ADR-0017 §7).
pub(super) const STAGE4_POLICY: &str = r#"{
        "version": "fdae/v1",
        "definitions": {
            "items": {
                "table": "items",
                "principal_column": "creator_id",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [["caller"]],
                        "authorize_rows": true
                    }
                }
            }
        }
    }"#;

/// Shared harness for the saga compensation deploy-gate tests below --
/// the same construction every other test in this module repeats
/// inline, factored here only because this group needs it six times in
/// a row.
pub(super) async fn saga_gate_test_service(
    temp_dir: &std::path::Path,
) -> (ControlPlaneService, Arc<SqliteStorageProvider>) {
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine = Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let native_dispatch = NativeDispatchRegistry::default();
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.to_path_buf(),
        key_store,
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();
    (service, storage_provider)
}

pub(super) fn saga_gate_manifest(wat: &str, interfaces: Vec<String>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wat.as_bytes().to_vec()),
            hash: None,
            interfaces,
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

/// Exports `saga-undo-reserve` with no `reserve` beside it -- the
/// defect the deploy gate exists to catch.
pub(super) const WASM_UNDO_WITH_NO_FORWARD: &str = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "saga-undo-reserve" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;

/// Exports both `reserve` and its compensation `saga-undo-reserve`.
pub(super) const WASM_UNDO_WITH_FORWARD: &str = r#"
(component
  (core module $m
    (func (export "noop_a"))
    (func (export "noop_b")))
  (core instance $i (instantiate $m))
  (func $a (canon lift (core func $i "noop_a")))
  (func $b (canon lift (core func $i "noop_b")))
  (instance $interface
    (export "reserve" (func $a))
    (export "saga-undo-reserve" (func $b)))
  (export "test-interface" (instance $interface))
)
"#;

/// Exports a plain `undo-last-update` -- an ordinary business verb, not
/// a saga compensation (`undo-` is not the reserved prefix).
pub(super) const WASM_PLAIN_UNDO_PREFIX: &str = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "undo-last-update" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;

/// Wraps `MockStorage`, failing `save` for one specific interface name --
/// lets a test deterministically fail `EndpointRegistry::register`
/// (used by `register_wasm_endpoints`/`deploy_container_service`'s
/// registration loop) without needing a real network/podman failure.
pub(super) struct FailingEndpointStorage {
    pub(super) inner: MockStorage,
    pub(super) fail_interface: String,
}

#[async_trait::async_trait]
impl EndpointStorage for FailingEndpointStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        self.inner.load_all().await
    }
    async fn save(
        &self,
        service_id: &str,
        interface_name: &str,
        endpoint: &SubstrateEndpoint,
    ) -> Result<()> {
        if interface_name == self.fail_interface {
            anyhow::bail!("simulated registry storage failure for {interface_name}");
        }
        self.inner.save(service_id, interface_name, endpoint).await
    }
    async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
        self.inner.remove(service_id, interface_name).await
    }
    async fn load_all_owners(&self) -> Result<Vec<(String, String)>> {
        self.inner.load_all_owners().await
    }
    async fn save_owner(&self, service_id: &str, owner_did: &str) -> Result<()> {
        self.inner.save_owner(service_id, owner_did).await
    }
    async fn remove_owner(&self, service_id: &str) -> Result<()> {
        self.inner.remove_owner(service_id).await
    }
    async fn load_all_certs(&self) -> Result<Vec<(String, String)>> {
        self.inner.load_all_certs().await
    }
    async fn save_cert(&self, service_id: &str, certificate_json: &str) -> Result<()> {
        self.inner.save_cert(service_id, certificate_json).await
    }
    async fn remove_cert(&self, service_id: &str) -> Result<()> {
        self.inner.remove_cert(service_id).await
    }
    async fn load_all_deploy_facts(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        self.inner.load_all_deploy_facts().await
    }
    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
        manifest_hash: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<()> {
        self.inner
            .save_deploy_facts(
                service_id,
                service_type,
                health_check_json,
                manifest_hash,
                visibility,
            )
            .await
    }
    async fn remove_deploy_facts(&self, service_id: &str) -> Result<()> {
        self.inner.remove_deploy_facts(service_id).await
    }
    async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>> {
        self.inner.load_all_app_contexts().await
    }
    async fn save_app_context(
        &self,
        service_id: &str,
        app_instance_id: &str,
        service_name: &str,
    ) -> Result<()> {
        self.inner.save_app_context(service_id, app_instance_id, service_name).await
    }
    async fn remove_app_context(&self, service_id: &str) -> Result<()> {
        self.inner.remove_app_context(service_id).await
    }
    async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>> {
        self.inner.load_all_bindings().await
    }
    async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        topology_entry_json: &str,
    ) -> Result<()> {
        self.inner
            .save_binding(service_id, app_instance_id, dependency_name, topology_entry_json)
            .await
    }
    async fn load_binding(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> Result<Option<String>> {
        self.inner.load_binding(service_id, dependency_name).await
    }
    async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>> {
        self.inner.load_bindings_for(service_id).await
    }
    async fn load_all_app_instance_management(
        &self,
    ) -> Result<Vec<(String, AppInstanceManagement)>> {
        self.inner.load_all_app_instance_management().await
    }
    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> Result<()> {
        self.inner.save_app_instance_management(app_instance_id, management).await
    }
    async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()> {
        self.inner.remove_app_instance_management(app_instance_id).await
    }
}

/// A real, trivial WASM component's WAT text, encoded as bytes -- the
/// same one `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
/// uses, so `deploy_wasm_service` itself succeeds without needing the
/// `greeter`/`proxy-test` test-fixture artifacts (which may not be
/// built) for tests that only need *a* valid component, not any
/// particular one -- e.g. an asset-bundle test, where the component
/// itself is incidental to what's under test.
pub(super) fn minimal_wasm_component() -> Vec<u8> {
    r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "greet" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#
    .as_bytes()
    .to_vec()
}

/// A minimal gzip-compressed tar archive, one entry per `(path, bytes)`
/// pair -- the same shape `syneroym_control_plane::assets`' own unit
/// tests build, duplicated here rather than exposed as a `pub`
/// test-only helper across the module boundary.
pub(super) fn make_asset_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *data).unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap()
}

pub(super) fn guest_route_custom_config() -> String {
    serde_json::json!({
        "http_routes": [
            {"method": "GET", "path": "/echo", "target": "guest", "operation": "handle-request"}
        ]
    })
    .to_string()
}

pub(super) fn owner_test_manifest() -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 9100,
            }],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

/// Like `node_wide_caller`, plus `orchestrator/status` -- needed by the
/// `instance_identity` tests below, which `deploy`/`undeploy` never gate
/// on.
pub(super) fn status_capable_caller(caller_did: &str) -> CallerContext {
    use syneroym_rpc::{AuthLevel, Capability, SessionContext};

    let resource = ResourceUri::substrate("did:key:zTestNode");
    CallerContext {
        caller_did: caller_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: caller_did.to_string(),
            capabilities: vec![
                Capability {
                    with: resource.clone(),
                    can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                    caveats: None,
                },
                Capability {
                    with: resource.clone(),
                    can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                    caveats: None,
                },
                Capability {
                    with: resource,
                    can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                    caveats: None,
                },
            ],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// Builds a `ControlPlaneService` rooted at `temp_dir` with a caller-
/// supplied node identity (so tests can compute the exact instance key
/// the substrate will derive), returning the registry alongside it so
/// tests can inspect what got stored.
pub(super) async fn service_with_node_identity(
    temp_dir: &std::path::Path,
    node_identity: Arc<syneroym_identity::Identity>,
) -> (ControlPlaneService, EndpointRegistry) {
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let app_sandbox = Arc::new(
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
    let container_engine = Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None));

    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry.clone(),
        temp_dir.to_path_buf(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        NativeDispatchRegistry::default(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        node_identity,
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    (service, registry)
}

/// A minimal, stage-4-free policy for the renewal tests: enough for
/// `resolve-relation` on `members` to reach a real query rather than
/// short-circuiting, which is what makes "did the policy survive the
/// rebuild" observable from outside. Deliberately not `STAGE4_POLICY`
/// -- `authorize_rows` needs a guest component to export the after-step
/// and is refused on the `tcp` services these tests deploy.
pub(super) const RENEWAL_POLICY: &str = r#"{
        "version": "fdae/v1",
        "definitions": {
            "members": {
                "table": "members",
                "principal_column": "owner_id",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [["caller"]]
                    }
                }
            }
        }
    }"#;

/// Like `service_with_node_identity`, but hands the caller the
/// `NativeDispatchRegistry` back as well. `ControlPlaneService` holds it
/// as a `Weak` (see the cycle its own field doc explains), so a test
/// that lets the `Arc` drop at the end of construction gets a registry
/// that never upgrades -- and `renew_cert` fails closed on exactly that,
/// since the whole point of the verb is the rebuild it does through it.
pub(super) async fn service_with_dispatch(
    temp_dir: &std::path::Path,
    node_identity: Arc<syneroym_identity::Identity>,
) -> (ControlPlaneService, EndpointRegistry, NativeDispatchRegistry) {
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let app_sandbox = Arc::new(
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
    let native_dispatch = NativeDispatchRegistry::default();

    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None)),
        registry.clone(),
        temp_dir.to_path_buf(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        node_identity,
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    (service, registry, native_dispatch)
}

/// A `service-instance`-scoped certificate over exactly the key this
/// substrate derives for `(caller, service_id)` -- what every real mint
/// produces and what every install-time check below expects.
pub(super) fn instance_cert_for(
    node_identity: &syneroym_identity::Identity,
    master: &syneroym_identity::Identity,
    caller_did: &str,
    service_id: &str,
    expires_in_secs: u64,
) -> DelegationCertificate {
    let derived = node_identity.derive_service_identity(caller_did, service_id);
    DelegationCertificate::issue(
        master,
        derived.public_key(),
        expires_in_secs,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap()
}

/// Asks the service's own native-dispatch entry to sign a relationship
/// proof, which carries whichever certificate that entry currently
/// holds -- the only way to observe the by-value copy a renewal has to
/// refresh, from outside.
pub(super) async fn resolve_relation_through_dispatch(
    native_dispatch: &NativeDispatchRegistry,
    service_id: &str,
    relation: &str,
    caller: &CallerContext,
) -> Result<syneroym_rpc::RelationshipProof, syneroym_rpc::RpcError> {
    let entry = native_dispatch
        .get(service_id)
        .unwrap_or_else(|| panic!("no native dispatch entry for '{service_id}'"))
        .clone();
    let response = entry
        .dispatch(syneroym_rpc::NativeInvocation {
            interface: "data-layer".to_string(),
            method: "resolve-relation".to_string(),
            params: serde_json::json!({
                "relation": relation,
                "principal": caller.session.subject_did,
            }),
            caller: caller.clone(),
        })
        .await?;
    Ok(serde_json::from_value(response.payload).expect("payload must be a RelationshipProof"))
}

/// The proof a service with no matching relation definition still
/// signs -- empty `ids`, but carrying whichever certificate the
/// dispatch entry currently holds, which is the only way to observe
/// the by-value copy a renewal has to refresh.
pub(super) async fn relationship_proof_from_dispatch(
    native_dispatch: &NativeDispatchRegistry,
    service_id: &str,
    caller: &CallerContext,
) -> syneroym_rpc::RelationshipProof {
    resolve_relation_through_dispatch(native_dispatch, service_id, "unmatched", caller)
        .await
        .expect("resolve-relation must succeed")
}

/// Builds a service rooted at `temp_dir`, for the inline-document tests
/// below. They care about nothing in the wiring except that the working
/// directory holds no schema or policy file.
pub(super) async fn service_for_inline_tests(temp_dir: &std::path::Path) -> ControlPlaneService {
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );

    ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None)),
        EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        temp_dir.to_path_buf(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        NativeDispatchRegistry::default(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap()
}

pub(super) fn inline_manifest(
    custom_config: Option<&str>,
    schema: Option<DocumentSource>,
    fdae_policy: Option<DocumentSource>,
) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: custom_config.map(str::to_string),
            quota: None,
            schema,
            rotation_policy: None,
            fdae_policy,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

pub(super) fn tcp_manifest_with(port: u16, health_check: Option<WitHealthCheck>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "main".to_string(),
                host: "127.0.0.1".to_string(),
                port,
            }],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

pub(super) fn container_manifest_with(health_check: Option<WitHealthCheck>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Container(ContainerManifest {
            source: ArtifactSource::Url("docker.io/library/nginx:1.27".to_string()),
            hash: None,
            image: "docker.io/library/nginx:1.27".to_string(),
            ports: vec![],
            volumes: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

pub(super) fn wasm_manifest_with(health_check: Option<WitHealthCheck>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(vec![]),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

/// Stands in for the router's outbox, which this crate cannot depend
/// on. Records what was asked of it so `replay` can be shown not to
/// execute anything inline.
#[derive(Debug, Default)]
pub(super) struct FakeProxyQueues {
    pub(super) queued: std::sync::Mutex<Vec<QueuedCallInfo>>,
    pub(super) dead: std::sync::Mutex<Vec<DeadLetterInfo>>,
    pub(super) replayed: std::sync::Mutex<Vec<(String, u64)>>,
    pub(super) delivered: std::sync::atomic::AtomicUsize,
    pub(super) sagas: std::sync::Mutex<Vec<SagaInfo>>,
    pub(super) rearmed: std::sync::Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl ProxyQueueInspector for FakeProxyQueues {
    async fn queued_calls(&self, _service_id: &str) -> Result<Vec<QueuedCallInfo>, String> {
        Ok(self.queued.lock().unwrap().clone())
    }

    async fn dead_letters(&self, _service_id: &str) -> Result<Vec<DeadLetterInfo>, String> {
        Ok(self.dead.lock().unwrap().clone())
    }

    async fn replay_dead_letter(&self, service_id: &str, id: u64) -> Result<(), String> {
        // Re-enqueue only: a replay that executed here would be doing
        // the delivery itself, which is exactly what must not happen.
        self.replayed.lock().unwrap().push((service_id.to_string(), id));
        let mut dead = self.dead.lock().unwrap();
        let Some(pos) = dead.iter().position(|d| d.id == id) else {
            return Err(format!("no dead letter with id {id}"));
        };
        let letter = dead.remove(pos);
        self.queued.lock().unwrap().push(QueuedCallInfo {
            id: letter.id,
            idempotency_key: letter.idempotency_key,
            attempts: letter.attempts,
        });
        Ok(())
    }

    async fn sagas(&self, _service_id: &str) -> Result<Vec<SagaInfo>, String> {
        Ok(self.sagas.lock().unwrap().clone())
    }

    async fn rearm_saga(&self, service_id: &str, saga_id: &str) -> Result<(), String> {
        // Re-arm only: walking the saga here would be doing the
        // delivery itself, which is exactly what must not happen.
        self.rearmed.lock().unwrap().push((service_id.to_string(), saga_id.to_string()));
        Ok(())
    }
}

pub(super) async fn service_with_proxy_queues(
    dir: &std::path::Path,
    queues: &Arc<FakeProxyQueues>,
) -> ControlPlaneService {
    let service = service_for_inline_tests(dir).await;
    service
        .proxy_queues
        .set(Arc::downgrade(queues) as std::sync::Weak<dyn ProxyQueueInspector>)
        .expect("proxy queues set once");
    service
}

pub(super) fn a_dead_letter(id: u64, key: &str) -> DeadLetterInfo {
    DeadLetterInfo {
        id,
        idempotency_key: key.to_string(),
        attempts: 54,
        last_error: "target unreachable".to_string(),
        created_at: 1_700_000_000_000,
    }
}

pub(super) fn a_saga(saga_id: &str, state: &str) -> SagaInfo {
    SagaInfo {
        saga_id: saga_id.to_string(),
        name: "checkout".to_string(),
        state: syneroym_rpc::SagaState::Compensating,
        steps: 2,
        compensated_steps: 1,
        created_at: 1_700_000_000_000,
        deadline_at: 1_700_003_600_000,
        last_error: Some(state.to_string()),
    }
}

/// A bare TCP listener that answers every connection with a fixed,
/// hand-written HTTP response -- avoids pulling in a full HTTP server
/// framework as a test-only dependency just to drive an `http-get`
/// probe (A4-13).
pub(super) async fn serve_http_responses(response: &'static str) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    port
}

pub(super) fn test_signed_record(service_id: &str, is_private: bool) -> String {
    serde_json::to_string(&SignedEndpointInfo {
        info: syneroym_core::dht_registry::EndpointInfo {
            service_id: service_id.to_string(),
            substrate_id: "test-node".to_string(),
            endpoint_type: syneroym_core::dht_registry::EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private,
            ttl: None,
            not_after: 0,
            generation: 0,
        },
        pkarr_packet_hex: "abcd".to_string(),
    })
    .unwrap()
}
