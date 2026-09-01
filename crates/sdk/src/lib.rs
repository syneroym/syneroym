#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym App SDK
//!
//! High-level APIs and traits to help third-party developers build apps
//! that integrate seamlessly with the Syneroym runtime and services.

use std::{
    fmt::{self, Debug, Formatter},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl,
    endpoint::{Connection, RecvStream, SendStream},
};
pub mod deploy;
pub mod health;
pub mod mapper;
pub mod topology;
pub use deploy::{
    ApplyReport, ApplyRequest, DeployTarget, ServiceFailure, SubstrateActor, apply_plan,
    resolve_targets,
};
use serde_json::Value;
use syneroym_core::dht_registry::{EndpointMechanism, RegistryClient, SignedEndpointInfo};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport, SYNEROYM_ALPN};
use syneroym_rpc::{
    CapabilityToken, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse,
    MESSAGING_MESSAGE_METHOD, MessagingNotification, framing,
};
pub use syneroym_rpc::{DeadLetterInfo, QueuedCallInfo, SagaInfo, SagaState};
pub use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, AssetBundle, BindingWrite, ContainerManifest, ContainerPortMapping,
    ContainerVolumeMapping, DependencyBinding, DeployManifest, DeploymentPlan, HealthCheck,
    HttpProbe, InstanceIdentity, NetworkEndpoint, PlannedService, RpcProbe, ServiceConfig,
    ServiceType, TcpManifest, TcpProbe, TopologyMode, Visibility, WasmManifest,
};
use tokio::{io, net::TcpStream, sync::mpsc, task::JoinHandle, time};
pub use topology::{RegistryTopologyFetcher, fetch_and_register};
use tracing::debug;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeployedService {
    pub service_id: String,
    pub interfaces: Vec<String>,
    pub endpoint_type: String,
    /// Unix seconds when the installed instance certificate expires, if one
    /// is installed. `#[serde(default)]` so a substrate predating this field
    /// still deserializes.
    #[serde(default)]
    pub instance_certificate_expires_at: Option<u64>,
    /// Declared publication visibility (ADR-0018 §4). `#[serde(default)]` so a
    /// substrate predating this field still deserializes as `None`.
    #[serde(default)]
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SigningIdentityInfo {
    pub signing_did: String,
    pub pubkey_hex: String,
    pub owner_did: Option<String>,
}

/// Publication policy for a service deployment (ADR-0018 §4). One type rather
/// than two loose fields, so the three legal pairings are the only ones a
/// caller can express -- the substrate still validates independently.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Publication {
    /// Never registered. The default.
    #[default]
    Private,
    /// Registered, and propagated to parent registries. The record's own
    /// `is_private` must be `false`.
    Public(SignedEndpointInfo),
    /// Registered with the local registry only. The record's own
    /// `is_private` must be `true`.
    Internal(SignedEndpointInfo),
}

impl Publication {
    /// `(visibility, serialized certificate)` for a `DeployManifest`.
    pub fn split(self) -> Result<(Visibility, Option<String>)> {
        match self {
            Self::Private => Ok((Visibility::Private, None)),
            Self::Public(record) => {
                let serialized = serde_json::to_string(&record).map_err(|e| {
                    anyhow::anyhow!("Failed to serialize registry certificate: {e}")
                })?;
                Ok((Visibility::Public, Some(serialized)))
            }
            Self::Internal(record) => {
                let serialized = serde_json::to_string(&record).map_err(|e| {
                    anyhow::anyhow!("Failed to serialize registry certificate: {e}")
                })?;
                Ok((Visibility::Internal, Some(serialized)))
            }
        }
    }
}

/// Whether the substrate believes a service's instance is running (M05A A4).
/// Deliberately not a bool: a supervisor's remediation differs per variant,
/// and `Unknown` must never be silently read as healthy.
///
/// **No `rename_all`**: this mirrors the wire type
/// (`syneroym_wit_interfaces::control_plane::exports::…::InstancePhase`),
/// which `wit_bindgen`'s `additional_derives` gives a plain, unrenamed
/// `Serialize`/`Deserialize` -- its JSON tags are the literal Rust variant
/// names (`"Running"`, not `"running"`). A `kebab-case` rename here would
/// silently stop parsing the server's actual response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InstancePhase {
    Running,
    NotRunning(String),
    Unknown(String),
    /// Also what a caller without a grant on an explicitly named id gets
    /// back -- identical to an id never deployed at all, deliberately
    /// (A4-10), so a caller with no grant cannot use this to probe for an
    /// id's existence.
    NotFound,
}

/// **No `rename_all`** -- see [`InstancePhase`]'s doc comment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProbeStatus {
    NotDeclared,
    Passing,
    Failing(String),
}

/// Outcome of one epoch-guarded binding write (ADR-0021 §3, M05A A5a).
/// **No `rename_all`** -- see [`InstancePhase`]'s doc comment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BindingWriteOutcome {
    Applied,
    NoOp,
    Stale(u64),
    Conflict(u64),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub service_id: String,
    pub service_type: Option<String>,
    pub endpoint_type: String,
    pub app_instance_id: Option<String>,
    pub service_name: Option<String>,
    pub phase: InstancePhase,
    pub probe: ProbeStatus,
    pub instance_certificate_issued_at: Option<u64>,
    pub instance_certificate_expires_at: Option<u64>,
    pub probe_checked_at: Option<u64>,
    /// Per declared dependency of this service, the epoch this substrate
    /// currently serves it (M05A A5a). `status`'s per-dependent binding
    /// convergence report.
    pub binding_epochs: Vec<(String, u64)>,
}

/// What this node is, as opposed to what is running on it (M05A A4). Present
/// only for a caller holding node-wide `orchestrator/status`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeFacts {
    pub node_did: String,
    pub service_types: Vec<String>,
    pub registry_url: Option<String>,
    pub dht_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubstrateStatus {
    pub node: Option<NodeFacts>,
    pub checked_at: u64,
    pub services: Vec<ServiceStatus>,
}

/// Default ceiling for establishing a connection to a single mechanism.
/// Without a bound here, iroh's relay/hole-punch retries can churn
/// indefinitely against an unreachable or overloaded peer, leaving the
/// caller with no way to give up. Override via
/// [`SyneroymClient::with_connect_timeout`].
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// M06A D-A1-5: the authoritative size check the asset-bundle deploy-time
/// caps are only a "cheap early guard" ahead of
/// (`crates/core/src/deploy_docs.rs`'s `MAX_ASSET_BUNDLE_BYTES` doc
/// comment) -- every binary artifact in a request's `params` (a Wasm
/// component, an asset bundle archive, a container volume file, ...)
/// serializes as a JSON integer array, ~3.57x its raw byte length, and all
/// of them share this one frame. Checking the real serialized length here
/// -- called from `open_request_stream` before any network I/O for the
/// call, not only before the send -- is exact (no estimated ratio to keep
/// in sync with serde's actual encoding) and general: it catches *any*
/// oversized request through this client, not only a deploy's asset
/// bundle, and fails before wasting a stream open and a round trip on
/// bytes the peer's own `framing::read_frame` would reject anyway with a
/// bare byte count and no context about which call or field caused it.
fn check_frame_size(method: &str, req_bytes: &[u8]) -> Result<()> {
    if req_bytes.len() as u64 > framing::MAX_FRAME_SIZE as u64 {
        return Err(anyhow::anyhow!(
            "'{method}' request is {} bytes once serialized, exceeding the {} byte frame limit \
             (MAX_FRAME_SIZE) -- reduce the size of any inline binary content (a Wasm component, \
             an asset bundle archive, container volume files) in this call's params",
            req_bytes.len(),
            framing::MAX_FRAME_SIZE
        ));
    }
    Ok(())
}

pub struct SyneroymClient {
    service_id: String,
    registry_url: String,
    provided_mechanisms: Option<Vec<EndpointMechanism>>,
    /// Overrides which DID `connect()` queries the registry under, when it
    /// differs from `service_id` itself. `None` for every constructor
    /// except [`Self::new_with_record`]: a *private* service (ADR-0018 §2)
    /// is never registered, so looking `service_id` up would fail, while
    /// the substrate hosting it always is (it publishes itself on every
    /// heartbeat) -- `new_with_record` sets this to that substrate's own
    /// DID, taken from the signed record, so `connect()` resolves *that*
    /// instead.
    registry_lookup_override: Option<String>,
    connection: Option<TransportConnection>,
    connect_timeout: Duration,
    /// A self-asserted caller identity (pubkey only, no delegation) sent on
    /// every outbound preamble (M04A Slice B0, ADR-0016 §4.2/§0.5). Without
    /// it, every SDK-driven call resolves to the anonymous bucket once the
    /// router makes verify_preamble mandatory for native-capability
    /// dispatch.
    ///
    /// TODO(M04B/FDAE): a self-asserted pubkey is an assertion, not proof-
    /// of-possession (the no-delegation handshake path does not challenge
    /// it). B1/M04B tighten this to verified UCAN chains; B0 only needs
    /// "not anonymous."
    identity: Identity,
    /// A verified UCAN capability chain to present on every outbound
    /// preamble (M04A Slice B1), set via [`Self::with_ucan`]. `None` by
    /// default -- callers that don't hold one still get the B0 self-
    /// asserted-identity admission.
    caller_ucan: Option<CapabilityToken>,
    /// Handles for endpoints from failed/timed-out connect attempts (see
    /// `spawn_background_close`), reaped by [`Self::shutdown`]. Unlike the
    /// `Drop` safety net, a retry loop such as `wait_for_ready` can spawn
    /// many of these against a single client; leaving them untracked lets
    /// the runtime abort them mid-close on teardown instead of letting them
    /// finish, which is what was producing iroh's "Endpoint dropped without
    /// calling `Endpoint::close`" warning.
    pending_closes: Vec<JoinHandle<()>>,
    /// Whether `registry_client()` builds its cached client with a real
    /// mainline-DHT client attached. `true` by default (existing behavior:
    /// the DHT is a best-effort backup, checked only if the HTTP registry
    /// lookup fails), overridden via [`Self::with_registry_dht`] by a
    /// caller that wants to opt out entirely -- e.g. a node whose own
    /// `enable_bep0044_dht` is off should not spin one up for its own
    /// outbound connections either.
    enable_dht: bool,
    /// Lazily built and cached on first use, then reused for the rest of
    /// this client's lifetime. Building a [`RegistryClient`] with DHT
    /// enabled spins up a real mainline-DHT client (a UDP socket plus a
    /// background routing-table-bootstrap task); rebuilding one on every
    /// `connect`/`lookup_registry` call -- as this used to do -- meant a
    /// retry loop like `wait_for_ready` (polling every 500ms) leaked one
    /// of these every iteration, exhausting sockets/fds under sustained
    /// retry pressure instead of just paying the bootstrap cost once.
    registry_client: OnceLock<Arc<RegistryClient>>,
}

impl Debug for SyneroymClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyneroymClient")
            .field("service_id", &self.service_id)
            .field("registry_url", &self.registry_url)
            .field("provided_mechanisms", &self.provided_mechanisms)
            .field("registry_lookup_override", &self.registry_lookup_override)
            .field("connection", &self.connection)
            .field("connect_timeout", &self.connect_timeout)
            .field("identity", &self.identity)
            .field("caller_ucan", &self.caller_ucan)
            .field("pending_closes", &self.pending_closes.len())
            .finish()
    }
}

/// Ephemeral identity generated when the caller did not supply one. Panics on
/// `expect` (mirroring `RouteHandler::new_coordinator`'s own ephemeral-
/// identity fallback) so `SyneroymClient::new`/`new_with_mechanisms` stay
/// infallible for their many existing callers.
#[allow(clippy::expect_used)]
fn generate_ephemeral_identity() -> Identity {
    Identity::generate().expect("failed to generate ephemeral SDK client identity")
}

/// Everything optional about a [`SyneroymClient::deploy_svc_wasm_with_options`]
/// call (M06A A2, `D-A2-9`). Replaces the growing positional tail
/// `deploy_svc_wasm_with_assets` had started: `assets` was A1's addition,
/// `custom_config` is A2's, and a third would have meant a third method.
#[derive(Debug, Default)]
pub struct DeploySvcOptions {
    pub publication: Publication,
    pub instance_certificate: Option<DelegationCertificate>,
    pub assets: Option<AssetBundle>,
    /// Verbatim `ServiceConfig.custom_config`. The reserved `http_routes`
    /// key inside it is what declares HTTP routes.
    pub custom_config: Option<String>,
}

#[derive(Clone)]
pub enum TransportConnection {
    Iroh {
        /// The endpoint must be kept alive for the duration of the connection.
        /// Dropping it closes the underlying QUIC socket, terminating all
        /// streams.
        endpoint: Endpoint,
        conn: Connection,
    },
}

impl Debug for TransportConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Iroh { conn, .. } => f
                .debug_struct("TransportConnection::Iroh")
                .field("endpoint", &"iroh::Endpoint")
                .field("conn", &format!("{:?}", conn.remote_id()))
                .finish(),
        }
    }
}

/// A live `messaging/subscribe` stream: `.recv()` yields `(topic, payload)`
/// pairs as the broker delivers them. Dropping it drops the send half of
/// the underlying bidirectional stream, which the router-side handler
/// observes as the client having gone away (close-as-unsubscribe).
pub struct MessageStream {
    receiver: mpsc::Receiver<(String, Vec<u8>)>,
    send: SendStream,
}

impl Debug for MessageStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageStream").finish_non_exhaustive()
    }
}

impl MessageStream {
    pub async fn recv(&mut self) -> Option<(String, Vec<u8>)> {
        self.receiver.recv().await
    }

    /// Closes the send half only (without dropping `self`), signalling
    /// the router-side handler to unsubscribe (close-as-unsubscribe).
    /// `.recv()` remains usable afterward and resolves to `None` once the
    /// router's own writer task exits in response and closes its side.
    pub fn stop(&mut self) -> Result<()> {
        self.send.finish().map_err(Into::into)
    }
}

/// Closes an iroh endpoint without making the caller wait for it, returning
/// the task's handle so it can be reaped later.
///
/// `Endpoint::close` is a graceful QUIC shutdown that, per its own docs, can
/// take up to ~3s to resolve against a peer with bad connectivity — it
/// notifies remaining peers and waits for their acknowledgment. That's fine
/// for a connection that succeeded, but on a connect failure or timeout it
/// would silently add ~3s on top of whatever deadline the caller configured.
/// Closing is still worth doing for the peer's sake, just not on the
/// caller's clock.
fn spawn_background_close(endpoint: Endpoint) -> JoinHandle<()> {
    tokio::spawn(async move {
        endpoint.close().await;
    })
}

impl SyneroymClient {
    #[must_use]
    pub fn new(service_id: String, registry_url: String) -> Self {
        Self {
            service_id,
            registry_url,
            provided_mechanisms: None,
            registry_lookup_override: None,
            connection: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity: generate_ephemeral_identity(),
            caller_ucan: None,
            pending_closes: Vec::new(),
            enable_dht: true,
            registry_client: OnceLock::new(),
        }
    }

    #[must_use]
    pub fn new_with_mechanisms(service_id: String, mechanisms: Vec<EndpointMechanism>) -> Self {
        Self {
            service_id,
            registry_url: String::new(),
            provided_mechanisms: Some(mechanisms),
            registry_lookup_override: None,
            connection: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity: generate_ephemeral_identity(),
            caller_ucan: None,
            pending_closes: Vec::new(),
            enable_dht: true,
            registry_client: OnceLock::new(),
        }
    }

    /// Connect using a privately-shared endpoint record (ADR-0018 §2).
    ///
    /// Supersedes `new_with_mechanisms` for every case that has a record: the
    /// record is verified on import, names the substrate that hosts the
    /// service, and that substrate's own record is public -- so a private
    /// service stays reachable by anyone the deployer handed the file to,
    /// with no registry entry for the service itself.
    pub fn new_with_record(record: SignedEndpointInfo, registry_url: String) -> Result<Self> {
        record.verify().context("failed to verify signed endpoint record")?;
        let service_id = record.info.service_id;
        let substrate_id = record.info.substrate_id;
        let provided_mechanisms =
            if !record.info.mechanisms.is_empty() { Some(record.info.mechanisms) } else { None };
        let registry_lookup_override =
            if provided_mechanisms.is_none() { Some(substrate_id) } else { None };
        Ok(Self {
            service_id,
            registry_url,
            provided_mechanisms,
            registry_lookup_override,
            connection: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity: generate_ephemeral_identity(),
            caller_ucan: None,
            pending_closes: Vec::new(),
            enable_dht: true,
            registry_client: OnceLock::new(),
        })
    }

    /// Like [`Self::new`], but with a caller-supplied, stable identity
    /// (rather than a freshly generated ephemeral one) -- for callers that
    /// need a *known* DID across restarts (e.g. `roymctl`, the client
    /// gateway using the node's own identity).
    #[must_use]
    pub const fn new_with_identity(
        service_id: String,
        registry_url: String,
        identity: Identity,
    ) -> Self {
        Self {
            service_id,
            registry_url,
            provided_mechanisms: None,
            registry_lookup_override: None,
            connection: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity,
            caller_ucan: None,
            pending_closes: Vec::new(),
            enable_dht: true,
            registry_client: OnceLock::new(),
        }
    }

    /// Overrides the default per-mechanism connect deadline (see
    /// [`DEFAULT_CONNECT_TIMEOUT`]).
    #[must_use]
    pub const fn with_connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    /// Attaches a verified UCAN capability chain (M04A Slice B1) to present
    /// on every outbound preamble opened by this client.
    #[must_use]
    pub fn with_ucan(mut self, caller_ucan: CapabilityToken) -> Self {
        self.caller_ucan = Some(caller_ucan);
        self
    }

    /// Opts this client's `connect`/`lookup_registry` out of the mainline
    /// DHT (see the `enable_dht` field doc). No effect once the cached
    /// registry client has already been built -- call before the first
    /// `connect`/`lookup_registry`/`lookup`/`wait_for_ready`.
    #[must_use]
    pub fn with_registry_dht(mut self, enable: bool) -> Self {
        self.enable_dht = enable;
        self
    }

    /// The registry client backing `connect`/`lookup_registry`, built once
    /// and cached (see the `registry_client` field doc) rather than on
    /// every call.
    fn registry_client(&self) -> Arc<RegistryClient> {
        self.registry_client
            .get_or_init(|| {
                Arc::new(RegistryClient::new(self.enable_dht, Some(self.registry_url.clone())))
            })
            .clone()
    }

    pub async fn connect(&mut self) -> Result<()> {
        if self.connection.is_some() {
            return Ok(());
        }

        debug!("Connecting to {} via registry or provided mechanisms", self.service_id);

        let lookup_target = self.registry_lookup_override.as_deref().unwrap_or(&self.service_id);
        let mechanisms = if let Some(m) = &self.provided_mechanisms {
            m.clone()
        } else if !self.registry_url.is_empty() {
            let info = self.registry_client().lookup(lookup_target, true).await?.info;
            if self.registry_lookup_override.is_none() {
                // The lookup might have been done by an alias. Update service_id to the
                // canonical DID.
                self.service_id = info.service_id;
            }
            info.mechanisms
        } else {
            return Err(anyhow::anyhow!("No registry URL or mechanisms provided"));
        };

        self.connect_with_mechanisms(mechanisms).await
    }

    pub async fn connect_with_mechanisms(
        &mut self,
        mechanisms: Vec<EndpointMechanism>,
    ) -> Result<()> {
        // Try mechanisms. Currently only Iroh is implemented.
        for mechanism in mechanisms {
            match mechanism {
                EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } => {
                    let mut endpoint_addr: EndpointAddr =
                        serde_json::from_slice(&endpoint_addr_bytes)?;

                    let mut ep_bldr = Endpoint::empty_builder();
                    if let Some(relay) = relay_url
                        && let Ok(url) = relay.parse::<RelayUrl>()
                    {
                        ep_bldr =
                            ep_bldr.relay_mode(RelayMode::Custom(RelayMap::from(url.clone())));
                        endpoint_addr = endpoint_addr.with_relay_url(url);
                    }

                    let endpoint = ep_bldr.bind().await?;
                    let dial = endpoint.connect(endpoint_addr, SYNEROYM_ALPN);
                    match time::timeout(self.connect_timeout, dial).await {
                        Ok(Ok(conn)) => {
                            self.connection = Some(TransportConnection::Iroh { endpoint, conn });
                            return Ok(());
                        }
                        Ok(Err(e)) => {
                            self.pending_closes.push(spawn_background_close(endpoint));
                            return Err(e.into());
                        }
                        Err(_) => {
                            self.pending_closes.push(spawn_background_close(endpoint));
                            return Err(anyhow::anyhow!(
                                "connect to {} timed out after {:?}",
                                self.service_id,
                                self.connect_timeout
                            ));
                        }
                    }
                }
                EndpointMechanism::WebRtc { .. } => {
                    // Not implemented
                }
            }
        }

        Err(anyhow::anyhow!("No supported communication mechanism found for {}", self.service_id))
    }

    pub async fn lookup(&self) -> Result<SignedEndpointInfo> {
        self.lookup_registry().await
    }

    pub async fn wait_for_discovery(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.lookup_registry().await.is_ok() {
                return Ok(());
            }
            time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow::anyhow!("Timed out waiting for {} to be discovered", self.service_id))
    }

    pub async fn wait_for_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            // Bound this attempt's connect by whatever remains of the caller's
            // budget, not just `connect_timeout`: otherwise a `connect_timeout`
            // larger than the remaining budget would let this call overrun the
            // deadline it was asked to honor.
            let remaining = timeout.saturating_sub(start.elapsed());
            match time::timeout(remaining, self.connect()).await {
                Ok(Ok(())) => {
                    // Check if readyz is ok
                    match self.request("orchestrator", "readyz", serde_json::json!({})).await {
                        Ok(res) if res.result == serde_json::json!({"status": "ok"}) => {
                            return Ok(());
                        }
                        Ok(_) => debug!("Substrate not ready yet (readyz != ok)"),
                        Err(e) => debug!("readyz request failed: {}", e),
                    }
                }
                Ok(Err(e)) => {
                    debug!("Connect attempt failed, retrying: {}", e);
                }
                Err(_) => {
                    debug!("Connect attempt exceeded remaining wait_for_ready budget");
                }
            }
            time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow::anyhow!("Timed out waiting for {} to become ready", self.service_id))
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(TransportConnection::Iroh { endpoint, .. }) = self.connection.take() {
            endpoint.close().await;
        }
        for handle in self.pending_closes.drain(..) {
            let _ = handle.await;
        }
        Ok(())
    }

    #[must_use]
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    #[must_use]
    pub fn connection(&self) -> Option<TransportConnection> {
        self.connection.clone()
    }

    pub async fn request(
        &self,
        interface: &str,
        method: &str,
        params: Value,
    ) -> Result<JsonRpcResponse> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(Value::Number(1.into())),
            idempotency_key: None,
        };
        self.request_raw(interface, request).await
    }

    /// Opens a new bidirectional stream to the connected peer, writes the
    /// route preamble and the JSON-RPC request frame, and returns the raw
    /// send/recv halves. Shared by `request_raw` (which finishes the send
    /// half immediately after) and `subscribe` (which must not, so the
    /// stream stays open for pushed notifications).
    async fn open_request_stream(
        &self,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<(SendStream, RecvStream)> {
        let req_bytes = serde_json::to_vec(request)?;
        check_frame_size(&request.method, &req_bytes)?;

        let conn_wrapper = self.connection.as_ref().context("Not connected")?;
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, recv) = conn.open_bi().await?;

                // Every stream must start with a RoutePreamble identifying the target service.
                // A self-asserted pubkey (no delegation) is set so this
                // connection is not anonymous (M04A Slice B0, ADR-0016 §0.5)
                // -- see `SyneroymClient::identity`'s doc comment.
                let mut preamble = RoutePreamble::binary_json_rpc(&self.service_id, interface_name);
                preamble.pubkey = Some(hex::encode(self.identity.public_key().to_bytes()));
                preamble.ucan = self.caller_ucan.clone();
                send.write_all(preamble.to_preamble_line().as_bytes()).await?;

                framing::write_frame(&mut send, &req_bytes).await?;
                Ok((send, recv))
            }
        }
    }

    pub async fn request_raw(
        &self,
        interface_name: &str,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse> {
        let (mut send, mut recv) = self.open_request_stream(interface_name, &request).await?;
        debug!(">>> Wrote request for method: {} to {}", request.method, self.service_id);
        send.finish()?;

        let frame = framing::read_frame(&mut recv).await?;
        if frame.is_empty() {
            return Err(anyhow::anyhow!(
                "Empty response from stream for method {}",
                request.method
            ));
        }
        // A wire error frame is a `JsonRpcErrorResponse` (`{error, ..}`),
        // structurally disjoint from the success shape `JsonRpcResponse`
        // (`{result, ..}`) this deserializes into first -- so a failed parse
        // here doesn't yet mean "malformed response," only "not a success."
        // Falling back to `JsonRpcErrorResponse` recovers the RPC-level
        // `code`/`message` (e.g. `PermissionDenied` at -32010) as a
        // downcastable `JsonRpcError` on the returned `anyhow::Error`,
        // instead of only ever surfacing an opaque deserialize failure.
        let res: JsonRpcResponse = match serde_json::from_slice(&frame) {
            Ok(res) => res,
            Err(parse_err) => {
                return match serde_json::from_slice::<JsonRpcErrorResponse>(&frame) {
                    Ok(err_res) => Err(err_res.error.into()),
                    Err(_) => Err(parse_err.into()),
                };
            }
        };
        debug!("got json response for method: {}: {:?}", request.method, res);
        Ok(res)
    }

    /// Subscribes to `topic` on `interface`'s messaging capability, over a
    /// live push channel. Unlike `request`/`request_raw`, this does **not**
    /// finish the send half of the stream after writing the request:
    /// finishing it would make the router-side reader hit EOF and tear the
    /// whole handler down before any notification arrives. Dropping the
    /// returned `MessageStream` closes the send half, which the router
    /// treats as the unsubscribe signal.
    pub async fn subscribe(&self, interface: &str, topic: &str) -> Result<MessageStream> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "subscribe".to_string(),
            params: serde_json::json!({"topic": topic}),
            id: Some(Value::Number(1.into())),
            idempotency_key: None,
        };
        let (send, mut recv) = self.open_request_stream(interface, &request).await?;

        let ack_frame = framing::read_frame(&mut recv).await?;
        if ack_frame.is_empty() {
            return Err(anyhow::anyhow!(
                "Empty ack for subscribe on topic {topic} (interface {interface})"
            ));
        }
        let ack: JsonRpcResponse = serde_json::from_slice(&ack_frame)?;
        debug!("subscribe ack for topic {}: {:?}", topic, ack);

        let (tx, rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            loop {
                match framing::read_frame(&mut recv).await {
                    Ok(frame) if frame.is_empty() => break,
                    Ok(frame) => {
                        let Ok(notify) = serde_json::from_slice::<JsonRpcRequest>(&frame) else {
                            continue;
                        };
                        if notify.method != MESSAGING_MESSAGE_METHOD {
                            continue;
                        }
                        let Ok(MessagingNotification { topic, payload }) =
                            serde_json::from_value(notify.params)
                        else {
                            continue;
                        };
                        if tx.send((topic, payload)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(MessageStream { receiver: rx, send })
    }

    pub async fn deploy_svc_wasm(
        &self,
        service_id: String,
        interfaces: Vec<String>,
        wasm_bytes: Vec<u8>,
        publication: Publication,
        instance_certificate: Option<DelegationCertificate>,
    ) -> Result<()> {
        self.deploy_svc_wasm_with_options(
            service_id,
            interfaces,
            wasm_bytes,
            DeploySvcOptions { publication, instance_certificate, ..Default::default() },
        )
        .await
    }

    /// [`deploy_svc_wasm`](Self::deploy_svc_wasm), plus everything optional
    /// about a WASM deploy (M06A A2, `D-A2-9`): a static asset bundle
    /// (M06A A1) and a `custom_config` JSON blob, whose reserved
    /// `http_routes` key declares a service's HTTP route table (M3B
    /// Slice 7, M06A A2's `target = "guest"`). Replaces
    /// `deploy_svc_wasm_with_assets`, which had exactly one call site --
    /// a third optional field would have made the next one a fourth
    /// `deploy_svc_wasm_*` method instead of growing this one.
    pub async fn deploy_svc_wasm_with_options(
        &self,
        service_id: String,
        interfaces: Vec<String>,
        wasm_bytes: Vec<u8>,
        options: DeploySvcOptions,
    ) -> Result<()> {
        let (visibility, registry_certificate) = options.publication.split()?;
        let instance_certificate = options
            .instance_certificate
            .map(|c| c.to_json())
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize instance certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: options.custom_config,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: options.assets,
                visibility: Some(visibility),
            },
            service_type: ServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces,
            }),
            registry_certificate,
            instance_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_svc_tcp(
        &self,
        service_id: String,
        endpoints: Vec<NetworkEndpoint>,
        publication: Publication,
        instance_certificate: Option<DelegationCertificate>,
    ) -> Result<()> {
        let (visibility, registry_certificate) = publication.split()?;
        let instance_certificate = instance_certificate
            .map(|c| c.to_json())
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize instance certificate: {e}"))?;
        let manifest = DeployManifest {
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
                visibility: Some(visibility),
            },
            service_type: ServiceType::Tcp(TcpManifest { endpoints }),
            registry_certificate,
            instance_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_container(
        &self,
        service_id: String,
        image: String,
        ports: Vec<ContainerPortMapping>,
        volumes: Vec<ContainerVolumeMapping>,
        publication: Publication,
        instance_certificate: Option<DelegationCertificate>,
    ) -> Result<()> {
        let (visibility, registry_certificate) = publication.split()?;
        let instance_certificate = instance_certificate
            .map(|c| c.to_json())
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize instance certificate: {e}"))?;
        let manifest = DeployManifest {
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
                visibility: Some(visibility),
            },
            service_type: ServiceType::Container(ContainerManifest {
                source: ArtifactSource::Binary(vec![]),
                hash: None,
                image,
                ports,
                volumes,
            }),
            registry_certificate,
            instance_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    /// The instance signing key the substrate would derive for `service_id`
    /// under this client's own identity -- answerable before the service is
    /// deployed (ADR-0020 §3), which is what lets a member master be
    /// certified without the substrate ever holding it.
    pub async fn instance_identity(&self, service_id: &str) -> Result<InstanceIdentity> {
        let params = serde_json::to_value((service_id,))?;
        let res = self.request("orchestrator", "resolve-instance-identity", params).await?;
        serde_json::from_value(res.result)
            .map_err(|e| anyhow::anyhow!("Failed to parse instance identity response: {e}"))
    }

    pub async fn signing_identity(&self, service_id: &str) -> Result<SigningIdentityInfo> {
        let params = serde_json::to_value((service_id,))?;
        let res = self.request("signing", "identity", params).await?;
        serde_json::from_value(res.result)
            .map_err(|e| anyhow::anyhow!("Failed to parse signing identity response: {e}"))
    }

    pub async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<()> {
        let params = serde_json::to_value((plan,))?;
        let res = self.request("orchestrator", "deploy-plan", params).await?;
        if res.result == serde_json::json!({"status": "deployed_plan"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment of plan failed: {:?}", res.result))
        }
    }

    /// `generation` is checked against the app instance's recorded
    /// management stamp when the service has one (M05A A5a, ADR-0021 §4);
    /// send 0 for a standalone service.
    pub async fn undeploy(&self, service_id: String, generation: u64) -> Result<()> {
        let params = serde_json::to_value((service_id, generation))?;
        let res = self.request("orchestrator", "undeploy", params).await?;
        if res.result == serde_json::json!({"status": "undeployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Undeployment failed: {:?}", res.result))
        }
    }

    /// Epoch-guarded binding write (M05A A5a, ADR-0021 §3) -- the only
    /// path that changes a dependent's resolution without redeploying it.
    /// One outcome per binding, in the order sent.
    pub async fn write_bindings(&self, write: BindingWrite) -> Result<Vec<BindingWriteOutcome>> {
        let params = serde_json::to_value((write,))?;
        let res = self.request("orchestrator", "write-bindings", params).await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Restart a deployed service in place, without reinstalling it (M05A
    /// A5's bounded remediation). `generation` is checked against the app
    /// instance's recorded management stamp when the service has one;
    /// send 0 for a standalone service.
    pub async fn restart(&self, service_id: String, generation: u64) -> Result<()> {
        let params = serde_json::to_value((service_id, generation))?;
        let res = self.request("orchestrator", "restart", params).await?;
        if res.result == serde_json::json!({"status": "restarted"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Restart failed: {:?}", res.result))
        }
    }

    /// Install a freshly-issued instance certificate on an already-deployed
    /// service, without reinstalling it (M05A A5's unattended renewal).
    /// `generation` follows `restart`'s rule; send 0 for a standalone
    /// service.
    pub async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
    ) -> Result<()> {
        let params = serde_json::to_value((service_id, generation, instance_certificate))?;
        let res = self.request("orchestrator", "renew-cert", params).await?;
        if res.result == serde_json::json!({"status": "cert_renewed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Certificate renewal failed: {:?}", res.result))
        }
    }

    /// Clear an app instance's management stamp (M05A A5a §0.24):
    /// `supervisor_did` back to `None`, `generation` back to 0. Without
    /// this, an adopted instance can never be hand-deployed again.
    pub async fn release_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
    ) -> Result<()> {
        let params = serde_json::to_value((app_instance_id, generation))?;
        let res = self.request("orchestrator", "release-app-instance", params).await?;
        if res.result == serde_json::json!({"status": "released"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Releasing the app instance failed: {:?}", res.result))
        }
    }

    /// Every call waiting in `service_id`'s durable proxy outbox.
    pub async fn proxy_outbox(&self, service_id: String) -> Result<Vec<QueuedCallInfo>> {
        let res = self
            .request("orchestrator", "proxy-outbox", serde_json::to_value((service_id,))?)
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Every dead letter `service_id`'s durable proxy outbox holds.
    pub async fn proxy_dead_letters(&self, service_id: String) -> Result<Vec<DeadLetterInfo>> {
        let res = self
            .request("orchestrator", "proxy-dead-letters", serde_json::to_value((service_id,))?)
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Re-enqueues one dead letter. It never executes inline.
    pub async fn proxy_replay(&self, service_id: String, dead_letter_id: u64) -> Result<()> {
        let params = serde_json::to_value((service_id, dead_letter_id))?;
        let res = self.request("orchestrator", "proxy-replay", params).await?;
        if res.result == serde_json::json!({"status": "replayed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Replay failed: {:?}", res.result))
        }
    }

    /// Every saga `service_id`'s own log holds, oldest first.
    pub async fn sagas(&self, service_id: String) -> Result<Vec<SagaInfo>> {
        let res =
            self.request("orchestrator", "sagas", serde_json::to_value((service_id,))?).await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Re-arms a `failed` saga back to `compensating`. It never walks
    /// inline.
    pub async fn saga_compensate(&self, service_id: String, saga_id: String) -> Result<()> {
        let params = serde_json::to_value((service_id, saga_id))?;
        let res = self.request("orchestrator", "saga-compensate", params).await?;
        if res.result == serde_json::json!({"status": "compensating"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Saga compensate failed: {:?}", res.result))
        }
    }

    /// Forces this substrate to publish its own endpoint record and every
    /// hosted service's record to its community registry right now,
    /// instead of waiting for the next hourly heartbeat -- e.g. after a
    /// registry this node's records were wiped from (an in-memory registry
    /// that itself restarted) comes back up.
    pub async fn republish(&self) -> Result<()> {
        let res = self.request("orchestrator", "republish", serde_json::json!({})).await?;
        if res.result == serde_json::json!({"status": "republished"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Republish failed: {:?}", res.result))
        }
    }

    pub async fn list_svcs(&self) -> Result<Vec<DeployedService>> {
        let res = self.request("orchestrator", "list", serde_json::json!({})).await?;
        let services: Vec<DeployedService> = serde_json::from_value(res.result)?;
        Ok(services)
    }

    /// A supervisor's poll: per-instance status for `service_ids`, or for
    /// every service this client may see when the list is empty (M05A A4).
    pub async fn status(&self, service_ids: Vec<String>) -> Result<SubstrateStatus> {
        let res = self
            .request("orchestrator", "status", serde_json::json!({ "service_ids": service_ids }))
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// `status`'s `node` field alone (A4-06), with none of `status`'s
    /// per-service work -- for a caller that wants only what this node is,
    /// not what is running on it (e.g. `app deploy`'s preflight, D-A4-15).
    /// `None` for a caller without node-wide `orchestrator/status`
    /// (D-A4-18), the same as `status`'s own `node` field.
    pub async fn node_facts(&self) -> Result<Option<NodeFacts>> {
        let res = self.request("orchestrator", "node-facts-only", serde_json::json!({})).await?;
        Ok(serde_json::from_value(res.result)?)
    }

    pub async fn passthrough(
        &self,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut TcpStream,
        delegation: Option<&DelegationCertificate>,
    ) -> Result<()> {
        let conn_wrapper = self.connection.as_ref().context("Not connected")?.clone();
        Self::passthrough_with_conn(
            conn_wrapper,
            &self.service_id,
            interface_name,
            initial_bytes,
            tcp_stream,
            &self.identity,
            delegation,
        )
        .await
    }

    pub async fn passthrough_with_conn(
        conn_wrapper: TransportConnection,
        service_id: &str,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut TcpStream,
        identity: &Identity,
        delegation: Option<&DelegationCertificate>,
    ) -> Result<()> {
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, recv) = conn.open_bi().await?;

                // Use HTTP transport for passthrough of raw requests. The node
                // pubkey is set along with any optional routing delegation so this
                // connection presents caller identity to the downstream service.
                let preamble = RoutePreamble {
                    transport: RouteTransport::Http,
                    protocol: RouteProtocol::JsonRpc,
                    interface: interface_name.to_string(),
                    service_id: service_id.to_string(),
                    enc: None,
                    pubkey: Some(hex::encode(identity.public_key().to_bytes())),
                    delegation: delegation.cloned(),
                    ucan: None,
                    dir: None,
                }
                .to_preamble_line();
                send.write_all(preamble.as_bytes()).await?;

                send.write_all(initial_bytes).await?;

                let mut joined_iroh = io::join(recv, send);
                if let Err(e) = io::copy_bidirectional(tcp_stream, &mut joined_iroh).await {
                    debug!("Bidirectional copy error between TCP and Iroh: {}", e);
                }

                Ok(())
            }
        }
    }

    pub async fn lookup_registry(&self) -> Result<SignedEndpointInfo> {
        self.registry_client().lookup(&self.service_id, true).await
    }

    pub async fn inject_kek(&self, kek_hex: String) -> Result<()> {
        let params = serde_json::to_value((kek_hex,))?;
        let res = self.request("security", "inject-kek", params).await?;
        if res.result == serde_json::json!({"status": "injected"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("KEK injection failed: {:?}", res.result))
        }
    }

    pub async fn rotate_kek(&self, new_kek_hex: String) -> Result<()> {
        let params = serde_json::to_value((new_kek_hex,))?;
        let res = self.request("security", "rotate-kek", params).await?;
        if res.result == serde_json::json!({"status": "rotated"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("KEK rotation failed: {:?}", res.result))
        }
    }

    pub async fn set_secret(&self, service_id: String, key: String, value: Vec<u8>) -> Result<()> {
        let params = serde_json::to_value((service_id, key, value))?;
        let res = self.request("security", "set-secret", params).await?;
        if res.result == serde_json::json!({"status": "secret_set"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Secret setting failed: {:?}", res.result))
        }
    }
}

/// A safety net, not the primary close path: `Drop` cannot `.await`, so it
/// cannot run `shutdown`'s graceful QUIC handshake directly. This backstops
/// a caller that forgets to call `shutdown` explicitly -- exactly what
/// `SupervisorService::handle_adopt` did before it was fixed to call
/// `shutdown_clients` (N2, M05A Slice A5b review round 2) -- by closing the
/// endpoint in the background instead of leaving iroh to abort it
/// ungracefully. Callers that need to *know* the close finished (a test, or
/// an RPC handler that should not return until its outbound connections are
/// torn down) should still call `shutdown` explicitly; this only covers the
/// case nothing does.
impl Drop for SyneroymClient {
    fn drop(&mut self) {
        if let Some(TransportConnection::Iroh { endpoint, .. }) = self.connection.take()
            && tokio::runtime::Handle::try_current().is_ok()
        {
            drop(spawn_background_close(endpoint));
        }
    }
}

#[cfg(test)]
mod frame_size_tests {
    use super::*;

    #[test]
    fn a_request_within_the_frame_limit_is_accepted() {
        check_frame_size("orchestrator.deploy", &vec![0u8; 1024]).unwrap();
    }

    #[test]
    fn a_request_right_at_the_limit_is_accepted() {
        check_frame_size("orchestrator.deploy", &vec![0u8; framing::MAX_FRAME_SIZE as usize])
            .unwrap();
    }

    #[test]
    fn a_request_one_byte_over_the_limit_is_refused_naming_the_method_and_both_sizes() {
        let err = check_frame_size(
            "orchestrator.deploy",
            &vec![0u8; framing::MAX_FRAME_SIZE as usize + 1],
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("orchestrator.deploy"), "{msg}");
        assert!(msg.contains(&(framing::MAX_FRAME_SIZE as usize + 1).to_string()), "{msg}");
        assert!(msg.contains(&framing::MAX_FRAME_SIZE.to_string()), "{msg}");
    }
}

#[cfg(test)]
mod publication_split_tests {
    use syneroym_core::dht_registry::{EndpointInfo, EndpointType};
    use syneroym_identity::Identity;

    use super::*;

    fn sample_record() -> SignedEndpointInfo {
        let identity = Identity::generate().unwrap();
        let service_id = syneroym_identity::substrate::derive_did_key(&identity.public_key());
        EndpointInfo {
            service_id,
            substrate_id: "did:key:z6Mksub".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: 9999999999,
            generation: 0,
        }
        .sign(&identity)
        .unwrap()
    }

    #[test]
    fn private_splits_to_no_certificate() {
        let (visibility, cert) = Publication::Private.split().unwrap();
        assert_eq!(visibility, Visibility::Private);
        assert!(cert.is_none());
    }

    #[test]
    fn public_and_internal_split_to_a_serialized_certificate() {
        let (visibility, cert) = Publication::Public(sample_record()).split().unwrap();
        assert_eq!(visibility, Visibility::Public);
        assert!(cert.is_some());

        let (visibility, cert) = Publication::Internal(sample_record()).split().unwrap();
        assert_eq!(visibility, Visibility::Internal);
        assert!(cert.is_some());
    }
}

#[cfg(test)]
mod new_with_record_tests {
    use syneroym_core::dht_registry::{EndpointInfo, EndpointType};
    use syneroym_identity::Identity;

    use super::*;

    #[tokio::test]
    async fn new_with_record_verifies_signature_and_sets_fields() {
        // `verify()` resolves the signing key from `service_id` itself
        // (the registry's own admission rule), so it must be the signer's
        // real derived DID, not a placeholder string.
        let identity = Identity::generate().unwrap();
        let service_id = syneroym_identity::substrate::derive_did_key(&identity.public_key());
        let substrate_id = "did:key:z6Mksub".to_string();
        let record = EndpointInfo {
            service_id: service_id.clone(),
            substrate_id: substrate_id.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("my-private-svc".to_string()),
            is_private: true,
            ttl: None,
            not_after: 9999999999,
            generation: 0,
        }
        .sign(&identity)
        .unwrap();

        let client =
            SyneroymClient::new_with_record(record.clone(), "http://127.0.0.1:9999".to_string())
                .unwrap();
        assert_eq!(client.service_id(), service_id);
        assert_eq!(client.registry_lookup_override.as_deref(), Some(substrate_id.as_str()));
        assert!(client.provided_mechanisms.is_none());

        // Tampered record fails verification
        let mut tampered = record;
        tampered.info.service_id = "did:key:z6Mktampered".to_string();
        assert!(
            SyneroymClient::new_with_record(tampered, "http://127.0.0.1:9999".to_string()).is_err()
        );
    }
}
