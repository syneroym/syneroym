//! Stable storage abstraction and persistence backend
//!
//! Defines the `EndpointStorage` trait and implements `SQLite` persistence
//! and thread-safe in-memory mock storage for the local `EndpointRegistry`.

use std::{fmt::Debug, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::local_registry::SubstrateEndpoint;

/// (`service_type`, `health_check_json`, `manifest_hash`, `visibility`) --
/// what a deploy recorded about a service. `visibility` is `None` only for a
/// service whose row predates the column (ADR-0018).
pub type DeployFacts = (String, Option<String>, Option<String>, Option<String>);

/// Who manages an app instance on this substrate (ADR-0021 §4). The
/// generation is a **tiebreaker among already-authorized writers**, not
/// an authorization mechanism: a party without `orchestrator/deploy` is
/// refused regardless of what generation it presents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInstanceManagement {
    /// First-write-wins, unchanged from A2's `app_instance_owners`.
    pub owner_did: String,
    /// The supervisor that most recently wrote at `generation`. `None`
    /// until an operator's `adopt` names one -- an unadopted instance is
    /// writable by any authorized caller, which is what keeps A0-A4's
    /// hand-deploy path working after this lands, and what
    /// `release-app-instance` returns it to.
    pub supervisor_did: Option<String>,
    /// Minted by the operator's `adopt`, never self-incremented.
    pub generation: u64,
}

/// A trait abstracting stable storage for the `EndpointRegistry`.
#[async_trait]
pub trait EndpointStorage: Send + Sync {
    /// Load all endpoints from stable storage. Returns a vector of
    /// (`service_id`, `interface_name`, endpoint).
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>>;

    /// Save an endpoint into stable storage.
    async fn save(
        &self,
        service_id: &str,
        interface_name: &str,
        endpoint: &SubstrateEndpoint,
    ) -> Result<()>;

    /// Remove an endpoint from stable storage.
    async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()>;

    /// Load every recorded service owner as (`service_id`, `owner_did`)
    /// (M04A Slice B7a).
    async fn load_all_owners(&self) -> Result<Vec<(String, String)>>;
    /// Record `owner_did` as the owner of `service_id` (upsert).
    async fn save_owner(&self, service_id: &str, owner_did: &str) -> Result<()>;
    /// Forget `service_id`'s owner. Idempotent.
    async fn remove_owner(&self, service_id: &str) -> Result<()>;

    /// Load every recorded instance certificate as (`service_id`, certificate
    /// JSON).
    async fn load_all_certs(&self) -> Result<Vec<(String, String)>>;
    /// Record `certificate_json` as the installed instance certificate for
    /// `service_id` (upsert -- a renewal replaces in place).
    async fn save_cert(&self, service_id: &str, certificate_json: &str) -> Result<()>;
    /// Forget `service_id`'s instance certificate. Idempotent.
    async fn remove_cert(&self, service_id: &str) -> Result<()>;

    /// Every stored deploy fact, as (`service_id`, `service_type`,
    /// `health_check_json`, `manifest_hash`, `visibility`) (M05A A4,
    /// `manifest_hash` added A5a for deploy idempotency, `visibility`
    /// added ADR-0018).
    async fn load_all_deploy_facts(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>>;
    /// Record what a deploy said `service_id` is, its declared health
    /// check if any, the canonical content hash of what was actually
    /// installed, and its declared visibility (upsert -- a redeploy that drops
    /// the check writes `None`, clearing it by construction; `manifest_hash`
    /// is written only on full deploy success, so a half-failed deploy is never
    /// deduplicated on the next attempt).
    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
        manifest_hash: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<()>;
    /// Forget `service_id`'s deploy facts. Idempotent.
    async fn remove_deploy_facts(&self, service_id: &str) -> Result<()>;

    /// Load every recorded app context as (`service_id`, `app_instance_id`,
    /// `service_name`) (A2).
    async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>>;
    /// Record which app instance and logical name `service_id` was deployed
    /// as (upsert).
    async fn save_app_context(
        &self,
        service_id: &str,
        app_instance_id: &str,
        service_name: &str,
    ) -> Result<()>;
    /// Forget `service_id`'s app context and every binding row it wrote.
    /// Idempotent.
    async fn remove_app_context(&self, service_id: &str) -> Result<()>;

    /// Load every recorded dependency binding as (`service_id`,
    /// `app_instance_id`, `dependency_name`, `topology_entry_json`).
    async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>>;
    /// Record one dependency binding for `service_id` (upsert on
    /// (`service_id`, `dependency_name`)). There is no `remove_binding`:
    /// `remove_app_context` covers binding removal too, since a redeploy
    /// overwrites the rows it still declares and `deploy` clears the
    /// service's rows first.
    async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        topology_entry_json: &str,
    ) -> Result<()>;
    /// One persisted binding's `entry_json` (M05A A5a). The epoch guard
    /// compares against exactly one row, so `load_all_bindings`' full
    /// scan (which exists for the startup replay) is the wrong shape for
    /// it.
    async fn load_binding(&self, service_id: &str, dependency_name: &str)
    -> Result<Option<String>>;
    /// Every persisted binding for one service, as (`dependency_name`,
    /// `entry_json`), for `status`'s per-dependent convergence report
    /// (M05A A5a).
    async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>>;

    /// Load every recorded app-instance management stamp as
    /// (`app_instance_id`, `AppInstanceManagement`) (M05A A5a, replacing
    /// A2's `app_instance_owners`).
    async fn load_all_app_instance_management(
        &self,
    ) -> Result<Vec<(String, AppInstanceManagement)>>;
    /// Record `management` for `app_instance_id` (upsert). The takeover /
    /// generation-tiebreak logic is the caller's responsibility
    /// (`ControlPlaneService::check_generation`), not this store's, the
    /// same split `save_owner` already uses for `service_id` ownership.
    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> Result<()>;
    /// Forget `app_instance_id`'s management stamp. Idempotent. Called
    /// when the last service of an instance is undeployed -- the standing
    /// backlog row `app_instance_owners` rows never get forgotten.
    async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()>;
}

/// A thread-safe in-memory storage for testing.
#[derive(Debug)]
pub struct MockStorage {
    data: Arc<DashMap<(String, String), SubstrateEndpoint>>,
    owners: Arc<DashMap<String, String>>,
    certs: Arc<DashMap<String, String>>,
    deploy_facts: Arc<DashMap<String, DeployFacts>>,
    app_contexts: Arc<DashMap<String, (String, String)>>,
    bindings: Arc<DashMap<(String, String), (String, String)>>,
    app_instance_management: Arc<DashMap<String, AppInstanceManagement>>,
}

impl Default for MockStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStorage {
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            owners: Arc::new(DashMap::new()),
            certs: Arc::new(DashMap::new()),
            deploy_facts: Arc::new(DashMap::new()),
            app_contexts: Arc::new(DashMap::new()),
            bindings: Arc::new(DashMap::new()),
            app_instance_management: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl EndpointStorage for MockStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        Ok(self
            .data
            .iter()
            .map(|e| (e.key().0.clone(), e.key().1.clone(), e.value().clone()))
            .collect())
    }
    async fn save(&self, sid: &str, iname: &str, ep: &SubstrateEndpoint) -> Result<()> {
        self.data.insert((sid.to_string(), iname.to_string()), ep.clone());
        Ok(())
    }
    async fn remove(&self, sid: &str, iname: &str) -> Result<()> {
        self.data.remove(&(sid.to_string(), iname.to_string()));
        Ok(())
    }
    async fn load_all_owners(&self) -> Result<Vec<(String, String)>> {
        Ok(self.owners.iter().map(|e| (e.key().clone(), e.value().clone())).collect())
    }
    async fn save_owner(&self, service_id: &str, owner_did: &str) -> Result<()> {
        self.owners.insert(service_id.to_string(), owner_did.to_string());
        Ok(())
    }
    async fn remove_owner(&self, service_id: &str) -> Result<()> {
        self.owners.remove(service_id);
        Ok(())
    }
    async fn load_all_certs(&self) -> Result<Vec<(String, String)>> {
        Ok(self.certs.iter().map(|e| (e.key().clone(), e.value().clone())).collect())
    }
    async fn save_cert(&self, service_id: &str, certificate_json: &str) -> Result<()> {
        self.certs.insert(service_id.to_string(), certificate_json.to_string());
        Ok(())
    }
    async fn remove_cert(&self, service_id: &str) -> Result<()> {
        self.certs.remove(service_id);
        Ok(())
    }
    async fn load_all_deploy_facts(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        Ok(self
            .deploy_facts
            .iter()
            .map(|e| {
                (
                    e.key().clone(),
                    e.value().0.clone(),
                    e.value().1.clone(),
                    e.value().2.clone(),
                    e.value().3.clone(),
                )
            })
            .collect())
    }
    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
        manifest_hash: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<()> {
        self.deploy_facts.insert(
            service_id.to_string(),
            (
                service_type.to_string(),
                health_check_json.map(str::to_string),
                manifest_hash.map(str::to_string),
                visibility.map(str::to_string),
            ),
        );
        Ok(())
    }
    async fn remove_deploy_facts(&self, service_id: &str) -> Result<()> {
        self.deploy_facts.remove(service_id);
        Ok(())
    }
    async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>> {
        Ok(self
            .app_contexts
            .iter()
            .map(|e| (e.key().clone(), e.value().0.clone(), e.value().1.clone()))
            .collect())
    }
    async fn save_app_context(
        &self,
        service_id: &str,
        app_instance_id: &str,
        service_name: &str,
    ) -> Result<()> {
        self.app_contexts.insert(
            service_id.to_string(),
            (app_instance_id.to_string(), service_name.to_string()),
        );
        Ok(())
    }
    async fn remove_app_context(&self, service_id: &str) -> Result<()> {
        self.app_contexts.remove(service_id);
        self.bindings.retain(|(sid, _), _| sid != service_id);
        Ok(())
    }
    async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>> {
        Ok(self
            .bindings
            .iter()
            .map(|e| {
                (e.key().0.clone(), e.value().0.clone(), e.key().1.clone(), e.value().1.clone())
            })
            .collect())
    }
    async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        topology_entry_json: &str,
    ) -> Result<()> {
        self.bindings.insert(
            (service_id.to_string(), dependency_name.to_string()),
            (app_instance_id.to_string(), topology_entry_json.to_string()),
        );
        Ok(())
    }
    async fn load_binding(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .bindings
            .get(&(service_id.to_string(), dependency_name.to_string()))
            .map(|e| e.value().1.clone()))
    }
    async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>> {
        Ok(self
            .bindings
            .iter()
            .filter(|e| e.key().0 == service_id)
            .map(|e| (e.key().1.clone(), e.value().1.clone()))
            .collect())
    }
    async fn load_all_app_instance_management(
        &self,
    ) -> Result<Vec<(String, AppInstanceManagement)>> {
        Ok(self
            .app_instance_management
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect())
    }
    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> Result<()> {
        self.app_instance_management.insert(app_instance_id.to_string(), management.clone());
        Ok(())
    }
    async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()> {
        self.app_instance_management.remove(app_instance_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SubstrateEndpoint ↔ storage string mapping (single source of truth)
// ---------------------------------------------------------------------------

impl SubstrateEndpoint {
    /// Returns the discriminant key used in stable storage.
    #[must_use]
    pub const fn storage_key(&self) -> &'static str {
        match self {
            Self::WasmChannel { .. } => "wasm",
            Self::PodmanSocket { .. } => "podman",
            Self::NativeHostChannel { .. } => "native",
            Self::TcpHostPort { .. } => "tcp",
        }
    }

    /// Returns the data payload stored alongside the key.
    pub fn storage_data(&self) -> String {
        match self {
            Self::WasmChannel { service_id } => service_id.clone(),
            Self::PodmanSocket { socket_path } => socket_path.clone(),
            Self::NativeHostChannel { service_id } => service_id.clone(),
            Self::TcpHostPort { host, port } => format!("{host}:{port}"),
        }
    }
}

impl TryFrom<(&str, String)> for SubstrateEndpoint {
    type Error = anyhow::Error;

    fn try_from((key, data): (&str, String)) -> Result<Self> {
        match key {
            "wasm" => Ok(Self::WasmChannel { service_id: data }),
            "podman" => Ok(Self::PodmanSocket { socket_path: data }),
            "native" => Ok(Self::NativeHostChannel { service_id: data }),
            "tcp" => {
                let (host, port_str) = data
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("Invalid TCP endpoint data: {data}"))?;
                let port = port_str.parse().map_err(|e| {
                    anyhow::anyhow!("Invalid port in TCP endpoint data: {data} ({e})")
                })?;
                Ok(Self::TcpHostPort { host: host.to_string(), port })
            }
            other => Err(anyhow::anyhow!("Unknown endpoint type in storage: {other}")),
        }
    }
}
