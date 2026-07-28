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
use dashmap::DashMap;
use oneshot::Sender;
use reqwest::Client;
use syneroym_core::{
    config::SubstrateConfig,
    dht_registry::{
        DEFAULT_REGISTRY_TTL_SECS, RecordTrust, SignedEndpointInfo, SignedMasterAnchor,
    },
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
    // Map of service_id -> (SignedEndpointInfo, std::time::Instant)
    endpoints: DashMap<String, (SignedEndpointInfo, Instant)>,
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
    let service_id = &payload.info.service_id;

    verify_endpoint_signature(&state, &payload)?;

    let alias = util::generate_alias(payload.info.nickname.as_deref(), service_id);

    if let Some(existing_id) = state.aliases.get(&alias)
        && *existing_id != *service_id
    {
        return Err((
            StatusCode::CONFLICT,
            "Alias collision: this nickname-shorthash is already in use by a different service"
                .to_string(),
        ));
    }

    // Remove any previous aliases associated with this service_id
    state.aliases.retain(|_, id| *id != *service_id);

    // Store in DashMap
    state.aliases.insert(alias, service_id.clone());
    state.endpoints.insert(service_id.clone(), (payload.clone(), Instant::now()));

    if let Some(parent_url) = &state.parent_registry_url
        && !payload.info.is_private
    {
        propagate_registration(payload, parent_url.clone());
    }

    Ok(StatusCode::OK)
}

fn verify_endpoint_signature(
    state: &RegistryState,
    payload: &SignedEndpointInfo,
) -> Result<(), (StatusCode, String)> {
    if let Err(e) = payload.verify(RecordTrust::Publishing) {
        return Err((StatusCode::UNAUTHORIZED, format!("Signature verification failed: {e}")));
    }

    // Defence in depth, not the gate: this stops a revoked instance key from
    // refreshing a record at a registry that already holds the master's
    // anchor. It costs a map lookup and no network call. Revocation is
    // actually enforced at the handshake, where a revoked temporary DID
    // cannot complete a connection at all -- so a record that slips past
    // this check still buys its holder nothing.
    if let Some(cert) = &payload.info.delegation
        && let Some(anchor) = state.master_anchors.get(&cert.master_did)
        && anchor.0.payload.revoked_keys.contains(&cert.temporary_did)
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("instance key {} has been revoked by its master", cert.temporary_did),
        ));
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

    state.master_anchors.insert(payload.master_id.clone(), (payload, Instant::now()));
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
    use axum::http::StatusCode;
    use syneroym_core::{
        config::{AccessControl, ServiceRegistryRole, SubstrateConfig},
        dht_registry::{
            EndpointInfo, EndpointMechanism, EndpointType, MASTER_ANCHOR_SCHEMA_V1,
            MasterAnchorPayload, RegistryClient,
        },
        util,
    };
    use syneroym_identity::{
        DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
    };

    use super::*;

    fn create_signed_info(identity: &Identity, info: EndpointInfo) -> SignedEndpointInfo {
        info.sign(identity).unwrap()
    }

    fn sample_service_info() -> EndpointInfo {
        EndpointInfo {
            service_id: "placeholder".to_string(),
            substrate_id: "did:key:zSubstrate".to_string(),
            endpoint_type: EndpointType::Service,
            nickname: None,
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            delegation: None,
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
            delegation: None,
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
            delegation: None,
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
            delegation: None,
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
                delegation: None,
            },
            pkarr_packet_hex: "mock-hex".to_string(),
        };
        state.endpoints.insert(substrate_id.to_string(), (substrate_info.clone(), Instant::now()));

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
                delegation: None,
            },
            pkarr_packet_hex: "mock-hex".to_string(),
        };
        state.endpoints.insert(service_id.to_string(), (service_info, Instant::now()));

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
            delegation: None,
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
            delegation: None,
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
    async fn a_delegation_signed_record_registers_and_looks_up_under_its_master_did() {
        let state = Arc::new(RegistryState::default());
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let signed = sample_service_info().sign_as_instance(&instance, cert).unwrap();

        let res = register_endpoint(State(state.clone()), Json(signed.clone())).await;
        assert_eq!(res.unwrap(), StatusCode::OK);

        let lookup_res = lookup_endpoint(Path(master_did.clone()), State(state)).await;
        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, master_did);
    }

    #[tokio::test]
    async fn a_record_signed_by_a_revoked_instance_key_is_rejected_at_admission() {
        let state = Arc::new(RegistryState::default());
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let instance_did = substrate::derive_did_key(&instance.public_key());

        let anchor_payload =
            MasterAnchorPayload { revoked_keys: vec![instance_did.clone()], ..Default::default() };
        let signed_anchor = anchor_payload.sign(&master).unwrap();
        state.master_anchors.insert(master_did, (signed_anchor, Instant::now()));

        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let signed = sample_service_info().sign_as_instance(&instance, cert).unwrap();

        let res = register_endpoint(State(state), Json(signed)).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_delegated_records_alias_is_derived_from_its_master_did() {
        let state = Arc::new(RegistryState::default());
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mut info = sample_service_info();
        info.nickname = Some("member-one".to_string());
        let signed = info.sign_as_instance(&instance, cert).unwrap();

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
