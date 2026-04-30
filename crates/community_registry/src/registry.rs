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
