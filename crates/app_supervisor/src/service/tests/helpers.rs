use std::{
    collections::{BTreeMap, VecDeque},
    future,
    path::{Path, PathBuf},
    sync::Mutex,
};

use syneroym_app_orchestration::{
    DEFAULT_SCHEDULE_TIMEOUT_MS,
    models::{
        AppBlueprintId, InterfaceName, LogicalServiceRef, ServiceConfig, ServiceType, TopologyMode,
    },
};
use syneroym_core::dht_registry::SignedEndpointInfo;
use syneroym_identity::substrate;
use syneroym_rpc::AuthLevel;

use super::super::*;

pub(super) fn test_broker() -> Arc<MqttBroker> {
    Arc::new(MqttBroker::new(syneroym_mqtt_broker::MqttBrokerConfig::default()).unwrap())
}

/// What a fixture varies about the supervisor under test. Everything
/// else -- node DID, broker, alert topic, intervals -- is fixed, since
/// no test has a reason to change it.
#[derive(Default)]
pub(super) struct Fixture {
    /// Encryption on with no KEK injected, so the vault genuinely
    /// refuses reads. A disabled-encryption fixture proves nothing
    /// about the locked case.
    pub(super) locked_vault: bool,
    /// Injects a KEK even when `locked_vault` turned encryption on --
    /// an encrypted vault that is currently *open*. Only the vault-race
    /// test needs this: it then clears the KEK to reach the state
    /// `kek_is_loaded()` cannot describe, where the check has already
    /// passed and the read that follows fails locked.
    pub(super) inject_kek_anyway: bool,
    /// Skips the KEK injection this builder otherwise gives an
    /// unencrypted (`locked_vault: false`) fixture by default -- the
    /// one way to reach `storage.encryption = false` with no KEK
    /// ever injected, which `kek_is_loaded()` (a `KeyStore`-only
    /// check) cannot distinguish from a genuinely locked, encrypted
    /// vault. Only a test proving a caller reads the vault by
    /// attempting it rather than pre-checking `kek_is_loaded()` needs
    /// this -- on this fixture, every vault read still succeeds.
    pub(super) skip_kek_injection: bool,
    /// `None` leaves the default (5).
    pub(super) max_renewals_per_pass: Option<u32>,
    pub(super) anchor_writer: Option<Arc<dyn AnchorWriter>>,
    pub(super) tier1_writer: Option<Arc<dyn Tier1Writer>>,
    pub(super) master_anchor_refresh_interval_secs: Option<u64>,
    /// `None` leaves the default -- a private `dir.path().join(
    /// "backups")` on the `TempDir` the built service now keeps alive
    /// on its own `_fixture_tempdir` field, for its own lifetime. The
    /// handover test needs two fixture-built services to share one
    /// backup directory (a stand-in for two supervisors handed the
    /// same operator-carried file), which the default cannot do -- so
    /// the test owns and passes one in, held for the whole test.
    pub(super) backup_dir: Option<PathBuf>,
    /// `None` leaves the default (5s). Tests driving the queue worker
    /// against a fake clock set this explicitly.
    pub(super) queue_tick_secs: Option<u64>,
    /// `None` leaves the default (30s). Tests proving recovery happens
    /// within one worker tick rather than one poll interval set this
    /// explicitly, far above the tick.
    pub(super) poll_interval_secs: Option<u64>,
    /// `None` leaves the default (3600s). Tests proving a document
    /// re-signs once less than half its validity remains set this low.
    pub(super) topology_document_not_after_secs: Option<u64>,
    /// `None` leaves the default (300s).
    pub(super) topology_document_cache_ttl_secs: Option<u64>,
}

impl Fixture {
    pub(super) fn build(self) -> SupervisorService {
        self.build_with_key_store().0
    }

    /// Hands back the `KeyStore` alongside the service, so a test can
    /// change the vault's locked state *after* construction -- the only
    /// way to reach the race where `kek_is_loaded()` answers
    /// "unlocked" and the vault read that follows still fails.
    pub(super) fn build_with_key_store(
        self,
    ) -> (SupervisorService, Arc<syneroym_data_keystore::KeyStore>) {
        // Kept alive on the returned `SupervisorService` itself
        // (`_fixture_tempdir`), not left to drop here: a dropped
        // `TempDir` deletes the directory tree from disk the instant
        // this function returns, while `SqliteStorageProvider` already
        // holds an open connection into it (found while
        // adding the first fixture-built test that performs a real
        // *encrypted* write -- every earlier fixture's writes went
        // through `open_service_db`'s own on-demand directory
        // recreation and never touched the provider's `substrate_conn`,
        // so this never surfaced before). An *unencrypted* mint
        // recreates its directory on demand and keeps working
        // regardless -- the DEK path does not, since it writes through
        // the provider's own top-level connection, opened once at
        // construction against a file an early drop would have already
        // unlinked, and a `-journal` file cannot be created in a
        // directory that no longer exists. An earlier fix instead
        // called `.keep()` on the `TempDir`, which stopped it from
        // dropping *ever* -- fixing the encrypted path at the cost of
        // leaking every fixture-built test's directory permanently.
        // Tying its lifetime to the service's own restores ordinary
        // cleanup on every ordinary test's `Drop`, ~150 of them,
        // while keeping the fix for the handful that mint under
        // encryption.
        let dir = tempfile::tempdir().unwrap();
        let store = SupervisorStore::open_in_memory().unwrap();
        let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> = Arc::new(
            syneroym_data_db::SqliteStorageProvider::new(dir.path().join("db"), self.locked_vault)
                .unwrap(),
        );
        let key_store = Arc::new(syneroym_data_keystore::KeyStore::new());
        // An unlocked fixture must actually report its KEK as loaded:
        // `kek_is_loaded` is what gates the renewal work-list, and it
        // reads the `KeyStore`, not the storage provider's encryption
        // flag. `skip_kek_injection` is the one deliberate exception --
        // an unencrypted vault whose `KeyStore` never sees a KEK,
        // needed to tell a real read attempt apart from a
        // `kek_is_loaded()` pre-check.
        if (!self.locked_vault || self.inject_kek_anyway) && !self.skip_kek_injection {
            key_store.inject_kek([7u8; 32]).unwrap();
        }
        let vault = MasterVault::new(
            storage_provider,
            key_store.clone(),
            "supervisor".to_string(),
            self.backup_dir.clone().unwrap_or_else(|| dir.path().join("backups")),
        );
        let identity = Identity::generate().unwrap();
        let mut service = SupervisorService::new(
            "did:key:zSupervisorNode".to_string(),
            store,
            vault,
            &identity,
            false,
            test_broker(),
            "supervisor/alerts".to_string(),
            self.poll_interval_secs.unwrap_or(30),
            3,
            30,
            4,
            self.max_renewals_per_pass.unwrap_or(5),
            self.master_anchor_refresh_interval_secs.unwrap_or(12 * 3600),
            self.anchor_writer,
            self.tier1_writer,
            self.queue_tick_secs.unwrap_or(5),
            self.topology_document_not_after_secs.unwrap_or(3600),
            self.topology_document_cache_ttl_secs.unwrap_or(300),
        );
        service._fixture_tempdir = Some(dir);
        (service, key_store)
    }
}

pub(super) fn service() -> SupervisorService {
    Fixture::default().build()
}

pub(super) fn service_with_locked_vault() -> SupervisorService {
    Fixture { locked_vault: true, ..Fixture::default() }.build()
}

pub(super) fn unauthenticated_caller() -> CallerContext {
    CallerContext {
        caller_did: "did:key:zRandom".to_string(),
        app_instance: None,
        session: Default::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

pub(super) fn admin_caller(node_did: &str) -> CallerContext {
    use syneroym_rpc::{Capability, SessionContext};
    CallerContext {
        caller_did: "did:key:zAdmin".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zAdmin".to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate(node_did),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

pub(super) async fn dispatch(
    service: &SupervisorService,
    caller: CallerContext,
    method: &str,
    params: Value,
) -> RpcResult<NativeResponse> {
    service
        .dispatch(NativeInvocation {
            interface: SUPERVISOR_INTERFACE.to_string(),
            method: method.to_string(),
            params,
            caller,
        })
        .await
}

pub(super) fn expected_alert_topic(app_instance_id: &str) -> String {
    namespace_topic_for_publish(
        SUPERVISOR_RESERVED_SERVICE_ID,
        &format!("supervisor/alerts/{app_instance_id}"),
    )
}

pub(super) fn plan_json_no_services(instance: &str) -> String {
    serde_json::json!({
        "app_instance_id": instance,
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": []
    })
    .to_string()
}

/// `logical_ref` serializes as `"<instance>/<service>"`, not a nested
/// object (`LogicalServiceRef`'s `#[serde(try_from = "String")]`), and
/// `ServiceConfig` is `#[serde(flatten)]`ed onto `PlannedService`, not
/// nested under a `config` key.
pub(super) fn plan_json_one_service(
    instance: &str,
    service_name: &str,
    substrate: Option<&str>,
) -> String {
    serde_json::json!({
        "app_instance_id": instance,
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hFabricated",
            "logical_ref": format!("{instance}/{service_name}"),
            "substrate": substrate,
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "singleton"
        }]
    })
    .to_string()
}

pub(super) fn supervisor_interface() -> (wit_parser::Resolve, wit_parser::InterfaceId) {
    let wit_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../wit_interfaces/wit/supervisor/supervisor.wit");
    let mut resolve = wit_parser::Resolve::default();
    let content = std::fs::read_to_string(&wit_path).expect("failed to read supervisor.wit");
    let group = wit_parser::UnresolvedPackageGroup::parse(&wit_path, &content)
        .expect("failed to parse supervisor.wit");
    let pkg_id = resolve.push(group.main, 0).expect("failed to resolve supervisor package");
    let package = &resolve.packages[pkg_id];
    let iface_id = package.interfaces["supervisor"];
    (resolve, iface_id)
}

/// A fake `SubstrateActor` that only counts `restart` calls -- every
/// other method is unreachable from a remediation test.
#[derive(Debug, Default)]
pub(super) struct CountingActor {
    pub(super) restart_calls: Mutex<u32>,
    pub(super) held_generation_calls: Mutex<u32>,
}

#[async_trait::async_trait]
impl SubstrateActor for CountingActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by remediation tests")
    }

    async fn write_bindings(
        &self,
        _write: syneroym_sdk::BindingWrite,
    ) -> Result<Vec<syneroym_sdk::BindingWriteOutcome>, String> {
        unimplemented!("not exercised by remediation tests")
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        *self.restart_calls.lock().unwrap() += 1;
        Ok(())
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by remediation tests")
    }

    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by remediation tests")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        *self.held_generation_calls.lock().unwrap() += 1;
        Ok(Some(0))
    }
}

pub(super) fn service_health(
    l_ref: &str,
    substrate_did: &str,
    signal: Signal,
) -> health::ServiceHealth {
    let (instance, name) = l_ref.split_once('/').unwrap();
    health::ServiceHealth {
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new(instance),
            service_name: LogicalServiceName::new(name),
        },
        service_id: format!("did:key:h{name}"),
        alias: None,
        substrate_did: substrate_did.to_string(),
        signal,
        instance_certificate_issued_at: None,
        instance_certificate_expires_at: None,
        binding_epochs: Vec::new(),
        member_index: 0,
    }
}

pub(super) fn plan_json_two_services(instance: &str, a: &str, b: &str) -> String {
    serde_json::json!({
        "app_instance_id": instance,
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [
            {
                "service_id": "did:key:hFabricatedA",
                "logical_ref": format!("{instance}/{a}"),
                "substrate": null,
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            },
            {
                "service_id": "did:key:hFabricatedB",
                "logical_ref": format!("{instance}/{b}"),
                "substrate": null,
                "service_type": "tcp", "source": "127.0.0.1:9001",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            }
        ]
    })
    .to_string()
}

/// A fake `SubstrateActor` that only answers `write_bindings`, from a
/// caller-queued sequence of responses (defaulting to `Applied` once
/// the queue empties) -- every other method is unreachable from a
/// push test.
#[derive(Debug, Default)]
pub(super) struct BindingActor {
    pub(super) responses: Mutex<Vec<Result<Vec<BindingWriteOutcome>, String>>>,
    pub(super) calls: Mutex<Vec<BindingWrite>>,
}

#[async_trait::async_trait]
impl SubstrateActor for BindingActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by push tests")
    }

    async fn write_bindings(
        &self,
        write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        self.calls.lock().unwrap().push(write);
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(vec![BindingWriteOutcome::Applied])
        } else {
            responses.remove(0)
        }
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by push tests")
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by push tests")
    }

    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by push tests")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by push tests")
    }
}

pub(super) fn dependent_service(name: &str, dep_name: &str) -> PlannedService {
    PlannedService {
        service_id: ServiceId::new(format!("did:key:h{name}")),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new(name),
        },
        substrate: Some(SubstrateAlias::new("edge-1")),
        config: dummy_config(),
        resolved_dependencies: BTreeMap::from([(
            LogicalServiceName::new(dep_name),
            vec![ServiceId::new("did:key:hDepMember")],
        )]),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: Default::default(),
    }
}

pub(super) fn dummy_config() -> ServiceConfig {
    ServiceConfig {
        service_type: ServiceType::Tcp,
        source: "127.0.0.1:9000".to_string(),
        hash: None,
        interfaces: vec![],
        env: BTreeMap::new(),
        args: vec![],
        custom_config: None,
        quota: None,
        schema: None,
        rotation_policy: Default::default(),
        fdae: None,
        health_check: None,
        assets: None,
        visibility: Default::default(),
    }
}

pub(super) fn plan_with_one_dependent(svc: PlannedService) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![svc],
    }
}

/// A fake substrate for the renewal path: answers `instance_identity`
/// with a fixed, real ed25519 key (so a certificate minted over it is
/// genuinely valid), records every `renew_cert`/`restart`, and can be
/// told to fail either one.
#[derive(Debug)]
pub(super) struct RenewalActor {
    pub(super) instance_key: Identity,
    pub(super) instance_identity_error: Option<String>,
    pub(super) renew_error: Option<String>,
    pub(super) restart_error: Option<String>,
    pub(super) renewed: Mutex<Vec<(String, u64, String)>>,
    pub(super) restarted: Mutex<Vec<String>>,
}

impl Default for RenewalActor {
    fn default() -> Self {
        Self {
            instance_key: Identity::generate().unwrap(),
            instance_identity_error: None,
            renew_error: None,
            restart_error: None,
            renewed: Mutex::new(Vec::new()),
            restarted: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl SubstrateActor for RenewalActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by renewal tests")
    }

    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("not exercised by renewal tests")
    }

    async fn restart(&self, service_id: String, _generation: u64) -> Result<(), String> {
        if let Some(e) = &self.restart_error {
            return Err(e.clone());
        }
        self.restarted.lock().unwrap().push(service_id);
        Ok(())
    }

    async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
    ) -> Result<(), String> {
        if let Some(e) = &self.renew_error {
            return Err(e.clone());
        }
        self.renewed.lock().unwrap().push((service_id, generation, instance_certificate));
        Ok(())
    }

    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        if let Some(e) = &self.instance_identity_error {
            return Err(e.clone());
        }
        Ok(syneroym_sdk::InstanceIdentity {
            instance_did: substrate::derive_did_key(&self.instance_key.public_key()),
            pubkey_hex: hex::encode(self.instance_key.public_key().to_bytes()),
            installed_temporary_did: None,
        })
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by renewal tests")
    }
}

/// An `AnchorWriter` that records what it was asked to publish and can
/// be made to fail, so both the schedule and the revocation write are
/// testable with no registry.
#[derive(Debug, Default)]
pub(super) struct RecordingAnchorWriter {
    pub(super) refreshed: Mutex<Vec<String>>,
    pub(super) revoked: Mutex<Vec<(String, String)>>,
    pub(super) fail: bool,
}

#[async_trait::async_trait]
impl AnchorWriter for RecordingAnchorWriter {
    async fn refresh(&self, master: &Identity) -> Result<(), String> {
        if self.fail {
            return Err("registry unreachable".to_string());
        }
        self.refreshed.lock().unwrap().push(substrate::derive_did_key(&master.public_key()));
        Ok(())
    }

    async fn revoke_instance(&self, master: &Identity, instance_did: &str) -> Result<(), String> {
        if self.fail {
            return Err("registry unreachable".to_string());
        }
        self.revoked
            .lock()
            .unwrap()
            .push((substrate::derive_did_key(&master.public_key()), instance_did.to_string()));
        Ok(())
    }
}

/// A `Tier1Writer` that records what it was asked to publish and can be
/// made to fail, the same shape `RecordingAnchorWriter` above uses.
/// `calls` counts every attempt, successful or not -- `published` only
/// grows on success, so a test proving a failing writer is still
/// retried needs the former.
#[derive(Debug, Default)]
pub(super) struct RecordingTier1Writer {
    pub(super) published: Mutex<Vec<SignedEndpointInfo>>,
    pub(super) calls: Mutex<u32>,
    pub(super) fail: bool,
}

#[async_trait::async_trait]
impl Tier1Writer for RecordingTier1Writer {
    async fn publish(&self, record: &SignedEndpointInfo) -> Result<(), String> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            return Err("registry unreachable".to_string());
        }
        self.published.lock().unwrap().push(record.clone());
        Ok(())
    }
}

/// A member with a real master in the supervisor's vault, under the
/// same computable name `mint_and_substitute` would have stored it as
/// -- so the renewal path finds it exactly the way production does.
pub(super) async fn seeded_member(s: &SupervisorService, service_name: &str) -> String {
    let master = s
        .vault
        .get_or_mint(&format!("member-inst-1#{service_name}-0"), keys::MasterKind::Member)
        .await
        .unwrap();
    substrate::derive_did_key(&master.public_key())
}

/// A plan naming one placed member by its real master DID, with the
/// given rotation policy.
pub(super) fn plan_json_with_master(
    service_name: &str,
    master_did: &str,
    rotation_policy: &str,
) -> String {
    serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": master_did,
            "logical_ref": format!("inst-1/{service_name}"),
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": rotation_policy,
            "resolved_dependencies": {},
            "topology_mode": "singleton"
        }]
    })
    .to_string()
}

/// One member's health, carrying a certificate window. `elapsed_ratio`
/// is how far through its lifetime the certificate is at `NOW`.
pub(super) const NOW: u64 = 1_000_000;

pub(super) fn health_with_cert(
    service_name: &str,
    service_id: &str,
    substrate_did: &str,
    issued_at: u64,
    expires_at: u64,
) -> health::ServiceHealth {
    health::ServiceHealth {
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new(service_name),
        },
        service_id: service_id.to_string(),
        alias: Some(SubstrateAlias::new("edge-1")),
        substrate_did: substrate_did.to_string(),
        signal: Signal::Healthy,
        instance_certificate_issued_at: Some(issued_at),
        instance_certificate_expires_at: Some(expires_at),
        binding_epochs: Vec::new(),
        member_index: 0,
    }
}

/// 90% through a 4-hour lifetime: inside `is_near_expiry_parts`'s
/// 25%-remaining window.
pub(super) fn near_expiry_health(service_name: &str, service_id: &str) -> health::ServiceHealth {
    health_with_cert(service_name, service_id, "did:key:zEdge1", NOW - 12_960, NOW + 1_440)
}

/// Freshly issued: 0% elapsed, comfortably outside the window.
pub(super) fn fresh_health(service_name: &str, service_id: &str) -> health::ServiceHealth {
    health_with_cert(service_name, service_id, "did:key:zEdge1", NOW, NOW + 14_400)
}

pub(super) fn report_of(services: Vec<health::ServiceHealth>) -> health::HealthReport {
    health::HealthReport { substrates: Vec::new(), services }
}

pub(super) fn edge_1_actor(
    actor: Arc<RenewalActor>,
) -> BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> {
    BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))])
}

pub(super) fn edge_1_alias() -> BTreeMap<String, String> {
    BTreeMap::from([("did:key:zEdge1".to_string(), "edge-1".to_string())])
}

pub(super) fn desired_state_with_app_master(
    app_instance_id: &str,
    app_master_did: &str,
) -> DesiredState {
    DesiredState {
        app_instance_id: app_instance_id.to_string(),
        plan_json: plan_json_no_services(app_instance_id),
        inventory_json: "{}".to_string(),
        owner_did: "did:key:zOwner".to_string(),
        generation: 0,
        paused: false,
        retired: false,
        submitted_at: 0,
        updated_at: 0,
        app_master_did: app_master_did.to_string(),
    }
}

pub(super) fn adopt_field<'a>(res: &'a NativeResponse, field: &str) -> Option<&'a Value> {
    res.payload.get(field)
}

/// A fake standing in for a connected substrate on the queue worker's
/// replay path: one scripted delivery per DID, consumed in FIFO order.
/// `ConnectFails` simulates the substrate still being unreachable;
/// `Attempt` simulates a reconnect that succeeds, with `write_bindings`
/// itself returning whatever the test scripts.
#[derive(Debug, Default)]
pub(super) struct FakeQueueConnector {
    pub(super) scripted: Mutex<BTreeMap<String, VecDeque<FakeDelivery>>>,
}

#[derive(Debug)]
pub(super) enum FakeDelivery {
    /// The reconnect itself fails -- a transport failure.
    ConnectFails,
    /// Reconnected; `write_bindings` returns this outcome.
    Attempt(Result<Vec<BindingWriteOutcome>, String>),
    /// Reconnected, and the substrate answered with a `JsonRpcError` --
    /// the callee-error shape `deploy::is_callee_error` classifies as
    /// terminal, distinct from a plain transport failure (which
    /// `Attempt(Err(_))` alone cannot express: an ordinary string error
    /// is not downcastable to `JsonRpcError`, so it would be classified
    /// as transport, same as `ConnectFails`).
    CalleeError(String),
    /// The reconnect never resolves -- stands in for a substrate that
    /// never answers, so a test can prove something is genuinely
    /// in-flight when shutdown is asked for.
    /// Distinct from an absent scripted entry (`None` below), which
    /// returns an error immediately and proves nothing about
    /// abandoning in-flight work.
    Blocks,
}

impl FakeQueueConnector {
    pub(super) fn script(&self, did: &str, delivery: FakeDelivery) {
        self.scripted.lock().unwrap().entry(did.to_string()).or_default().push_back(delivery);
    }
}

#[derive(Debug)]
pub(super) struct ScriptedAttempt {
    pub(super) result: Mutex<Option<anyhow::Result<Vec<BindingWriteOutcome>>>>,
}

#[async_trait::async_trait]
impl WriteBindingsAttempt for ScriptedAttempt {
    async fn attempt_write_bindings(
        &self,
        _write: BindingWrite,
    ) -> anyhow::Result<Vec<BindingWriteOutcome>> {
        self.result.lock().unwrap().take().expect("scripted result already consumed")
    }
}

#[async_trait::async_trait]
impl QueueConnector for FakeQueueConnector {
    async fn connect(
        &self,
        entry: &SupervisorInventoryEntry,
    ) -> anyhow::Result<Arc<dyn WriteBindingsAttempt>> {
        let next = self.scripted.lock().unwrap().get_mut(&entry.did).and_then(VecDeque::pop_front);
        let result: anyhow::Result<Vec<BindingWriteOutcome>> = match next {
            Some(FakeDelivery::ConnectFails) => {
                return Err(anyhow::anyhow!("simulated connect failure"));
            }
            Some(FakeDelivery::Attempt(Ok(outcomes))) => Ok(outcomes),
            Some(FakeDelivery::Attempt(Err(msg))) => Err(anyhow::anyhow!(msg)),
            Some(FakeDelivery::CalleeError(msg)) => {
                Err(syneroym_rpc::JsonRpcError { code: -32010, message: msg, data: None }.into())
            }
            Some(FakeDelivery::Blocks) => future::pending().await,
            None => return Err(anyhow::anyhow!("no scripted delivery for {}", entry.did)),
        };
        Ok(Arc::new(ScriptedAttempt { result: Mutex::new(Some(result)) }))
    }
}

/// Every real fake in this crate implements the full `SubstrateActor`
/// trait, not just `WriteBindingsAttempt` -- `deploy::build_durable_actor`
/// requires both, since a durable actor still answers every other
/// action synchronously.
#[derive(Debug, Default)]
pub(super) struct FakeSubstrateClient {
    pub(super) write_bindings_outcome: Mutex<Option<Result<Vec<BindingWriteOutcome>, String>>>,
}

#[async_trait::async_trait]
impl WriteBindingsAttempt for FakeSubstrateClient {
    async fn attempt_write_bindings(
        &self,
        _write: BindingWrite,
    ) -> anyhow::Result<Vec<BindingWriteOutcome>> {
        match self.write_bindings_outcome.lock().unwrap().take().expect("outcome not set for test")
        {
            Ok(outcomes) => Ok(outcomes),
            Err(msg) => Err(anyhow::anyhow!(msg)),
        }
    }
}

#[async_trait::async_trait]
impl SubstrateActor for FakeSubstrateClient {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by the queue worker tests")
    }

    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("DurableActor never calls the inner T::write_bindings directly")
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by the queue worker tests")
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by the queue worker tests")
    }

    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by the queue worker tests")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by the queue worker tests")
    }
}

pub(super) fn test_binding_write(service_id: &str, app_instance_id: &str) -> BindingWrite {
    BindingWrite {
        service_id: service_id.to_string(),
        app_instance_id: app_instance_id.to_string(),
        bindings: vec![],
        generation: 0,
    }
}

/// Submits minimal desired state whose inventory names one substrate
/// alias/DID pair -- enough for `deliver_queued_item` to resolve a
/// queued item's target, without a real plan or a real deploy.
pub(super) fn seed_inventory(store: &SupervisorStore, app_instance_id: &str, substrate_did: &str) {
    let inventory_json =
        serde_json::json!({"edge-1": {"did": substrate_did, "api_url": "http://127.0.0.1:1"}})
            .to_string();
    store
        .submit(
            app_instance_id,
            &plan_json_no_services(app_instance_id),
            &inventory_json,
            "did:key:owner",
            0,
        )
        .unwrap();
}

pub(super) fn enqueue_test_item(
    store: &SupervisorStore,
    app_instance_id: &str,
    logical_ref: &str,
    substrate_did: &str,
    write: BindingWrite,
) -> i64 {
    let key = QueueKey {
        app_instance_id: app_instance_id.to_string(),
        logical_ref: logical_ref.to_string(),
        substrate_did: substrate_did.to_string(),
    };
    let payload = outbox::QueuedBindingWrite { substrate_did: substrate_did.to_string(), write };
    store
        .queue
        .enqueue(
            app_instance_id,
            &key.to_string(),
            &serde_json::to_vec(&payload).unwrap(),
            outbox::now_ms(),
        )
        .unwrap()
}

pub(super) fn scheduled_service(name: &str, member_index: u32, cron: &str) -> PlannedService {
    PlannedService {
        service_id: ServiceId::new(format!("did:key:h{name}{member_index}")),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new(name),
        },
        substrate: Some(SubstrateAlias::new("edge-1")),
        config: dummy_config(),
        resolved_dependencies: BTreeMap::new(),
        topology_mode: if member_index > 0 {
            TopologyMode::Redundant
        } else {
            TopologyMode::Singleton
        },
        member_index,
        schedule: Some(ScheduleSpec {
            cron: cron.to_string(),
            interface: InterfaceName::new("scheduled-driver"),
            method: "tick".to_string(),
            params: None,
            timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
        }),
        sharding_strategy: None,
        topology_visibility: Default::default(),
    }
}

pub(super) fn plan_with_schedule(members: Vec<PlannedService>) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: members,
    }
}

pub(super) fn scheduled_health(
    name: &str,
    member_index: u32,
    substrate_did: &str,
    signal: Signal,
) -> health::ServiceHealth {
    health::ServiceHealth {
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new(name),
        },
        service_id: format!("did:key:h{name}{member_index}"),
        alias: Some(SubstrateAlias::new("edge-1")),
        substrate_did: substrate_did.to_string(),
        signal,
        instance_certificate_issued_at: None,
        instance_certificate_expires_at: None,
        binding_epochs: Vec::new(),
        member_index,
    }
}

/// A timestamp exactly on a cron minute boundary, offset by
/// `offset_secs` -- lets the watermark/grace tests reason about
/// `"* * * * *"` occurrences in exact whole seconds rather than
/// hoping `NOW` happens to align.
pub(super) fn minute(offset_secs: i64) -> u64 {
    use chrono::{TimeZone, Utc};
    let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 5, 0).unwrap().timestamp();
    u64::try_from(base + offset_secs).unwrap()
}

/// A fake `SubstrateActor` exercising only `run_scheduled`, for
/// `run_due_schedules`'s own tests. `error`/`delay` are behind a
/// `Mutex` so a test can flip the outcome between two calls in the
/// same actor, the same shape `DurableTestActor` uses.
#[derive(Debug, Default)]
pub(super) struct ScheduledActor {
    pub(super) error: Mutex<Option<String>>,
    pub(super) delay: Option<Duration>,
}

#[async_trait::async_trait]
impl SubstrateActor for ScheduledActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        if let Some(e) = self.error.lock().unwrap().clone() {
            return Err(e);
        }
        Ok(())
    }
}

pub(super) fn run_decision(
    logical_ref: &str,
    service_id: &str,
    substrate_did: &str,
) -> ScheduleDecision {
    ScheduleDecision::Run {
        logical_ref: logical_ref.to_string(),
        service_id: service_id.to_string(),
        substrate_did: substrate_did.to_string(),
        member_index: 0,
        schedule: ScheduleSpec {
            cron: "* * * * *".to_string(),
            interface: InterfaceName::new("scheduled-driver"),
            method: "tick".to_string(),
            params: None,
            timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
        },
    }
}

/// A direct proof of the watermark's ordering: `record_schedule_started`
/// must have already landed by the time the target is called, not after --
/// checked from *inside* the call itself, before its own outcome is
/// known.
#[derive(Debug)]
pub(super) struct AssertsStartedBeforeCallActor {
    pub(super) store: SupervisorStore,
    pub(super) expected_run_at: i64,
}

#[async_trait::async_trait]
impl SubstrateActor for AssertsStartedBeforeCallActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by this test")
    }
    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("not exercised by this test")
    }
    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by this test")
    }
    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by this test")
    }
    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by this test")
    }
    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by this test")
    }
    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        let states = self.store.schedule_states("inst-1").unwrap();
        let state = states
            .get("inst-1/worker")
            .expect("record_schedule_started must have written before the call");
        assert_eq!(state.last_run_at, Some(self.expected_run_at));
        Ok(())
    }
}

pub(super) fn plan_json_with_schedule(service_name: &str, master_did: &str) -> String {
    serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": master_did,
            "logical_ref": format!("inst-1/{service_name}"),
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "resolved_dependencies": {},
            "topology_mode": "singleton",
            "schedule": {
                "cron": "* * * * *",
                "interface": "scheduled-driver",
                "method": "tick"
            }
        }]
    })
    .to_string()
}

pub(super) fn plan_json_n_member_service_with_vis(
    instance: &str,
    service_name: &str,
    mode: &str,
    n: u32,
    visibility: &str,
    topology_visibility: &str,
) -> String {
    let services: Vec<_> = (0..n)
        .map(|i| {
            serde_json::json!({
                "service_id": format!("did:key:hMember{i}"),
                "logical_ref": format!("{instance}/{service_name}"),
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": mode,
                "member_index": i,
                "visibility": visibility,
                "topology_visibility": topology_visibility,
            })
        })
        .collect();
    serde_json::json!({
        "app_instance_id": instance,
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": services,
    })
    .to_string()
}

pub(super) fn plan_json_n_member_service(
    instance: &str,
    service_name: &str,
    mode: &str,
    n: u32,
) -> String {
    plan_json_n_member_service_with_vis(instance, service_name, mode, n, "private", "restricted")
}

/// Directly submits desired state and adopts an app master, bypassing
/// the RPC `submit`/`adopt` pipeline's mint/apply machinery -- this
/// slice's `resolve` reads only the store and the vault, so a fully
/// applied plan (which needs a live substrate connection) is not
/// needed to exercise it. Returns the app master DID `resolve` is
/// looked up by.
pub(super) async fn adopted_instance(
    s: &SupervisorService,
    instance_id: &str,
    plan_json: &str,
) -> String {
    s.store.submit(instance_id, plan_json, "{}", "did:key:owner", 0).unwrap();
    let (app_did, _) = keys::app_master(&s.vault, instance_id).await.unwrap();
    s.store.record_adopt(instance_id, 1, &app_did).unwrap();
    app_did
}

/// A caller holding `supervisor/resolve` on exactly `synapp:<app_did>`
/// -- not `substrate/admin`, so this is an honest reading of the
/// reference scenario's "a caller outside the app instance".
pub(super) fn resolve_grant(caller_did: &str, app_did: &str) -> CallerContext {
    use syneroym_rpc::{Capability, SessionContext};
    CallerContext {
        caller_did: caller_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: caller_did.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri(format!("synapp:{app_did}")),
                can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

pub(super) fn caller_with_no_capabilities(caller_did: &str) -> CallerContext {
    CallerContext {
        caller_did: caller_did.to_string(),
        app_instance: None,
        session: Default::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

pub(super) fn decode_signed_document(payload: Value) -> SignedTopologyDocument {
    serde_json::from_value(payload).unwrap()
}
