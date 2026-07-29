//! Community Service Registry
//!
//! A public/shared registry server allowing nodes to register their network
//! addresses and nicknames, enabling global peer lookup.

use std::{
    fmt::{self, Debug, Formatter},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use dashmap::{DashMap, Entry};
use oneshot::Sender;
use reqwest::Client;
use syneroym_core::{
    config::SubstrateConfig,
    dht_registry::{DEFAULT_REGISTRY_TTL_SECS, SignedEndpointInfo, SignedMasterAnchor},
    util,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time};
use tracing::{debug, error, info, warn};

pub struct EcosystemRegistry {
    bind_address: String,
    state: Arc<RegistryState>,
    shutdown_tx: Option<Sender<()>>,
    server_handle: Option<JoinHandle<()>>,
    listener: Option<TcpListener>,
}

impl Debug for EcosystemRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("EcosystemRegistry")
            .field("bind_address", &self.bind_address)
            .field("state", &self.state)
            .field("shutdown_tx", &self.shutdown_tx.as_ref().map(|_| "oneshot::Sender"))
            .field("server_handle", &self.server_handle)
            .field("listener", &self.listener.as_ref().map(|l| l.local_addr().ok()))
            .finish()
    }
}

#[derive(Debug)]
struct RegistryState {
    // Map of service_id -> (SignedEndpointInfo, admitted-at, pkarr/BEP44
    // timestamp of the admitted record -- the compare-and-swap key, kept
    // alongside rather than re-derived by re-verifying on every write).
    endpoints: DashMap<String, (SignedEndpointInfo, Instant, u64)>,
    // Map of alias -> service_id
    aliases: DashMap<String, String>,
    // Map of master_id -> (SignedMasterAnchor, std::time::Instant)
    master_anchors: DashMap<String, (SignedMasterAnchor, Instant)>,
    // Needed when registry is not accessible from internal network and multi-hop-relays are needed
    // for data transfer
    parent_registry_url: Option<String>,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            endpoints: DashMap::new(),
            aliases: DashMap::new(),
            master_anchors: DashMap::new(),
            parent_registry_url: None,
        }
    }
}

impl EcosystemRegistry {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing service registry");

        let bind_address = config
            .roles
            .community_registry
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!("community registry role must be enabled to initialize registry")
            })?
            .http_bind_address
            .clone();

        let parent_registry_url =
            config.roles.community_registry.as_ref().and_then(|r| r.parent_registry_url.clone());

        Ok(Self {
            bind_address,
            state: Arc::new(RegistryState {
                endpoints: DashMap::new(),
                aliases: DashMap::new(),
                master_anchors: DashMap::new(),
                parent_registry_url,
            }),
            shutdown_tx: None,
            server_handle: None,
            listener: None,
        })
    }

    pub async fn bind(&mut self) -> Result<String> {
        if self.listener.is_none() {
            let listener = TcpListener::bind(&self.bind_address)
                .await
                .context("Failed to bind registry listener")?;
            let bound_address = listener.local_addr()?;
            self.bind_address = format!("127.0.0.1:{}", bound_address.port());
            self.listener = Some(listener);
        }
        Ok(format!("http://{}", self.bind_address))
    }

    pub async fn spawn(&mut self) -> Result<()> {
        let listener = match self.listener.take() {
            Some(l) => l,
            None => TcpListener::bind(&self.bind_address)
                .await
                .context("Failed to bind registry listener")?,
        };

        let bound_address = listener.local_addr()?;
        self.bind_address = format!("127.0.0.1:{}", bound_address.port());
        let addr_str = format!("http://{}", self.bind_address);

        info!("running service registry on {}", addr_str);

        let app = Router::new()
            .route("/register", post(register_endpoint))
            .route("/lookup/{service_id}", get(lookup_endpoint))
            .route("/register_master", post(register_master_endpoint))
            .route("/lookup_master/{master_id}", get(lookup_master_endpoint))
            .with_state(self.state.clone());

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        let state_clone = self.state.clone();
        tokio::spawn(async move {
            let default_ttl = Duration::from_secs(DEFAULT_REGISTRY_TTL_SECS);
            loop {
                time::sleep(Duration::from_secs(15 * 60)).await; // 15 mins
                let mut expired_keys = Vec::new();
                for entry in state_clone.endpoints.iter() {
                    let ttl =
                        entry.value().0.info.ttl.map(Duration::from_secs).unwrap_or(default_ttl);
                    if entry.value().1.elapsed() > ttl {
                        expired_keys.push(entry.key().clone());
                    }
                }
                // (`entry.value().2`, the CAS timestamp, is not read by the
                // sweep -- it only ever gates admission, on the write path.)
                for key in expired_keys {
                    state_clone.endpoints.remove(&key);
                    state_clone.aliases.retain(|_, v| *v != key);
                    debug!("Expired registry entry for {}", key);
                }
            }
        });

        let server_handle = tokio::spawn(async move {
            let server = axum::serve(listener, app);
            let graceful = server.with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(e) = graceful.await {
                error!("Registry server error: {}", e);
            }
        });
        self.server_handle = Some(server_handle);

        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        self.spawn().await?;
        if let Some(ref mut handle) = self.server_handle {
            let _ = handle.await;
        }
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down service registry");
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server_handle.take() {
            let _ = handle.await;
        }
        Ok(())
    }
}

async fn register_endpoint(
    State(state): State<Arc<RegistryState>>,
    Json(payload): Json<SignedEndpointInfo>,
) -> Result<StatusCode, (StatusCode, String)> {
    let service_id = payload.info.service_id.clone();

    let timestamp = verify_endpoint_signature(&payload)?;

    let alias = util::generate_alias(payload.info.nickname.as_deref(), &service_id);

    if let Some(existing_id) = state.aliases.get(&alias)
        && *existing_id != service_id
    {
        return Err((
            StatusCode::CONFLICT,
            "Alias collision: this nickname-shorthash is already in use by a different service"
                .to_string(),
        ));
    }

    admit_endpoint(&state.endpoints, service_id.clone(), payload.clone(), timestamp)?;

    // Remove any previous aliases associated with this service_id
    state.aliases.retain(|_, id| *id != service_id);
    state.aliases.insert(alias, service_id);

    if let Some(parent_url) = &state.parent_registry_url
        && !payload.info.is_private
    {
        propagate_registration(payload, parent_url.clone());
    }

    Ok(StatusCode::OK)
}

fn verify_endpoint_signature(payload: &SignedEndpointInfo) -> Result<u64, (StatusCode, String)> {
    payload
        .verify()
        .map(|ts| ts.as_u64())
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Signature verification failed: {e}")))
}

/// Admits `payload` under `service_id`, last-writer-wins by pkarr/BEP44
/// timestamp rather than by arrival order -- the same rule
/// `mainline`'s own server enforces for the DHT leg, so a rollback that the
/// DHT would refuse cannot land here just because the registry answers
/// lookups first (`RegistryClient::lookup` tries HTTP before falling back).
/// A record that has moved to another substrate carries a strictly newer
/// timestamp (a fresh `EndpointInfo::sign`), so the old host cannot
/// resurrect its stale mapping by continuing to heartbeat it here.
///
/// Equal timestamp, byte-identical bytes is accepted and treated as a
/// refresh (resets the TTL clock) rather than a conflict: a substrate that
/// cannot re-sign a master-signed record replays the exact same blob on
/// every heartbeat, and that replay is what keeps the record from expiring
/// on this registry's TTL sweep. Equal timestamp with *different* bytes --
/// two distinct records claiming the same instant -- is rejected exactly
/// like an older one; it cannot be resolved by preferring one arbitrarily.
fn admit_endpoint(
    endpoints: &DashMap<String, (SignedEndpointInfo, Instant, u64)>,
    service_id: String,
    payload: SignedEndpointInfo,
    timestamp: u64,
) -> Result<(), (StatusCode, String)> {
    match endpoints.entry(service_id) {
        Entry::Occupied(mut e) => {
            let (stored_payload, _, stored_timestamp) = e.get();
            if timestamp < *stored_timestamp
                || (timestamp == *stored_timestamp
                    && stored_payload.pkarr_packet_hex != payload.pkarr_packet_hex)
            {
                return Err((
                    StatusCode::CONFLICT,
                    "a newer or equally-recent endpoint record is already registered for this \
                     service_id"
                        .to_string(),
                ));
            }
            e.insert((payload, Instant::now(), timestamp));
        }
        Entry::Vacant(e) => {
            e.insert((payload, Instant::now(), timestamp));
        }
    }
    Ok(())
}

fn propagate_registration(payload: SignedEndpointInfo, parent_url: String) {
    tokio::spawn(async move {
        let client = Client::new();
        let url = format!("{parent_url}/register");
        debug!("Propagating registration to parent registry at: {}", url);
        match client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!("Successfully propagated registration to {}", url);
            }
            Ok(resp) => {
                warn!("Failed to propagate registration to {} (status: {})", url, resp.status());
            }
            Err(e) => {
                warn!("Error propagating registration to {}: {}", url, e);
            }
        }
    });
}

async fn lookup_endpoint(
    Path(service_id): Path<String>,
    State(state): State<Arc<RegistryState>>,
) -> Result<Json<SignedEndpointInfo>, StatusCode> {
    let resolved_id = state.aliases.get(&service_id).map(|e| e.clone()).unwrap_or(service_id);
    let entry = state.endpoints.get(&resolved_id).map(|e| e.0.clone());

    if let Some(entry) = entry { Ok(Json(entry)) } else { Err(StatusCode::NOT_FOUND) }
}

async fn register_master_endpoint(
    State(state): State<Arc<RegistryState>>,
    Json(payload): Json<SignedMasterAnchor>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Err(e) = payload.verify() {
        return Err((StatusCode::UNAUTHORIZED, format!("Signature verification failed: {}", e)));
    }

    // Same last-writer-wins discipline as `admit_endpoint`, applied to the
    // anchor for consistency: `MasterAnchorPayload.timestamp` is already
    // authenticated as equal to the packet's own signed timestamp by
    // `verify()`'s whole-payload check, so it doubles as the CAS key with
    // no extra field needed.
    match state.master_anchors.entry(payload.master_id.clone()) {
        Entry::Occupied(mut e) => {
            let stored_timestamp = e.get().0.payload.timestamp;
            if payload.payload.timestamp < stored_timestamp
                || (payload.payload.timestamp == stored_timestamp
                    && e.get().0.pkarr_packet_hex != payload.pkarr_packet_hex)
            {
                return Err((
                    StatusCode::CONFLICT,
                    "a newer or equally-recent master anchor is already registered for this \
                     master_id"
                        .to_string(),
                ));
            }
            e.insert((payload, Instant::now()));
        }
        Entry::Vacant(e) => {
            e.insert((payload, Instant::now()));
        }
    }
    Ok(StatusCode::OK)
}

async fn lookup_master_endpoint(
    Path(master_id): Path<String>,
    State(state): State<Arc<RegistryState>>,
) -> Result<Json<SignedMasterAnchor>, StatusCode> {
    let entry = state.master_anchors.get(&master_id).map(|e| e.0.clone());
    if let Some(entry) = entry { Ok(Json(entry)) } else { Err(StatusCode::NOT_FOUND) }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::http::StatusCode;
    use syneroym_core::{
        config::{AccessControl, ServiceRegistryRole, SubstrateConfig},
        dht_registry::{
            EndpointInfo, EndpointMechanism, EndpointType, MASTER_ANCHOR_SCHEMA_V1,
            MasterAnchorPayload, RegistryClient,
        },
        endpoint_publisher::EndpointPublisher,
        util,
    };
    use syneroym_identity::{Identity, substrate};

    use super::*;

    fn create_signed_info(identity: &Identity, info: EndpointInfo) -> SignedEndpointInfo {
        info.sign(identity).unwrap()
    }

    fn far_future() -> u64 {
        u64::MAX / 2
    }

    fn sample_service_info_for(service_id: &str) -> EndpointInfo {
        EndpointInfo {
            service_id: service_id.to_string(),
            substrate_id: "did:key:zSubstrate".to_string(),
            endpoint_type: EndpointType::Service,
            nickname: None,
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            not_after: far_future(),
        }
    }

    async fn spawn_registry() -> (EcosystemRegistry, String) {
        let config = SubstrateConfig {
            roles: syneroym_core::config::RolesConfig {
                community_registry: Some(ServiceRegistryRole {
                    access: AccessControl::String("everyone".to_string()),
                    http_bind_address: "127.0.0.1:0".to_string(),
                    parent_registry_url: None,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut registry = EcosystemRegistry::init(&config).await.unwrap();
        let url = registry.bind().await.unwrap();
        registry.spawn().await.unwrap();
        (registry, url)
    }

    #[tokio::test]
    async fn test_master_anchor_register_and_lookup() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let master_id = substrate::derive_did_key(&identity.public_key());

        let _temp_identity = Identity::generate().unwrap();

        let payload = MasterAnchorPayload {
            revoked_keys: vec!["did:key:revoked".to_string()],
            timestamp: 1690000000,
            ..Default::default()
        };

        let signed_anchor = payload.sign(&identity).unwrap();

        // Register
        let reg_res =
            register_master_endpoint(State(state.clone()), Json(signed_anchor.clone())).await;
        assert!(reg_res.is_ok());

        // Lookup
        let lookup_res = lookup_master_endpoint(Path(master_id.clone()), State(state)).await;
        assert!(lookup_res.is_ok());
        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.master_id, master_id);
        assert_eq!(retrieved.payload.schema, MASTER_ANCHOR_SCHEMA_V1);
        assert_eq!(retrieved.payload.revoked_keys.len(), 1);
    }

    #[tokio::test]
    async fn test_register_and_lookup_success() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: Some("alice".to_string()),
            mechanisms: vec![EndpointMechanism::Iroh {
                endpoint_addr_bytes: vec![1, 2, 3],
                relay_url: Some("http://relay.example.com".to_string()),
            }],
            is_private: false,
            ttl: None,
            not_after: far_future(),
        };

        let signed_info = create_signed_info(&identity, info);

        // Register
        let res = register_endpoint(State(state.clone()), Json(signed_info.clone())).await;
        assert_eq!(res.unwrap(), StatusCode::OK);

        // Lookup by alias
        let service_hash = util::short_hash(&did);
        let alias = format!("alice-p{service_hash}");
        let lookup_res = lookup_endpoint(Path(alias), State(state)).await;

        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, signed_info.info.service_id);
    }

    #[tokio::test]
    async fn test_register_invalid_signature() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let other_identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: None,
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            not_after: far_future(),
        };

        // Sign with OTHER identity
        let signed_info = create_signed_info(&other_identity, info);

        let res = register_endpoint(State(state), Json(signed_info)).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_register_invalid_did() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();

        let info = EndpointInfo {
            service_id: "invalid-did".to_string(),
            substrate_id: "invalid-did".to_string(),
            endpoint_type: EndpointType::Substrate,
            nickname: None,
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            not_after: far_future(),
        };

        let signed_info = create_signed_info(&identity, info);

        let res = register_endpoint(State(state), Json(signed_info)).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_indirect_lookup() {
        let state = Arc::new(RegistryState::default());
        let substrate_id = "did:key:hsubstrate";
        let service_id = "did:key:hservice";

        // Mock a substrate record
        let substrate_info = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: substrate_id.to_string(),
                substrate_id: substrate_id.to_string(),
                endpoint_type: EndpointType::Substrate,
                nickname: None,
                mechanisms: vec![EndpointMechanism::Iroh {
                    endpoint_addr_bytes: vec![42],
                    relay_url: None,
                }],
                is_private: false,
                ttl: None,
                not_after: far_future(),
            },
            pkarr_packet_hex: "mock-hex".to_string(),
        };
        state
            .endpoints
            .insert(substrate_id.to_string(), (substrate_info.clone(), Instant::now(), 0));

        // Mock a service record pointing to that substrate
        let service_info = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: service_id.to_string(),
                substrate_id: substrate_id.to_string(),
                endpoint_type: EndpointType::Service,
                nickname: None,
                mechanisms: vec![],
                is_private: false,
                ttl: None,
                not_after: far_future(),
            },
            pkarr_packet_hex: "mock-hex".to_string(),
        };
        state.endpoints.insert(service_id.to_string(), (service_info, Instant::now(), 0));

        // Lookup service
        let lookup_res = lookup_endpoint(Path(service_id.to_string()), State(state.clone())).await;

        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, service_id);
        // Ensure mechanisms are NOT populated since we removed server-side resolution
        assert!(retrieved.info.mechanisms.is_empty());
    }

    #[tokio::test]
    async fn test_lookup_by_shorthash_no_nickname() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: None, // No nickname
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            not_after: far_future(),
        };

        let signed_info = create_signed_info(&identity, info);
        register_endpoint(State(state.clone()), Json(signed_info)).await.unwrap();

        // Lookup by shorthash (p{hash}) should work
        let service_hash = util::short_hash(&did);
        let alias = format!("p{service_hash}");
        let lookup_res = lookup_endpoint(Path(alias), State(state)).await;

        assert!(lookup_res.is_ok());
        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, did);
    }

    #[tokio::test]
    async fn test_lookup_by_shorthash_fails_if_nickname_present() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: Some("alice".to_string()),
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            not_after: far_future(),
        };

        let signed_info = create_signed_info(&identity, info);
        register_endpoint(State(state.clone()), Json(signed_info)).await.unwrap();

        // Lookup by shorthash-only (p{hash}) should FAIL because nickname was provided
        let service_hash = util::short_hash(&did);
        let alias = format!("p{service_hash}");
        let lookup_res = lookup_endpoint(Path(alias), State(state)).await;

        assert!(lookup_res.is_err());
        assert_eq!(lookup_res.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_lookup_not_found() {
        let state = Arc::new(RegistryState::default());
        let res = lookup_endpoint(Path("non-existent".to_string()), State(state)).await;

        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_master_signed_endpoint_record_registers_and_looks_up_under_its_own_did() {
        // The one keying shape this design has: a member's endpoint record
        // is signed by the deployer's own member master key,
        // self-consistently, exactly like any other self-signed record --
        // there is no longer a separate "signed by a delegated instance
        // key, keyed by a different master DID" shape to exercise.
        let state = Arc::new(RegistryState::default());
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let signed = sample_service_info_for(&master_did).sign(&master).unwrap();

        let res = register_endpoint(State(state.clone()), Json(signed.clone())).await;
        assert_eq!(res.unwrap(), StatusCode::OK);

        let lookup_res = lookup_endpoint(Path(master_did.clone()), State(state)).await;
        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, master_did);
    }

    #[tokio::test]
    async fn a_masters_records_alias_is_derived_from_its_own_did() {
        let state = Arc::new(RegistryState::default());
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let mut info = sample_service_info_for(&master_did);
        info.nickname = Some("member-one".to_string());
        let signed = info.sign(&master).unwrap();

        register_endpoint(State(state.clone()), Json(signed)).await.unwrap();

        let master_hash = util::short_hash(&master_did);
        let alias = format!("member-one-p{master_hash}");
        let lookup_res = lookup_endpoint(Path(alias), State(state)).await;
        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, master_did);
    }

    #[tokio::test]
    async fn refreshing_a_master_anchor_keeps_its_revocations_and_its_revoke_list_registry() {
        let (_registry, url) = spawn_registry().await;
        let client = RegistryClient::new(false, Some(url));
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        client
            .publish_master_anchor(
                &master_did,
                vec!["did:key:zRevoked".to_string()],
                Some("https://revocations.example/list".to_string()),
                &master,
                true,
            )
            .await
            .unwrap();

        client.refresh_master_anchor(&master).await.unwrap();

        let refreshed = client.resolve_master_anchor(&master_did, None).await.unwrap();
        assert_eq!(refreshed.revoked_keys, vec!["did:key:zRevoked".to_string()]);
        assert_eq!(
            refreshed.revoke_list_registry,
            Some("https://revocations.example/list".to_string())
        );
    }

    #[tokio::test]
    async fn refreshing_a_stale_master_anchor_keeps_its_revocations() {
        let (registry, url) = spawn_registry().await;
        let client = RegistryClient::new(false, Some(url));
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        // Seed the registry directly with a backdated, but genuinely signed,
        // anchor -- the common case a late operator hits, per D-A1-12.
        let stale_payload = MasterAnchorPayload {
            revoked_keys: vec!["did:key:zRevoked".to_string()],
            ..Default::default()
        };
        let stale_signed = sign_backdated(stale_payload, &master, 25);
        registry.state.master_anchors.insert(master_did.clone(), (stale_signed, Instant::now()));

        client.refresh_master_anchor(&master).await.unwrap();

        let refreshed = client.resolve_master_anchor(&master_did, None).await.unwrap();
        assert_eq!(refreshed.revoked_keys, vec!["did:key:zRevoked".to_string()]);
    }

    #[tokio::test]
    async fn refreshing_refuses_to_overwrite_an_anchor_it_cannot_read() {
        let (registry, url) = spawn_registry().await;
        let client = RegistryClient::new(false, Some(url));
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        // A corrupted anchor: signed by a different master than the one it
        // claims (`master_id` mismatches the signing key), so it can never
        // pass `verify_signature`.
        let other = Identity::generate().unwrap();
        let mut corrupt = MasterAnchorPayload::default().sign(&other).unwrap();
        corrupt.master_id = master_did.clone();
        registry.state.master_anchors.insert(master_did.clone(), (corrupt.clone(), Instant::now()));

        let err = client.refresh_master_anchor(&master).await;
        assert!(err.is_err());

        let stored = registry.state.master_anchors.get(&master_did).unwrap();
        assert_eq!(stored.0.pkarr_packet_hex, corrupt.pkarr_packet_hex);
    }

    /// D-A1-12's `master_id` equality tightening, against
    /// `fetch_own_master_anchor` (reached through `refresh_master_anchor`).
    /// Unlike `refreshing_refuses_to_overwrite_an_anchor_it_cannot_read`'s
    /// corrupted anchor, this one has a perfectly valid signature -- it is
    /// honestly signed by `other`, and only the identity it is served under
    /// is wrong, the shape a compromised or buggy registry would produce by
    /// answering a lookup for one master with another's anchor.
    #[tokio::test]
    async fn refresh_refuses_an_anchor_served_under_the_wrong_master() {
        let (registry, url) = spawn_registry().await;
        let client = RegistryClient::new(false, Some(url));
        let requested_master = Identity::generate().unwrap();
        let requested_master_did = substrate::derive_did_key(&requested_master.public_key());

        let other_master = Identity::generate().unwrap();
        let other_anchor = MasterAnchorPayload::default().sign(&other_master).unwrap();
        registry
            .state
            .master_anchors
            .insert(requested_master_did.clone(), (other_anchor, Instant::now()));

        let err = client.refresh_master_anchor(&requested_master).await;
        assert!(err.is_err(), "a validly-signed anchor for a different master must be refused");
    }

    /// The same D-A1-12 tightening, against `resolve_master_anchor` -- the
    /// consumer-facing read path, not the refresh path above.
    #[tokio::test]
    async fn resolve_master_anchor_refuses_an_anchor_served_under_the_wrong_master() {
        let (registry, url) = spawn_registry().await;
        let client = RegistryClient::new(false, Some(url));
        let requested_master = Identity::generate().unwrap();
        let requested_master_did = substrate::derive_did_key(&requested_master.public_key());

        let other_master = Identity::generate().unwrap();
        let other_anchor = MasterAnchorPayload::default().sign(&other_master).unwrap();
        registry
            .state
            .master_anchors
            .insert(requested_master_did.clone(), (other_anchor, Instant::now()));

        let err = client.resolve_master_anchor(&requested_master_did, None).await;
        assert!(err.is_err(), "a validly-signed anchor for a different master must be refused");
    }

    /// D-A1-3's recovery path, against a live registry -- `build_record`'s
    /// own tests (`crates/core/src/endpoint_publisher.rs`) exercise the
    /// decision table but never call `publish_all_services` itself, since
    /// `register` against no registry is a silent no-op rather than the
    /// failure this test needs.
    ///
    /// The failure exercised here is D-A1-14's compare-and-swap: a stored
    /// record whose `service_id` a strictly newer record already occupies
    /// at the live registry (as if a relocation already published one
    /// elsewhere) is a genuine `Err` from `publish_service`, not the benign
    /// `Ok(false)` a verification failure would produce. The sweep must
    /// survive it and still publish the other stored record.
    ///
    /// **The rejected record's filename is load-bearing and must keep
    /// sorting first.** The sweep walks a `BTreeSet`, so ids run in
    /// ascending byte order; naming it to sort *last* would let every
    /// assertion below hold even under an implementation that aborted on
    /// the first error. `aaa-` (0x61) precedes `did:key:` (0x64), so the
    /// rejection happens before the other record is reached. The filename
    /// is independent of the record's own `info.service_id` --
    /// `build_record` looks the file up by the sweep's id, not by what is
    /// inside it -- so naming the file for sort order does not change what
    /// gets registered.
    #[tokio::test]
    async fn publish_all_services_survives_a_record_rejected_by_admission() {
        let (_registry, url) = spawn_registry().await;
        let hosted_apps_dir = tempfile::tempdir().unwrap();
        let client = RegistryClient::new(false, Some(url.clone()));

        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let stale = sample_service_info_for(&did).sign(&identity).unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let fresh = sample_service_info_for(&did).sign(&identity).unwrap();

        // The fresher record lands first, as if a relocation had already
        // published it elsewhere.
        client.register(&fresh, false).await.unwrap();

        fs::write(
            hosted_apps_dir.path().join("aaa-conflicting.json"),
            serde_json::to_string(&stale).unwrap(),
        )
        .unwrap();

        // A second, unrelated service with a valid stored record -- sorts
        // after the conflicting one, so reaching it proves the sweep
        // continued rather than aborting.
        let other_identity = Identity::generate().unwrap();
        let other_did = substrate::derive_did_key(&other_identity.public_key());
        let other_signed = sample_service_info_for(&other_did).sign(&other_identity).unwrap();
        fs::write(
            hosted_apps_dir.path().join(format!("{other_did}.json")),
            serde_json::to_string(&other_signed).unwrap(),
        )
        .unwrap();

        let publisher = EndpointPublisher::new(
            Arc::new(RegistryClient::new(false, Some(url.clone()))),
            hosted_apps_dir.path().to_path_buf(),
        );

        publisher.publish_all_services().await;

        let looked_up = client.lookup(&did, false).await.unwrap();
        assert_eq!(
            looked_up.pkarr_packet_hex, fresh.pkarr_packet_hex,
            "the stale record must not have overwritten the fresh one"
        );
        assert!(
            client.lookup(&other_did, false).await.is_ok(),
            "the other stored record must still have been published despite the conflict"
        );
    }

    /// Mirrors `MasterAnchorPayload::sign`, backdated -- kept in this test
    /// module too, since
    /// `refreshing_a_stale_master_anchor_keeps_its_revocations`
    /// needs to seed a registry directly rather than go through a
    /// `RegistryClient`.
    fn sign_backdated(
        mut payload: MasterAnchorPayload,
        identity: &Identity,
        hours_ago: u64,
    ) -> SignedMasterAnchor {
        use std::time::{SystemTime, UNIX_EPOCH};

        use pkarr::{
            Keypair, SignedPacket, Timestamp,
            dns::{CLASS, Name, ResourceRecord, rdata::RData},
        };
        use syneroym_core::dht_registry::{PKARR_DNS_NAME, PKARR_TTL};

        let master_id = substrate::derive_did_key(&identity.public_key());
        let keypair = Keypair::from_secret_key(&identity.to_bytes());

        let now_micros = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64;
        let backdated_micros = now_micros - hours_ago * 60 * 60 * 1_000_000;
        let timestamp = Timestamp::from(backdated_micros);
        payload.timestamp = timestamp.as_u64();

        let json_str = serde_json::to_string(&payload).unwrap();
        let txt_rdata = pkarr::dns::rdata::TXT::try_from(json_str.as_str()).unwrap();
        let name = Name::new(PKARR_DNS_NAME).unwrap();
        let records = vec![ResourceRecord::new(name, CLASS::IN, PKARR_TTL, RData::TXT(txt_rdata))];
        let signed_packet = SignedPacket::new(&keypair, &records, timestamp).unwrap();
        let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
        SignedMasterAnchor { master_id, payload, pkarr_packet_hex }
    }
}
