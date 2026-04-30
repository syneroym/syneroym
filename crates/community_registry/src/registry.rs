use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use dashmap::DashMap;
use ed25519_dalek::{Signature, Verifier};
use std::sync::Arc;
use syneroym_core::community_registry::{EndpointType, SignedEndpointInfo};
use syneroym_core::config::SubstrateConfig;
use syneroym_identity::substrate::{canonicalize_json_value, resolve_did_key};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, error, info};

pub struct EcosystemRegistry {
    bind_address: String,
    state: Arc<RegistryState>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
}

struct RegistryState {
    // Map of service_id -> SignedEndpointInfo
    endpoints: DashMap<String, SignedEndpointInfo>,
}

impl EcosystemRegistry {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing service registry");

        let bind_address = config
            .roles
            .community_registry
            .as_ref()
            .expect("community registry role must be enabled to initialize registry")
            .http_bind_address
            .clone();

        Ok(Self {
            bind_address,
            state: Arc::new(RegistryState { endpoints: DashMap::new() }),
            shutdown_tx: None,
            server_handle: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running service registry on {}", self.bind_address);

        let app = Router::new()
            .route("/register", post(register_endpoint))
            .route("/lookup/{service_id}", get(lookup_endpoint))
            .with_state(self.state.clone());

        let listener = TcpListener::bind(&self.bind_address)
            .await
            .context("Failed to bind registry listener")?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

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

        // Keep the run method from returning until shut down
        std::future::pending::<()>().await;
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

    // Resolve public key
    let pubkey = resolve_did_key(service_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid service_id (did:key): {}", e)))?;
    debug!("Registering public key: {:?} to registry", pubkey);

    // Canonicalize EndpointInfo
    let info_value = serde_json::to_value(&payload.info)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let canonical_value = canonicalize_json_value(&info_value);
    let canonical_string = serde_json::to_string(&canonical_value)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Verify signature
    let sig_bytes = z32::decode(payload.signature.as_bytes()).map_err(|_| {
        (StatusCode::BAD_REQUEST, "Invalid z-base-32 signature encoding".to_string())
    })?;

    if sig_bytes.len() != 64 {
        return Err((StatusCode::BAD_REQUEST, "Invalid signature length".to_string()));
    }

    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid signature format: {}", e)))?;

    if pubkey.verify(canonical_string.as_bytes(), &signature).is_err() {
        return Err((StatusCode::UNAUTHORIZED, "Signature verification failed".to_string()));
    }

    // Store in DashMap
    state.endpoints.insert(service_id.clone(), payload);
    Ok(StatusCode::OK)
}

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub resolve: Option<bool>,
}

async fn lookup_endpoint(
    Path(service_id): Path<String>,
    Query(query): Query<LookupQuery>,
    State(state): State<Arc<RegistryState>>,
) -> Result<Json<SignedEndpointInfo>, StatusCode> {
    let mut entry = state.endpoints.get(&service_id).map(|e| e.clone());

    if query.resolve.unwrap_or(false)
        && let Some(record) = &entry
        && record.info.endpoint_type == EndpointType::Service
    {
        entry = state.endpoints.get(&record.info.substrate_id).map(|e| e.clone());
    }

    if let Some(entry) = entry { Ok(Json(entry)) } else { Err(StatusCode::NOT_FOUND) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syneroym_core::community_registry::EndpointInfo;
    use syneroym_identity::Identity;
    use syneroym_identity::substrate::derive_did_key;

    fn create_signed_info(identity: &Identity, info: EndpointInfo) -> SignedEndpointInfo {
        let info_value = serde_json::to_value(&info).unwrap();
        let canonical_value = canonicalize_json_value(&info_value);
        let canonical_string = serde_json::to_string(&canonical_value).unwrap();

        let signature = identity.sign(canonical_string.as_bytes());
        let signature_z32 = z32::encode(&signature.to_bytes());

        SignedEndpointInfo { info, signature: signature_z32 }
    }

    #[tokio::test]
    async fn test_register_and_lookup_success() {
        let state = Arc::new(RegistryState { endpoints: DashMap::new() });
        let identity = Identity::generate().unwrap();
        let did = derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            relay_url: Some("http://relay.example.com".to_string()),
            endpoint_addr_bytes: vec![1, 2, 3],
        };

        let signed_info = create_signed_info(&identity, info);

        // Register
        let res = register_endpoint(State(state.clone()), Json(signed_info.clone())).await;
        assert_eq!(res.unwrap(), StatusCode::OK);

        // Lookup
        let lookup_res =
            lookup_endpoint(Path(did), Query(LookupQuery { resolve: None }), State(state)).await;

        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, signed_info.info.service_id);
    }

    #[tokio::test]
    async fn test_register_invalid_signature() {
        let state = Arc::new(RegistryState { endpoints: DashMap::new() });
        let identity = Identity::generate().unwrap();
        let other_identity = Identity::generate().unwrap();
        let did = derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Substrate,
            relay_url: None,
            endpoint_addr_bytes: vec![],
        };

        // Sign with OTHER identity
        let signed_info = create_signed_info(&other_identity, info);

        let res = register_endpoint(State(state), Json(signed_info)).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_register_invalid_did() {
        let state = Arc::new(RegistryState { endpoints: DashMap::new() });
        let identity = Identity::generate().unwrap();

        let info = EndpointInfo {
            service_id: "invalid-did".to_string(),
            substrate_id: "invalid-did".to_string(),
            endpoint_type: EndpointType::Substrate,
            relay_url: None,
            endpoint_addr_bytes: vec![],
        };

        let signed_info = create_signed_info(&identity, info);

        let res = register_endpoint(State(state), Json(signed_info)).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_indirect_lookup() {
        let state = Arc::new(RegistryState { endpoints: DashMap::new() });
        let substrate_id = "did:key:hsubstrate";
        let service_id = "did:key:hservice";

        // Mock a substrate record
        let substrate_info = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: substrate_id.to_string(),
                substrate_id: substrate_id.to_string(),
                endpoint_type: EndpointType::Substrate,
                relay_url: None,
                endpoint_addr_bytes: vec![42],
            },
            signature: "mock-sig".to_string(),
        };
        state.endpoints.insert(substrate_id.to_string(), substrate_info.clone());

        // Mock a service record pointing to that substrate
        let service_info = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: service_id.to_string(),
                substrate_id: substrate_id.to_string(),
                endpoint_type: EndpointType::Service,
                relay_url: None,
                endpoint_addr_bytes: vec![],
            },
            signature: "mock-sig".to_string(),
        };
        state.endpoints.insert(service_id.to_string(), service_info);

        // Lookup service with resolve=true
        let lookup_res = lookup_endpoint(
            Path(service_id.to_string()),
            Query(LookupQuery { resolve: Some(true) }),
            State(state.clone()),
        )
        .await;

        let Json(retrieved) = lookup_res.unwrap();
        assert_eq!(retrieved.info.service_id, substrate_id);
        assert_eq!(retrieved.info.endpoint_addr_bytes, vec![42]);

        // Lookup service with resolve=false
        let lookup_res_no_resolve = lookup_endpoint(
            Path(service_id.to_string()),
            Query(LookupQuery { resolve: Some(false) }),
            State(state),
        )
        .await;

        let Json(retrieved_no_resolve) = lookup_res_no_resolve.unwrap();
        assert_eq!(retrieved_no_resolve.info.service_id, service_id);
    }

    #[tokio::test]
    async fn test_lookup_not_found() {
        let state = Arc::new(RegistryState { endpoints: DashMap::new() });
        let res = lookup_endpoint(
            Path("non-existent".to_string()),
            Query(LookupQuery { resolve: None }),
            State(state),
        )
        .await;

        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), StatusCode::NOT_FOUND);
    }
}
