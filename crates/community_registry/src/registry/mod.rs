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
        return Err((StatusCode::UNAUTHORIZED, format!("Signature verification failed: {e}")));
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
mod tests;
