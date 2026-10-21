//! Syneroym client for connecting to substrates, sending requests, and
//! subscribing to topics.

use std::{
    fmt::{self, Debug, Formatter},
    sync::{Arc, Mutex, OnceLock, PoisonError, atomic::AtomicUsize},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use ed25519_dalek::VerifyingKey;
use iroh::{Endpoint, endpoint::Connection};
use serde_json::Value;
use syneroym_core::dht_registry::{EndpointMechanism, RegistryClient, SignedEndpointInfo};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_rpc::{
    CapabilityToken, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse,
    MESSAGING_MESSAGE_METHOD, MessagingNotification, framing,
};
use tokio::{
    io::AsyncWrite,
    sync::{Mutex as AsyncMutex, mpsc},
    task::JoinHandle,
    time,
};
use tracing::debug;

mod host;
mod orchestrator;
mod passthrough;
mod rehome;
mod streams;

pub use passthrough::PassthroughOutcome;
use rehome::{is_bad_host, is_not_hosted_here};

pub(crate) fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Default ceiling for establishing a connection to a single mechanism.
/// Without a bound here, iroh's relay/hole-punch retries can churn
/// indefinitely against an unreachable or overloaded peer, leaving the
/// caller with no way to give up. Override via
/// [`SyneroymClient::with_connect_timeout`].
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The authoritative size check the asset-bundle deploy-time
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
pub(crate) fn check_frame_size(method: &str, req_bytes: &[u8]) -> Result<()> {
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
    pub(crate) provided_mechanisms: Option<Vec<EndpointMechanism>>,
    /// Overrides which DID `connect()` queries the registry under, when it
    /// differs from `service_id` itself. `None` for every constructor
    /// except [`Self::new_with_record`]: a *private* service (ADR-0018 §2)
    /// is never registered, so looking `service_id` up would fail, while
    /// the substrate hosting it always is (it publishes itself on every
    /// heartbeat) -- `new_with_record` sets this to that substrate's own
    /// DID, taken from the signed record, so `connect()` resolves *that*
    /// instead.
    pub(crate) registry_lookup_override: Option<String>,
    /// The host DID given by `with_host` or by the record of
    /// `new_with_record`. Used when no registry lookup named the host.
    pub(crate) expected_host: Option<String>,
    connection: Mutex<Option<TransportConnection>>,
    connect_timeout: Duration,
    identity: Identity,
    /// A verified UCAN capability chain to present on every outbound
    /// preamble, set via [`Self::with_ucan`]. `None` by default -- callers
    /// that don't hold one still get the self-asserted-identity admission.
    caller_ucan: Option<CapabilityToken>,
    /// A certificate that lets this client's key act as its master, sent on
    /// every request preamble. Set via [`Self::with_delegation`].
    caller_delegation: Option<DelegationCertificate>,
    /// Handles for endpoints from failed/timed-out connect attempts (see
    /// `spawn_background_close`), reaped by [`Self::shutdown`]. Unlike the
    /// `Drop` safety net, a retry loop such as `wait_for_ready` can spawn
    /// many of these against a single client; leaving them untracked lets
    /// the runtime abort them mid-close on teardown instead of letting them
    /// finish, which is what was producing iroh's "Endpoint dropped without
    /// calling `Endpoint::close`" warning.
    pending_closes: Mutex<Vec<JoinHandle<()>>>,
    rehome_lock: AsyncMutex<()>,
    rehome_count: AtomicUsize,
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
            .field("expected_host", &self.expected_host)
            .field("connection", &self.connection.lock().map(|c| c.clone()).ok())
            .field("connect_timeout", &self.connect_timeout)
            .field("identity", &self.identity)
            .field("caller_ucan", &self.caller_ucan)
            .field("caller_delegation", &self.caller_delegation)
            .field("pending_closes", &self.pending_closes.lock().map(|p| p.len()).unwrap_or(0))
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

#[derive(Clone)]
pub enum TransportConnection {
    Iroh {
        /// The endpoint must be kept alive for the duration of the connection.
        /// Dropping it closes the underlying QUIC socket, terminating all
        /// streams.
        endpoint: Endpoint,
        conn: Connection,
        /// The node key expected to host the target. When it is not the
        /// key of the node this connection reached, the connection is
        /// through a forwarder, and the identity goes in the encrypted line.
        host_key: Option<VerifyingKey>,
    },
}

impl TransportConnection {
    #[must_use]
    pub fn stable_id(&self) -> usize {
        match self {
            Self::Iroh { conn, .. } => conn.stable_id(),
        }
    }
}

impl Debug for TransportConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Iroh { conn, host_key, .. } => {
                let host_str =
                    host_key.as_ref().map_or_else(|| "-".to_string(), substrate::derive_did_key);
                f.debug_struct("TransportConnection::Iroh")
                    .field("endpoint", &"iroh::Endpoint")
                    .field("conn", &format!("{:?}", conn.remote_id()))
                    .field("host_key", &host_str)
                    .finish()
            }
        }
    }
}

/// A live `messaging/subscribe` stream: `.recv()` yields `(topic, payload)`
/// pairs as the broker delivers them. Dropping it drops the send half of
/// the underlying bidirectional stream, which the router-side handler
/// observes as the client having gone away (close-as-unsubscribe).
pub struct MessageStream {
    receiver: mpsc::Receiver<(String, Vec<u8>)>,
    send: Box<dyn AsyncWrite + Unpin + Send>,
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
    /// Dropping or stopping closes the send half.
    /// `.recv()` remains usable afterward and resolves to `None` once the
    /// router's own writer task exits in response and closes its side.
    pub async fn stop(&mut self) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        self.send.shutdown().await.map_err(Into::into)
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
            expected_host: None,
            connection: Mutex::new(None),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity: generate_ephemeral_identity(),
            caller_ucan: None,
            caller_delegation: None,
            pending_closes: Mutex::new(Vec::new()),
            rehome_lock: AsyncMutex::new(()),
            rehome_count: AtomicUsize::new(0),
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
            expected_host: None,
            connection: Mutex::new(None),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity: generate_ephemeral_identity(),
            caller_ucan: None,
            caller_delegation: None,
            pending_closes: Mutex::new(Vec::new()),
            rehome_lock: AsyncMutex::new(()),
            rehome_count: AtomicUsize::new(0),
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
        let expected_host = Some(substrate_id.clone());
        let provided_mechanisms =
            if !record.info.mechanisms.is_empty() { Some(record.info.mechanisms) } else { None };
        let registry_lookup_override =
            if provided_mechanisms.is_none() { Some(substrate_id) } else { None };
        Ok(Self {
            service_id,
            registry_url,
            provided_mechanisms,
            registry_lookup_override,
            expected_host,
            connection: Mutex::new(None),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity: generate_ephemeral_identity(),
            caller_ucan: None,
            caller_delegation: None,
            pending_closes: Mutex::new(Vec::new()),
            rehome_lock: AsyncMutex::new(()),
            rehome_count: AtomicUsize::new(0),
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
            expected_host: None,
            connection: Mutex::new(None),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            identity,
            caller_ucan: None,
            caller_delegation: None,
            pending_closes: Mutex::new(Vec::new()),
            rehome_lock: AsyncMutex::const_new(()),
            rehome_count: AtomicUsize::new(0),
            enable_dht: true,
            registry_client: OnceLock::new(),
        }
    }

    /// Sets an explicit caller identity for this client.
    #[must_use]
    pub fn with_identity(mut self, identity: Identity) -> Self {
        self.identity = identity;
        self
    }

    /// The DID of the node that hosts the target, for a client that dials
    /// given mechanisms and has no record: the client checks the handshake of a
    /// forwarder against this node's key.
    #[must_use]
    pub fn with_host(mut self, host_did: impl Into<String>) -> Self {
        self.expected_host = Some(host_did.into());
        self
    }

    /// Overrides the default per-mechanism connect deadline (see
    /// [`DEFAULT_CONNECT_TIMEOUT`]).
    #[must_use]
    pub const fn with_connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    /// Attaches a verified UCAN capability chain to present
    /// on every outbound preamble opened by this client.
    #[must_use]
    pub fn with_ucan(mut self, caller_ucan: CapabilityToken) -> Self {
        self.caller_ucan = Some(caller_ucan);
        self
    }

    /// Attaches a delegation certificate proving this client acts on behalf
    /// of its member master, sent on every outbound preamble.
    #[must_use]
    pub fn with_delegation(mut self, cert: DelegationCertificate) -> Self {
        self.caller_delegation = Some(cert);
        self
    }

    /// The delegation certificate attached to this client, if any.
    #[must_use]
    pub const fn caller_delegation(&self) -> Option<&DelegationCertificate> {
        self.caller_delegation.as_ref()
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
        if self.connection.get_mut().unwrap_or_else(PoisonError::into_inner).is_some() {
            return Ok(());
        }

        debug!("Connecting to {} via registry or provided mechanisms", self.service_id);

        let lookup_target = self.registry_lookup_override.as_deref().unwrap_or(&self.service_id);
        let (mechanisms, looked_up_host) = if let Some(m) = &self.provided_mechanisms {
            (m.clone(), None)
        } else if !self.registry_url.is_empty() {
            let info = self.registry_client().lookup(lookup_target, true).await?.info;
            if self.registry_lookup_override.is_none() {
                // The lookup might have been done by an alias. Update service_id to the
                // canonical DID.
                self.service_id = info.service_id;
            }
            (info.mechanisms, Some(info.substrate_id))
        } else {
            return Err(anyhow::anyhow!("No registry URL or mechanisms provided"));
        };

        let conn = self.dial(mechanisms, looked_up_host.as_deref()).await?;
        *self.connection.get_mut().unwrap_or_else(PoisonError::into_inner) = Some(conn);
        Ok(())
    }

    pub async fn connect_with_mechanisms(
        &mut self,
        mechanisms: Vec<EndpointMechanism>,
    ) -> Result<()> {
        let conn = self.dial(mechanisms, None).await?;
        *self.connection.get_mut().unwrap_or_else(PoisonError::into_inner) = Some(conn);
        Ok(())
    }

    pub(crate) fn push_pending_close(&self, handle: JoinHandle<()>) {
        let mut list = self.pending_closes.lock().unwrap_or_else(PoisonError::into_inner);
        list.retain(|h| !h.is_finished());
        list.push(handle);
    }

    pub fn disconnect(&mut self) {
        if let Some(TransportConnection::Iroh { endpoint, .. }) =
            self.connection.get_mut().unwrap_or_else(PoisonError::into_inner).take()
        {
            let handle = spawn_background_close(endpoint);
            let list = self.pending_closes.get_mut().unwrap_or_else(PoisonError::into_inner);
            list.retain(|h| !h.is_finished());
            list.push(handle);
        }
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
        if let Some(TransportConnection::Iroh { endpoint, .. }) =
            self.connection.get_mut().unwrap_or_else(PoisonError::into_inner).take()
        {
            endpoint.close().await;
        }
        let handles: Vec<_> =
            std::mem::take(self.pending_closes.get_mut().unwrap_or_else(PoisonError::into_inner));
        for handle in handles {
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
        self.connection.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[must_use]
    pub fn current_stable_id(&self) -> Option<usize> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(TransportConnection::stable_id)
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

    pub async fn request_raw(
        &self,
        interface_name: &str,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse> {
        let (first_try, used_conn) = self.request_once(interface_name, &request).await;
        match first_try {
            Ok(res) => Ok(res),
            Err(e) if (is_not_hosted_here(&e) || is_bad_host(&e)) && self.can_rehome() => {
                if let Some(used) = used_conn {
                    if let Err(rehome_err) = self.rehome(&used).await {
                        tracing::debug!("rehome failed: {rehome_err}; returning original refusal");
                        return Err(e);
                    }
                    let (second_try, _) = self.request_once(interface_name, &request).await;
                    second_try
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn subscribe_once(
        &self,
        interface: &str,
        topic: &str,
        request: &JsonRpcRequest,
    ) -> Result<MessageStream, Box<(anyhow::Error, Option<TransportConnection>)>> {
        let conn_wrapper = self.connection();
        let stream = self
            .open_request_stream(interface, request)
            .await
            .map_err(|e| Box::new((e, conn_wrapper.clone())))?;
        let syneroym_router::ReaderWriter { reader: mut recv, writer: send } = stream;

        let ack_frame = framing::read_frame(&mut recv)
            .await
            .map_err(|e| Box::new((e, conn_wrapper.clone())))?;
        if ack_frame.is_empty() {
            return Err(Box::new((
                anyhow::anyhow!("Empty ack for subscribe on topic {topic} (interface {interface})"),
                conn_wrapper,
            )));
        }
        let ack: JsonRpcResponse = match serde_json::from_slice(&ack_frame) {
            Ok(res) => res,
            Err(parse_err) => {
                return match serde_json::from_slice::<JsonRpcErrorResponse>(&ack_frame) {
                    Ok(err_res) => Err(Box::new((err_res.error.into(), conn_wrapper))),
                    Err(_) => Err(Box::new((parse_err.into(), conn_wrapper))),
                };
            }
        };
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
        match self.subscribe_once(interface, topic, &request).await {
            Ok(stream) => Ok(stream),
            Err(boxed) => {
                let (e, used_opt) = *boxed;
                if let Some(used) = used_opt
                    .filter(|_| (is_not_hosted_here(&e) || is_bad_host(&e)) && self.can_rehome())
                {
                    if let Err(rehome_err) = self.rehome(&used).await {
                        debug!("subscribe rehome failed: {rehome_err}; returning original refusal");
                        return Err(e);
                    }
                    return self.subscribe_once(interface, topic, &request).await.map_err(|b| b.0);
                }
                Err(e)
            }
        }
    }

    pub async fn lookup_registry(&self) -> Result<SignedEndpointInfo> {
        self.registry_client().lookup(&self.service_id, true).await
    }
}

/// A safety net, not the primary close path: `Drop` cannot `.await`, so it
/// cannot run `shutdown`'s graceful QUIC handshake directly. This backstops
/// a caller that forgets to call `shutdown` explicitly -- exactly what
/// `SupervisorService::handle_adopt` did before it was fixed to call
/// `shutdown_clients` -- by closing the
/// endpoint in the background instead of leaving iroh to abort it
/// ungracefully. Callers that need to *know* the close finished (a test, or
/// an RPC handler that should not return until its outbound connections are
/// torn down) should still call `shutdown` explicitly; this only covers the
/// case nothing does.
impl Drop for SyneroymClient {
    fn drop(&mut self) {
        if let Some(TransportConnection::Iroh { endpoint, .. }) =
            self.connection.get_mut().unwrap_or_else(PoisonError::into_inner).take()
            && tokio::runtime::Handle::try_current().is_ok()
        {
            drop(spawn_background_close(endpoint));
        }
    }
}
