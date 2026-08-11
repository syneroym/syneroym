//! Substrate Service discovery registry
//!
//! Tracks local running/deployed micro-services (WASM, TCP, Podman, native
//! host), enabling internal service-to-service discovery (Internal
//! Micro-Discovery).

use std::{
    fmt::{self, Debug, Formatter},
    sync::Arc,
};

use anyhow::Result;
use dashmap::DashMap;
use syneroym_identity::DelegationCertificate;

use crate::{
    storage::{AppInstanceManagement, DeployFacts, EndpointStorage, MockStorage},
    util,
};

/// The reserved native-capability interface names every deployed service
/// (regardless of `service-type`) automatically gets registered as
/// `SubstrateEndpoint::NativeHostChannel` entries, pointing at
/// `SynSvcNativeService::dispatch` -- no WASM component or app-declared
/// interface required (`crates/control_plane/src/service/orchestration.rs`'s
/// `deploy`). Shared here (M04A Slice A1) so `control_plane`'s registration
/// logic, `router`'s guest native-capability proxy gate
/// (`ProxyRouter::check_native_capability_gate`), and their tests all read
/// from one list rather than three independently-maintained copies that can
/// drift.
///
/// "http-native", not the bare "http": `roymctl svc deploy --interfaces http
/// --tcp ...` is an existing, real convention for declaring a TCP/container
/// service's own plain HTTP-serving interface (see
/// `crates/substrate/tests/e2e/global-setup.ts`) -- reserving the bare
/// "http" name here collided with it (registering this native-capability
/// endpoint under the same interface name silently overwrote the app's own
/// `TcpHostPort` registration, discovered via `mise run test:e2e` breaking
/// end to end during M3B Slice 7's own verification).
pub const NATIVE_CAPABILITY_INTERFACES: [&str; 6] =
    ["data-layer", "vault", "app-config", "blob-store", "messaging", "http-native"];

/// `orchestrator`/`security`: every substrate registers these under its
/// **own** DID (`runtime.rs`'s `setup_router`), not under a deployed
/// service's id. Unlike `NATIVE_CAPABILITY_INTERFACES`, a guest has no
/// same-service exemption to fall back to -- there is no legitimate reason
/// for a WASM guest to reach either directly through the proxy, so these are
/// denied outright rather than gated on `target_service`.
pub const NODE_NATIVE_INTERFACES: [&str; 2] = ["orchestrator", "security"];

/// A deployable entity within the Substrate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SubstrateEndpoint {
    /// A WASM component communicating via WASM channels
    WasmChannel { service_id: String },
    /// A containerized service running via Podman
    PodmanSocket { socket_path: String },
    /// A native Rust host capability or service (e.g. `SubstrateService`)
    NativeHostChannel { service_id: String },
    /// An already existing TCP service running on a host:port
    TcpHostPort { host: String, port: u16 },
}

/// The Endpoint Registry tracks where local Services are currently executing.
/// It acts as Internal Micro-Discovery.
#[derive(Clone)]
pub struct EndpointRegistry {
    /// Thread-safe shared map of (`service_id`, `interface_name`) to
    /// `LocalEndpoint`
    active_endpoints: Arc<DashMap<(String, String), SubstrateEndpoint>>,
    /// Secondary map for fast lookup by interface hash: (`service_id`,
    /// `interface_hash`) -> `interface_name`
    interface_hashes: Arc<DashMap<(String, String), String>>,
    /// `service_id` -> `owner_did` (M04A Slice B7a). Separate from
    /// `active_endpoints`, which is keyed per interface.
    service_owners: Arc<DashMap<String, String>>,
    /// `service_id` -> the installed `DelegationCertificate` binding this
    /// substrate's derived instance key to the member master that
    /// `service_id` names. Absent for a service deployed without a master
    /// (the pre-existing "service is its own master" fallback).
    service_certs: Arc<DashMap<String, DelegationCertificate>>,
    /// `service_id` -> (`service_type`, `health_check_json`) recorded at
    /// deploy (M05A A4). Absent for a service deployed by a pre-A4 binary,
    /// which is why every reader treats a missing entry as "unknown" rather
    /// than guessing.
    service_deploy_facts: Arc<DashMap<String, DeployFacts>>,
    /// `service_id` -> (`app_instance_id`, `service_name`) for a service
    /// deployed as part of an app instance (A2). Absent for a standalone
    /// `svc deploy`, which resolves no declared dependencies.
    service_app_contexts: Arc<DashMap<String, (String, String)>>,
    /// `app_instance_id` -> its management stamp (M05A A5a, replacing A2's
    /// `app_instance_owners`): who first declared it (first-write-wins,
    /// unchanged from A2) plus which supervisor, if any, manages it at
    /// which generation (ADR-0021 §4).
    app_instance_management: Arc<DashMap<String, AppInstanceManagement>>,
    /// Stable storage connection for persistence
    storage: Arc<dyn EndpointStorage>,
}

impl Debug for EndpointRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointRegistry")
            .field("active_endpoints", &self.active_endpoints)
            .field("interface_hashes", &self.interface_hashes)
            // storage does not implement Debug, so we skip it
            .finish()
    }
}

impl EndpointRegistry {
    /// Create a new Endpoint Registry with the given stable storage.
    pub async fn new(storage: Arc<dyn EndpointStorage>) -> Result<Self> {
        let registry = Self {
            active_endpoints: Arc::new(DashMap::new()),
            interface_hashes: Arc::new(DashMap::new()),
            service_owners: Arc::new(DashMap::new()),
            service_certs: Arc::new(DashMap::new()),
            service_deploy_facts: Arc::new(DashMap::new()),
            service_app_contexts: Arc::new(DashMap::new()),
            app_instance_management: Arc::new(DashMap::new()),
            storage,
        };

        registry.load_from_db().await?;

        Ok(registry)
    }

    /// Load endpoints from stable storage into memory map on startup
    async fn load_from_db(&self) -> Result<()> {
        let endpoints = self.storage.load_all().await?;

        for (service_id, interface_name, endpoint) in endpoints {
            let hash = util::short_hash(&interface_name);
            self.interface_hashes.insert((service_id.clone(), hash), interface_name.clone());
            self.active_endpoints.insert((service_id, interface_name), endpoint);
        }

        for (service_id, owner_did) in self.storage.load_all_owners().await? {
            self.service_owners.insert(service_id, owner_did);
        }

        for (service_id, certificate_json) in self.storage.load_all_certs().await? {
            match DelegationCertificate::from_json(&certificate_json) {
                Ok(cert) => {
                    self.service_certs.insert(service_id, cert);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse stored instance certificate for service_id: \
                         {service_id}: {e:?}"
                    );
                }
            }
        }

        for (service_id, service_type, check, manifest_hash) in
            self.storage.load_all_deploy_facts().await?
        {
            self.service_deploy_facts.insert(service_id, (service_type, check, manifest_hash));
        }

        for (service_id, app_instance_id, service_name) in
            self.storage.load_all_app_contexts().await?
        {
            self.service_app_contexts.insert(service_id, (app_instance_id, service_name));
        }

        for (app_instance_id, management) in self.storage.load_all_app_instance_management().await?
        {
            self.app_instance_management.insert(app_instance_id, management);
        }
        Ok(())
    }

    /// Register a local service. Stores it in memory and stable storage.
    pub async fn register(
        &self,
        service_id: String,
        interface_name: String,
        endpoint: SubstrateEndpoint,
    ) -> Result<()> {
        self.storage.save(&service_id, &interface_name, &endpoint).await?;

        let hash = util::short_hash(&interface_name);
        self.interface_hashes.insert((service_id.clone(), hash), interface_name.clone());
        self.active_endpoints.insert((service_id, interface_name), endpoint);
        Ok(())
    }

    /// Canonicalizes the interface a caller named into one this service
    /// actually registered. Three inputs, one rule -- the party holding
    /// the candidate set does the reversing:
    ///
    /// - an exact registered name;
    /// - a `short_hash` of one (a caller off the network carries hashes, not
    ///   names);
    /// - **empty**, meaning "this service's one app-declared interface"
    ///   (ADR-0022 §7's hostname omits `-i` when a caller has nothing to say
    ///   about it, D-S3-15).
    ///
    /// The empty case filters [`NATIVE_CAPABILITY_INTERFACES`] and
    /// [`NODE_NATIVE_INTERFACES`], which every deployed service (the
    /// latter, only the node's own DID) is registered under regardless of
    /// type, so "only one" means only one the app itself declared. Zero or
    /// two or more is `None`: an ambiguous interface is refused, never
    /// guessed.
    #[must_use]
    pub fn resolve_interface(&self, service_id: &str, interface_name: &str) -> Option<String> {
        if interface_name.is_empty() {
            let mut declared =
                self.lookup_by_service(service_id).into_iter().filter(|(name, _)| {
                    !NATIVE_CAPABILITY_INTERFACES.contains(&name.as_str())
                        && !NODE_NATIVE_INTERFACES.contains(&name.as_str())
                });
            let only = declared.next()?;
            return if declared.next().is_none() { Some(only.0) } else { None };
        }

        if self.active_endpoints.contains_key(&(service_id.to_string(), interface_name.to_string()))
        {
            return Some(interface_name.to_string());
        }

        self.interface_hashes
            .get(&(service_id.to_string(), interface_name.to_string()))
            .map(|canonical| canonical.clone())
    }

    /// Lookup a destination for an incoming request.
    /// Returns the endpoint and the canonical interface name it was registered
    /// under. The canonical interface name may differ from `interface_name`
    /// when a short hash -- or, since S3, an empty string -- is provided.
    #[must_use]
    pub fn lookup(
        &self,
        service_id: &str,
        interface_name: &str,
    ) -> Option<(SubstrateEndpoint, String)> {
        let canonical = self.resolve_interface(service_id, interface_name)?;
        let ep = self.active_endpoints.get(&(service_id.to_string(), canonical.clone()))?.clone();
        Some((ep, canonical))
    }

    /// Lookup all endpoints for a given `service_id`.
    #[must_use]
    pub fn lookup_by_service(&self, service_id: &str) -> Vec<(String, SubstrateEndpoint)> {
        self.active_endpoints
            .iter()
            .filter(|entry| entry.key().0 == service_id)
            .map(|entry| (entry.key().1.clone(), entry.value().clone()))
            .collect()
    }

    /// Remove a service from registry
    pub async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
        self.storage.remove(service_id, interface_name).await?;

        let hash = util::short_hash(interface_name);
        self.interface_hashes.remove(&(service_id.to_string(), hash));
        self.active_endpoints.remove(&(service_id.to_string(), interface_name.to_string()));
        Ok(())
    }

    /// Returns a list of all currently registered endpoints
    #[must_use]
    pub fn get_all_endpoints(&self) -> Vec<(String, String, SubstrateEndpoint)> {
        self.active_endpoints
            .iter()
            .map(|entry| {
                let (service_id, interface_name) = entry.key().clone();
                (service_id, interface_name, entry.value().clone())
            })
            .collect()
    }

    /// Creates a mock registry with in-memory storage for testing.
    #[must_use]
    pub fn new_mock(storage: Arc<MockStorage>) -> Self {
        Self {
            active_endpoints: Arc::new(DashMap::new()),
            interface_hashes: Arc::new(DashMap::new()),
            service_owners: Arc::new(DashMap::new()),
            service_certs: Arc::new(DashMap::new()),
            service_deploy_facts: Arc::new(DashMap::new()),
            service_app_contexts: Arc::new(DashMap::new()),
            app_instance_management: Arc::new(DashMap::new()),
            storage,
        }
    }

    /// Record the owner of a deployed service (M04A Slice B7a). Overwrites
    /// any existing entry -- the takeover check is the caller's
    /// responsibility (`ControlPlaneService::deploy`), not this store's.
    pub async fn set_owner(&self, service_id: String, owner_did: String) -> Result<()> {
        self.storage.save_owner(&service_id, &owner_did).await?;
        self.service_owners.insert(service_id, owner_did);
        Ok(())
    }

    /// The recorded owner, or `None` for a service deployed before B7a.
    #[must_use]
    pub fn owner_of(&self, service_id: &str) -> Option<String> {
        self.service_owners.get(service_id).map(|e| e.value().clone())
    }

    /// Forget `service_id`'s owner. Idempotent.
    pub async fn remove_owner(&self, service_id: &str) -> Result<()> {
        self.storage.remove_owner(service_id).await?;
        self.service_owners.remove(service_id);
        Ok(())
    }

    /// Install `cert` as `service_id`'s instance certificate (upsert -- a
    /// renewal replaces in place).
    pub async fn set_instance_cert(
        &self,
        service_id: String,
        cert: DelegationCertificate,
    ) -> Result<()> {
        self.storage.save_cert(&service_id, &cert.to_json()?).await?;
        self.service_certs.insert(service_id, cert);
        Ok(())
    }

    /// The installed instance certificate, or `None` for a service deployed
    /// without a member master.
    #[must_use]
    pub fn instance_cert(&self, service_id: &str) -> Option<DelegationCertificate> {
        self.service_certs.get(service_id).map(|e| e.value().clone())
    }

    /// Forget `service_id`'s instance certificate. Idempotent.
    pub async fn remove_instance_cert(&self, service_id: &str) -> Result<()> {
        self.storage.remove_cert(service_id).await?;
        self.service_certs.remove(service_id);
        Ok(())
    }

    /// Every installed instance certificate, keyed by `service_id`. For
    /// consumers that need the whole set rather than one lookup: the
    /// heartbeat sweep that warns on a near-expiry certificate, and
    /// `EndpointPublisher::publish_all_services`' id union with the
    /// stored-record directory scan.
    #[must_use]
    pub fn all_instance_certs(&self) -> Vec<(String, DelegationCertificate)> {
        self.service_certs.iter().map(|e| (e.key().clone(), e.value().clone())).collect()
    }

    /// Record what a deploy said `service_id` is, its declared health
    /// check if any, and the canonical content hash of what was actually
    /// installed (M05A A4, `manifest_hash` added A5a -- upsert; a
    /// redeploy that drops the check writes `None`, clearing it by
    /// construction).
    pub async fn set_deploy_facts(
        &self,
        service_id: String,
        service_type: String,
        health_check_json: Option<String>,
        manifest_hash: Option<String>,
    ) -> Result<()> {
        self.storage
            .save_deploy_facts(
                &service_id,
                &service_type,
                health_check_json.as_deref(),
                manifest_hash.as_deref(),
            )
            .await?;
        self.service_deploy_facts
            .insert(service_id, (service_type, health_check_json, manifest_hash));
        Ok(())
    }

    /// The recorded `(service_type, health_check_json, manifest_hash)`, or
    /// `None` for a service deployed by a pre-A4 binary.
    #[must_use]
    pub fn deploy_facts(&self, service_id: &str) -> Option<DeployFacts> {
        self.service_deploy_facts.get(service_id).map(|e| e.value().clone())
    }

    /// Forget `service_id`'s deploy facts. Idempotent.
    pub async fn remove_deploy_facts(&self, service_id: &str) -> Result<()> {
        self.storage.remove_deploy_facts(service_id).await?;
        self.service_deploy_facts.remove(service_id);
        Ok(())
    }

    /// Record which app instance and logical name `service_id` was deployed
    /// as (A2, upsert).
    pub async fn set_app_context(
        &self,
        service_id: String,
        app_instance_id: String,
        service_name: String,
    ) -> Result<()> {
        self.storage.save_app_context(&service_id, &app_instance_id, &service_name).await?;
        self.service_app_contexts.insert(service_id, (app_instance_id, service_name));
        Ok(())
    }

    /// The recorded `(app_instance_id, service_name)`, or `None` for a
    /// standalone deploy that participates in no app.
    #[must_use]
    pub fn app_context_of(&self, service_id: &str) -> Option<(String, String)> {
        self.service_app_contexts.get(service_id).map(|e| e.value().clone())
    }

    /// Forget `service_id`'s app context and every binding row it wrote
    /// (A2). Idempotent.
    pub async fn remove_app_context(&self, service_id: &str) -> Result<()> {
        self.storage.remove_app_context(service_id).await?;
        self.service_app_contexts.remove(service_id);
        Ok(())
    }

    /// Persist one dependency binding (A2). The in-memory `AppRegistry` is
    /// written separately by the caller -- this store only makes the write
    /// survive a restart.
    pub async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        entry_json: &str,
    ) -> Result<()> {
        self.storage.save_binding(service_id, app_instance_id, dependency_name, entry_json).await
    }

    /// Every persisted binding, for the composition root's startup replay
    /// (A2).
    pub async fn all_bindings(&self) -> Result<Vec<(String, String, String, String)>> {
        self.storage.load_all_bindings().await
    }

    /// One persisted binding's `entry_json`, or `None` if `service_id`
    /// declares no such dependency (M05A A5a). The epoch guard's read.
    pub async fn binding_of(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> Result<Option<String>> {
        self.storage.load_binding(service_id, dependency_name).await
    }

    /// Every persisted binding for one service, as (`dependency_name`,
    /// `entry_json`) (M05A A5a). `status`'s per-dependent convergence
    /// report.
    pub async fn bindings_of(&self, service_id: &str) -> Result<Vec<(String, String)>> {
        self.storage.load_bindings_for(service_id).await
    }

    /// `app_instance_id`'s management stamp, or `None` if no deploy has
    /// ever named it here (M05A A5a, replacing A2's `app_instance_owner_
    /// of`). Mirrors `owner_of`.
    #[must_use]
    pub fn app_instance_management_of(
        &self,
        app_instance_id: &str,
    ) -> Option<AppInstanceManagement> {
        self.app_instance_management.get(app_instance_id).map(|e| e.value().clone())
    }

    /// Record `management` for `app_instance_id` (upsert). The takeover /
    /// generation-tiebreak logic is the caller's responsibility
    /// (`ControlPlaneService::check_generation`), not this store's,
    /// mirroring `set_owner`.
    pub async fn set_app_instance_management(
        &self,
        app_instance_id: String,
        management: AppInstanceManagement,
    ) -> Result<()> {
        self.storage.save_app_instance_management(&app_instance_id, &management).await?;
        self.app_instance_management.insert(app_instance_id, management);
        Ok(())
    }

    /// Forget `app_instance_id`'s management stamp. Idempotent. Called
    /// when the last service of an instance is undeployed (M05A A5a) --
    /// the standing backlog row `app_instance_owners` rows never get
    /// forgotten.
    pub async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()> {
        self.storage.remove_app_instance_management(app_instance_id).await?;
        self.app_instance_management.remove(app_instance_id);
        Ok(())
    }

    /// The `service_id` of a service that still records `app_instance_id`
    /// as its app context, if any (M05A A5a) -- used to decide when an app
    /// instance's management row can be forgotten. Unlike a per-service
    /// context lookup, this has to scan: the map is keyed by `service_id`.
    #[must_use]
    pub fn app_context_of_any(&self, app_instance_id: &str) -> Option<String> {
        self.service_app_contexts
            .iter()
            .find(|e| e.value().0 == app_instance_id)
            .map(|e| e.key().clone())
    }
}

#[cfg(test)]
mod tests {
    use syneroym_identity::{Identity, delegation::SCOPE_SERVICE_INSTANCE};

    use super::*;
    use crate::storage::MockStorage;

    #[tokio::test]
    async fn test_registry_lifecycle() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

        // 1. Register
        let service = "test-service".to_string();
        let iface = "health".to_string();
        let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

        registry.register(service.clone(), iface.clone(), endpoint.clone()).await.unwrap();

        // 2. Lookup
        let (found, canonical) = registry.lookup(&service, &iface).unwrap();
        assert_eq!(canonical, iface);
        match found {
            SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
            _ => panic!("Wrong endpoint type"),
        }

        // 3. Persistence check (new registry instance with same storage)
        let registry2 = EndpointRegistry::new(storage).await.unwrap();
        assert!(registry2.lookup(&service, &iface).is_some());

        // 4. Remove
        registry2.remove(&service, &iface).await.unwrap();
        assert!(registry2.lookup(&service, &iface).is_none());
    }
    #[tokio::test]
    async fn test_registry_empty_interface() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();

        let service = "empty-iface-service".to_string();
        let iface = String::new();
        let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

        registry.register(service.clone(), iface.clone(), endpoint.clone()).await.unwrap();

        let (found, canonical) = registry.lookup(&service, &iface).unwrap();
        assert_eq!(canonical, "");
        match found {
            SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
            _ => panic!("Wrong endpoint type"),
        }
    }

    /// Test 79: D-S3-15, with the six `NATIVE_CAPABILITY_INTERFACES` also
    /// registered, which is what makes the naive "only one endpoint" rule
    /// wrong (§0.11).
    #[tokio::test]
    async fn an_empty_interface_resolves_to_the_only_app_declared_one() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        let service = "svc-with-one-declared-iface".to_string();
        let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

        for native in NATIVE_CAPABILITY_INTERFACES {
            registry.register(service.clone(), native.to_string(), endpoint.clone()).await.unwrap();
        }
        registry.register(service.clone(), "default".to_string(), endpoint.clone()).await.unwrap();

        let (found, canonical) = registry.lookup(&service, "").unwrap();
        assert_eq!(canonical, "default");
        match found {
            SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
            _ => panic!("Wrong endpoint type"),
        }
    }

    /// Finding A4: `NODE_NATIVE_INTERFACES` (`orchestrator`/`security`) is
    /// filtered by the empty branch exactly like `NATIVE_CAPABILITY_INTERFACES`
    /// -- neither is ever "the one app-declared interface" a caller with no
    /// interface named actually meant, even though today `orchestrator`/
    /// `security` are only ever registered under a node's own DID and never
    /// under a deployed service's id (so this was previously unreachable in
    /// practice, only in principle).
    #[tokio::test]
    async fn an_empty_interface_never_resolves_to_a_node_native_interface() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        let service = "svc-with-node-native-only".to_string();
        let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

        for native in NODE_NATIVE_INTERFACES {
            registry.register(service.clone(), native.to_string(), endpoint.clone()).await.unwrap();
        }
        assert!(registry.lookup(&service, "").is_none());

        // With one genuine app-declared interface alongside them, that one
        // still resolves -- the node-native pair is filtered out, not
        // counted toward the ambiguity check.
        registry.register(service.clone(), "default".to_string(), endpoint.clone()).await.unwrap();
        let (_, canonical) = registry.lookup(&service, "").unwrap();
        assert_eq!(canonical, "default");
    }

    /// Test 80: the ambiguity and the empty-set halves, both `None`, never
    /// a guess.
    #[tokio::test]
    async fn an_empty_interface_is_refused_when_two_are_declared_and_when_none_is() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        let endpoint_of = |id: &str| SubstrateEndpoint::WasmChannel { service_id: id.to_string() };

        // Zero app-declared interfaces (only native ones): refused.
        let none_declared = "svc-only-native".to_string();
        for native in NATIVE_CAPABILITY_INTERFACES {
            registry
                .register(none_declared.clone(), native.to_string(), endpoint_of(&none_declared))
                .await
                .unwrap();
        }
        assert!(registry.lookup(&none_declared, "").is_none());

        // Two app-declared interfaces: refused.
        let two_declared = "svc-two-declared".to_string();
        registry
            .register(two_declared.clone(), "default".to_string(), endpoint_of(&two_declared))
            .await
            .unwrap();
        registry
            .register(two_declared.clone(), "admin".to_string(), endpoint_of(&two_declared))
            .await
            .unwrap();
        assert!(registry.lookup(&two_declared, "").is_none());
    }

    /// Test 81: the scoped form of D-S3-15's property -- at the hop that
    /// resolves the service, the interface a downstream check sees is the
    /// registered name, exactly as on the hash path (covered above by
    /// `an_empty_interface_resolves_to_the_only_app_declared_one`). Its
    /// paired assertion, here, is the relay case: for a service this
    /// registry does not host, `lookup` (and `resolve_interface`) return
    /// `None` regardless of the interface a caller named -- empty or
    /// hashed alike -- which is exactly the condition
    /// `RouteHandler::handle_stream` (`crates/router/src/route_handler/io.rs`)
    /// uses to forward the original, uncanonicalized preamble to the next
    /// hop rather than terminate it here. A relay hop does not host the
    /// service and cannot know its interface names, so it must not guess
    /// -- the same rule it already applies to an unresolved `short_hash`.
    #[tokio::test]
    async fn the_terminating_hop_canonicalizes_before_any_capability_check() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        let hosted = "svc-hosted-here".to_string();
        registry
            .register(
                hosted.clone(),
                "default".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: hosted.clone() },
            )
            .await
            .unwrap();

        // Terminating: an empty interface canonicalizes to the one
        // app-declared interface (the property test 79 already pins;
        // repeated here as the paired half of this test's own claim).
        let (_, canonical) = registry.lookup(&hosted, "").unwrap();
        assert_eq!(canonical, "default");

        // Relay: a service this registry does not host resolves to
        // nothing, for every interface shape a caller might have named --
        // empty, a hash, or a literal name -- which is what makes
        // `handle_stream` take the relay branch and forward the original
        // preamble untouched instead of canonicalizing here.
        let unhosted = "svc-hosted-elsewhere";
        assert!(registry.resolve_interface(unhosted, "").is_none());
        assert!(registry.resolve_interface(unhosted, "default").is_none());
        assert!(registry.resolve_interface(unhosted, &util::short_hash("default")).is_none());
    }

    /// M04A Slice B7a: `set_owner`/`owner_of`/`remove_owner` round-trip, and
    /// persist across a second `EndpointRegistry::new` on the same storage
    /// (mirrors `test_registry_lifecycle`'s persistence step).
    #[tokio::test]
    async fn test_owner_round_trip_and_persistence() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

        assert_eq!(registry.owner_of("svc-1"), None);

        registry.set_owner("svc-1".to_string(), "did:key:zOwner".to_string()).await.unwrap();
        assert_eq!(registry.owner_of("svc-1"), Some("did:key:zOwner".to_string()));

        let registry2 = EndpointRegistry::new(storage).await.unwrap();
        assert_eq!(registry2.owner_of("svc-1"), Some("did:key:zOwner".to_string()));

        registry2.remove_owner("svc-1").await.unwrap();
        assert_eq!(registry2.owner_of("svc-1"), None);
    }

    #[tokio::test]
    async fn test_owner_of_unknown_service_is_none() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        assert_eq!(registry.owner_of("never-deployed"), None);
    }

    fn test_cert(master: &Identity, service_id: &str) -> DelegationCertificate {
        let instance = Identity::generate().unwrap();
        let mut cert = DelegationCertificate::issue(
            master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        cert.temporary_did = service_id.to_string();
        cert
    }

    #[tokio::test]
    async fn an_instance_certificate_round_trips_through_storage() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage.clone()).await.unwrap();
        let master = Identity::generate().unwrap();

        assert_eq!(registry.instance_cert("svc-1"), None);

        let cert = test_cert(&master, "svc-1");
        registry.set_instance_cert("svc-1".to_string(), cert.clone()).await.unwrap();
        assert_eq!(registry.instance_cert("svc-1"), Some(cert.clone()));

        // Persists across a second `EndpointRegistry::new` on the same storage.
        let registry2 = EndpointRegistry::new(storage).await.unwrap();
        assert_eq!(registry2.instance_cert("svc-1"), Some(cert));
    }

    #[tokio::test]
    async fn removing_a_service_forgets_its_instance_certificate() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        let master = Identity::generate().unwrap();

        let cert = test_cert(&master, "svc-1");
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();
        assert!(registry.instance_cert("svc-1").is_some());

        registry.remove_instance_cert("svc-1").await.unwrap();
        assert_eq!(registry.instance_cert("svc-1"), None);
    }

    #[tokio::test]
    async fn all_instance_certs_returns_every_installed_certificate() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage).await.unwrap();
        assert!(registry.all_instance_certs().is_empty());

        let master = Identity::generate().unwrap();
        registry.set_instance_cert("svc-1".to_string(), test_cert(&master, "svc-1")).await.unwrap();
        registry.set_instance_cert("svc-2".to_string(), test_cert(&master, "svc-2")).await.unwrap();

        let mut all = registry.all_instance_certs();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, "svc-1");
        assert_eq!(all[1].0, "svc-2");
    }
}
