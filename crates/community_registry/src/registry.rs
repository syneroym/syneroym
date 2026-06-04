//! Community Service Registry
//!
//! A public/shared registry server allowing nodes to register their network addresses
//! and nicknames, enabling global peer lookup.

use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use dashmap::DashMap;
use ed25519_dalek::VerifyingKey;
use oneshot::Sender;
use reqwest::Client;
use syneroym_core::{
    config::SubstrateConfig,
    dht_registry::{DEFAULT_REGISTRY_TTL_SECS, SignedEndpointInfo},
    util,
};
use syneroym_identity::substrate::resolve_did_key;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time};
use tracing::{debug, error, info};

pub struct EcosystemRegistry {
    bind_address: String,
    state: Arc<RegistryState>,
    shutdown_tx: Option<Sender<()>>,
    server_handle: Option<JoinHandle<()>>,
    listener: Option<TcpListener>,
}

impl Debug for EcosystemRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
    // Needed when registry is not accessible from internal network and multi-hop-relays are needed for data transfer
    parent_registry_url: Option<String>,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self { endpoints: DashMap::new(), aliases: DashMap::new(), parent_registry_url: None }
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
                    tracing::debug!("Expired registry entry for {}", key);
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

    verify_endpoint_signature(&payload)?;

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
    payload: &SignedEndpointInfo,
) -> Result<VerifyingKey, (StatusCode, String)> {
    let service_id = &payload.info.service_id;

    if let Err(e) = payload.verify() {
        return Err((StatusCode::UNAUTHORIZED, format!("Signature verification failed: {}", e)));
    }

    // Resolve public key
    let pubkey = resolve_did_key(service_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid service_id (did:key): {e}")))?;
    debug!("Registering public key: {:?} to registry", pubkey);

    Ok(pubkey)
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
                tracing::warn!(
                    "Failed to propagate registration to {} (status: {})",
                    url,
                    resp.status()
                );
            }
            Err(e) => {
                tracing::warn!("Error propagating registration to {}: {}", url, e);
            }
        }
    });
}

#[derive(serde::Deserialize)]
pub struct LookupQuery {}

async fn lookup_endpoint(
    Path(service_id): Path<String>,
    Query(_query): Query<LookupQuery>,
    State(state): State<Arc<RegistryState>>,
) -> Result<Json<SignedEndpointInfo>, StatusCode> {
    let resolved_id = state.aliases.get(&service_id).map(|e| e.clone()).unwrap_or(service_id);
    let entry = state.endpoints.get(&resolved_id).map(|e| e.0.clone());

    if let Some(entry) = entry { Ok(Json(entry)) } else { Err(StatusCode::NOT_FOUND) }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use syneroym_core::{
        dht_registry::{EndpointInfo, EndpointMechanism, EndpointType},
        util,
    };
    use syneroym_identity::{Identity, substrate::derive_did_key};

    use super::*;

    fn create_signed_info(identity: &Identity, info: EndpointInfo) -> SignedEndpointInfo {
        info.sign(identity).unwrap()
    }

    #[tokio::test]
    async fn test_register_and_lookup_success() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let did = derive_did_key(&identity.public_key());

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
        };

        let signed_info = create_signed_info(&identity, info);

        // Register
        let res = register_endpoint(State(state.clone()), Json(signed_info.clone())).await;
        assert_eq!(res.unwrap(), StatusCode::OK);

        // Lookup by alias
        let service_hash = util::short_hash(&did);
        let alias = format!("alice-p{service_hash}");
        let lookup_res = lookup_endpoint(Path(alias), Query(LookupQuery {}), State(state)).await;

        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, signed_info.info.service_id);
    }

    #[tokio::test]
    async fn test_register_invalid_signature() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let other_identity = Identity::generate().unwrap();
        let did = derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: None,
            mechanisms: vec![],
            is_private: false,
            ttl: None,
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
            },
            pkarr_packet_hex: "mock-hex".to_string(),
        };
        state.endpoints.insert(service_id.to_string(), (service_info, Instant::now()));

        // Lookup service
        let lookup_res = lookup_endpoint(
            Path(service_id.to_string()),
            Query(LookupQuery {}),
            State(state.clone()),
        )
        .await;

        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, service_id);
        // Ensure mechanisms are NOT populated since we removed server-side resolution
        assert!(retrieved.info.mechanisms.is_empty());
    }

    #[tokio::test]
    async fn test_lookup_by_shorthash_no_nickname() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let did = derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: None, // No nickname
            mechanisms: vec![],
            is_private: false,
            ttl: None,
        };

        let signed_info = create_signed_info(&identity, info);
        register_endpoint(State(state.clone()), Json(signed_info)).await.unwrap();

        // Lookup by shorthash (p{hash}) should work
        let service_hash = util::short_hash(&did);
        let alias = format!("p{service_hash}");
        let lookup_res = lookup_endpoint(Path(alias), Query(LookupQuery {}), State(state)).await;

        assert!(lookup_res.is_ok());
        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, did);
    }

    #[tokio::test]
    async fn test_lookup_by_shorthash_fails_if_nickname_present() {
        let state = Arc::new(RegistryState::default());
        let identity = Identity::generate().unwrap();
        let did = derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            nickname: Some("alice".to_string()),
            mechanisms: vec![],
            is_private: false,
            ttl: None,
        };

        let signed_info = create_signed_info(&identity, info);
        register_endpoint(State(state.clone()), Json(signed_info)).await.unwrap();

        // Lookup by shorthash-only (p{hash}) should FAIL because nickname was provided
        let service_hash = util::short_hash(&did);
        let alias = format!("p{service_hash}");
        let lookup_res = lookup_endpoint(Path(alias), Query(LookupQuery {}), State(state)).await;

        assert!(lookup_res.is_err());
        assert_eq!(lookup_res.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_lookup_not_found() {
        let state = Arc::new(RegistryState::default());
        let res =
            lookup_endpoint(Path("non-existent".to_string()), Query(LookupQuery {}), State(state))
                .await;

        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), StatusCode::NOT_FOUND);
    }
}
