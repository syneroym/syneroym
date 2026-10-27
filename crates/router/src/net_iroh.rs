//! Iroh network client/server channel routing
//!
//! Adapts Iroh peer endpoints to the connection router, handling secure tunnel
//! connection requests.

use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::Context;
use ed25519_dalek::VerifyingKey;
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey,
    endpoint::{Builder, Connection, QuicTransportConfig, RecvStream, SendStream},
};
use rustls::crypto::ring;
use syneroym_core::{
    config::{IrohParentConfig, RetryPolicy},
    dht_registry::{
        EndpointMailbox, EndpointMechanism, EndpointType, RegistryClient, SignedEndpointInfo,
    },
    retry::retry_with_backoff,
};
use syneroym_identity::substrate;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time,
};

use crate::{peer_records::PeerRecordStore, stop_signal::StopSignal};

mod close;

pub use close::{ENDPOINT_CLOSE_LIMIT, close_within};

pub struct IrohStream {
    send: SendStream,
    recv: RecvStream,
    conn: Option<Connection>,
}

impl Debug for IrohStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohStream")
            .field("send", &"iroh::endpoint::SendStream")
            .field("recv", &"iroh::endpoint::RecvStream")
            .field("conn", &self.conn.is_some())
            .finish()
    }
}

impl IrohStream {
    #[must_use]
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv, conn: None }
    }

    #[must_use]
    pub fn with_conn(mut self, conn: Connection) -> Self {
        self.conn = Some(conn);
        self
    }
}

impl AsyncRead for IrohStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for IrohStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.send).poll_write(cx, buf).map_err(io::Error::other)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}

impl StopSignal for IrohStream {
    fn stop_signal(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let stopped = self.send.stopped();
        Box::pin(async move {
            let _ = stopped.await;
        })
    }
}

/// Parses a relay URL from a config setting. A relay URL must be `http`
/// or `https` and have a host. The error names the setting and the value,
/// because a bad value stops startup.
///
/// # Errors
/// When `raw` is not such a URL.
pub fn parse_relay_url(setting: &str, raw: &str) -> anyhow::Result<RelayUrl> {
    let url = raw
        .parse::<RelayUrl>()
        .with_context(|| format!("{setting} = {raw:?} is not a valid URL"))?;
    // `RelayUrl` derefs to `url::Url`.
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "{setting} = {raw:?} must be an http:// or https:// URL with a host"
    );
    Ok(url)
}

/// The parent relay of this node: `parent_coordinator.iroh.url`, parsed.
/// `Ok(None)` when it is not set.
///
/// # Errors
/// When it is set and not a valid relay URL.
pub fn parent_relay_url(config: &IrohParentConfig) -> anyhow::Result<Option<RelayUrl>> {
    config.url.as_deref().map(|raw| parse_relay_url("parent_coordinator.iroh.url", raw)).transpose()
}

/// Only the given relay, or an empty map. Never the n0 preset: an endpoint
/// with an empty map can still dial a peer through that peer's relay, and
/// can get a relay later with `Endpoint::insert_relay`.
/// `RelayMode::Disabled` would build no relay transport at all.
fn relay_mode(relay_url: Option<RelayUrl>) -> RelayMode {
    RelayMode::Custom(relay_url.map_or_else(RelayMap::empty, RelayMap::from))
}

/// An endpoint builder with no relay and no address lookup. It sets only the
/// TLS crypto provider (ring, iroh's own default), which iroh requires. Every
/// endpoint starts here: the n0 presets add n0's public relays and DNS
/// lookup, which would show metadata to a third party.
#[must_use]
pub fn empty_endpoint_builder() -> Builder {
    Builder::empty().crypto_provider(Arc::new(ring::default_provider()))
}

pub async fn build_iroh_endpoint(
    relay_url: Option<RelayUrl>,
    secret_key: Option<SecretKey>,
    idle_timeout_secs: Option<u64>,
) -> anyhow::Result<Endpoint> {
    let mut builder = empty_endpoint_builder().relay_mode(relay_mode(relay_url));
    if let Some(sk) = secret_key {
        builder = builder.secret_key(sk);
    }

    if let Some(timeout) = idle_timeout_secs {
        let mut builder_cfg = QuicTransportConfig::builder();
        builder_cfg = builder_cfg.max_idle_timeout(Some(Duration::from_secs(timeout).try_into()?));
        builder = builder.transport_config(builder_cfg.build());
    }

    let endpoint = builder.bind().await?;
    Ok(endpoint)
}

/// How long a sender waits for the address of a peer that has a mailbox
/// before it treats the peer as asleep. A receiver that sleeps has no
/// substrate record, so the registry lookup falls back to the DHT, which
/// can take many seconds; the sender must reach the mailbox quickly instead.
pub const MAILBOX_PEER_ADDR_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

/// What a sender learns about a peer from its records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPeer {
    /// How to reach the peer now. `None` when its substrate record cannot
    /// be found, which is normal for a receiver that sleeps.
    pub addr: Option<EndpointAddr>,
    /// The peer's mailbox, from its own signed record.
    pub mailbox: Option<EndpointMailbox>,
    /// The node key of the host the record names (`substrate_id`). A client
    /// that dials another node checks the handshake against it.
    pub host_key: Option<VerifyingKey>,
}

/// The Iroh address in `mechanisms`, if any. An Iroh entry that cannot be
/// parsed is an error.
fn iroh_addr_from_mechanisms(
    mechanisms: Vec<EndpointMechanism>,
) -> anyhow::Result<Option<EndpointAddr>> {
    for mech in mechanisms {
        if let EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } = mech {
            let mut addr: EndpointAddr = serde_json::from_slice(&endpoint_addr_bytes)?;
            if let Some(r_url_str) = relay_url
                && let Ok(relay_url) = r_url_str.parse::<RelayUrl>()
            {
                addr = addr.with_relay_url(relay_url);
            }
            return Ok(Some(addr));
        }
    }
    Ok(None)
}

/// How long a sender waits for the registry when it already holds a stored
/// record with a mailbox for the peer. That record can answer at once, and a
/// registry that does not answer would otherwise add its full timeout and a
/// DHT try to the short direct attempt.
pub const MAILBOX_PEER_RECORD_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

/// The service record of `service_id`: from the registry, which is also
/// remembered, else from the peer record store. `lookup` gives the same
/// error for "registry down" and "not found", and a stored record is the
/// right answer to both. When the store already holds a record with a
/// mailbox, the registry gets only a short time to answer.
async fn record_of(
    registry_client: &RegistryClient,
    peer_records: Option<&PeerRecordStore>,
    service_id: &str,
) -> anyhow::Result<SignedEndpointInfo> {
    let stored = match peer_records {
        Some(store) => store.recall(service_id).await,
        None => None,
    };
    let lookup = registry_client.lookup(service_id, false);
    let looked_up = if stored.is_some() {
        time::timeout(MAILBOX_PEER_RECORD_LOOKUP_TIMEOUT, lookup)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("registry lookup of '{service_id}' timed out")))
    } else {
        lookup.await
    };
    match looked_up {
        Ok(record) => {
            if let Some(store) = peer_records
                && let Err(e) = store.remember(&record).await
            {
                tracing::warn!("could not keep the record of '{service_id}': {e}");
            }
            Ok(record)
        }
        Err(e) => stored.ok_or(e),
    }
}

/// The mechanisms of the substrate that hosts `record`'s service. For a peer
/// with a mailbox the lookup is bounded and a failure gives no mechanisms:
/// the peer may be asleep, and the sender can still deposit.
async fn substrate_mechanisms(
    registry_client: &RegistryClient,
    record: SignedEndpointInfo,
) -> anyhow::Result<Vec<EndpointMechanism>> {
    if record.info.endpoint_type == EndpointType::Substrate {
        return Ok(record.info.mechanisms);
    }
    let lookup = registry_client.lookup(&record.info.substrate_id, false);
    if record.info.mailbox.is_none() {
        return Ok(lookup.await?.info.mechanisms);
    }
    match time::timeout(MAILBOX_PEER_ADDR_LOOKUP_TIMEOUT, lookup).await {
        Ok(Ok(substrate)) => Ok(substrate.info.mechanisms),
        Ok(Err(e)) => {
            tracing::debug!("no substrate record for a peer with a mailbox: {e}");
            Ok(Vec::new())
        }
        Err(_) => {
            tracing::debug!("substrate lookup for a peer with a mailbox timed out");
            Ok(Vec::new())
        }
    }
}

/// Resolves a service or substrate DID. Unlike a full lookup, it keeps the
/// service record's mailbox, and a missing substrate record is not an error
/// when the service has a mailbox: the sender can still deposit. A stored
/// record (`peer_records`) answers when the registry cannot be reached.
pub async fn resolve_peer(
    registry_client: &RegistryClient,
    peer_records: Option<&PeerRecordStore>,
    service_id: &str,
) -> anyhow::Result<ResolvedPeer> {
    let record = record_of(registry_client, peer_records, service_id).await?;
    let mailbox = record.info.mailbox.clone();
    let host_key = substrate::resolve_did_key(&record.info.substrate_id).ok();
    let mechanisms = substrate_mechanisms(registry_client, record).await?;
    Ok(ResolvedPeer { addr: iroh_addr_from_mechanisms(mechanisms)?, mailbox, host_key })
}

/// Resolves a service's Iroh `EndpointAddr` via the community registry / DHT.
/// Factored out of `route_handler/io.rs`'s registry-miss relay path so
/// `ProxyRouter::invoke_remote`'s outbound hop can share the exact same
/// resolution logic instead of re-implementing it.
pub async fn resolve_iroh_addr(
    registry_client: &RegistryClient,
    service_id: &str,
) -> anyhow::Result<EndpointAddr> {
    resolve_peer(registry_client, None, service_id).await?.addr.ok_or_else(|| {
        anyhow::anyhow!("No valid Iroh mechanism found for service '{service_id}' in registry")
    })
}

/// Connects to an Iroh endpoint with exponential backoff retries.
pub async fn connect_with_retry(
    endpoint: &Endpoint,
    node_addr: iroh::EndpointAddr,
    alpn: &[u8],
    retry_policy: &RetryPolicy,
) -> anyhow::Result<Connection> {
    let node_addr_clone = node_addr.clone();
    retry_with_backoff(retry_policy, || {
        let ep = endpoint.clone();
        let addr = node_addr_clone.clone();
        async move { ep.connect(addr, alpn).await }
    })
    .await
    .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests;
