//! Iroh network client/server channel routing
//!
//! Adapts Iroh peer endpoints to the connection router, handling secure tunnel
//! connection requests.

use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    io,
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::Context;
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, QuicTransportConfig, RecvStream, SendStream},
};
use syneroym_core::{
    config::{IrohParentConfig, RetryPolicy},
    dht_registry::{EndpointMechanism, RegistryClient},
    retry::retry_with_backoff,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::stop_signal::StopSignal;

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

pub async fn build_iroh_endpoint(
    relay_url: Option<RelayUrl>,
    secret_key: Option<SecretKey>,
    idle_timeout_secs: Option<u64>,
) -> anyhow::Result<Endpoint> {
    let mut builder = Endpoint::empty_builder().relay_mode(relay_mode(relay_url));
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

/// Resolves a service's Iroh `EndpointAddr` via the community registry / DHT.
/// Factored out of `route_handler/io.rs`'s registry-miss relay path so
/// `ProxyRouter::invoke_remote`'s outbound hop can share the exact same
/// resolution logic instead of re-implementing it.
pub async fn resolve_iroh_addr(
    registry_client: &RegistryClient,
    service_id: &str,
) -> anyhow::Result<EndpointAddr> {
    let info = registry_client.lookup(service_id, true).await?;
    for mech in info.info.mechanisms {
        if let EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } = mech {
            let mut addr: EndpointAddr = serde_json::from_slice(&endpoint_addr_bytes)?;
            if let Some(r_url_str) = relay_url
                && let Ok(relay_url) = r_url_str.parse::<RelayUrl>()
            {
                addr = addr.with_relay_url(relay_url);
            }
            return Ok(addr);
        }
    }
    Err(anyhow::anyhow!("No valid Iroh mechanism found for service '{service_id}' in registry"))
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
mod tests {
    use super::*;

    #[test]
    fn relay_mode_without_url_is_an_empty_custom_map() {
        let mode = relay_mode(None);
        assert!(matches!(mode, RelayMode::Custom(_)));
        assert_eq!(mode.relay_map().len(), 0);
    }

    #[test]
    fn relay_mode_with_url_has_only_that_relay() {
        let url: RelayUrl = "http://127.0.0.1:7964".parse().unwrap();
        let mode = relay_mode(Some(url.clone()));
        assert!(matches!(mode, RelayMode::Custom(_)));
        assert_eq!(mode.relay_map().len(), 1);
        assert!(mode.relay_map().contains(&url));
    }

    #[test]
    fn parse_relay_url_accepts_http_and_https() {
        let http = parse_relay_url("test", "http://example.org:7964").unwrap();
        assert_eq!(http.as_str(), "http://example.org:7964/");

        let https = parse_relay_url("test", "https://example.org:7964").unwrap();
        assert_eq!(https.as_str(), "https://example.org:7964/");
    }

    #[test]
    fn parse_relay_url_rejects_a_value_without_scheme() {
        let err = parse_relay_url("parent_coordinator.iroh.url", "localhost:7964")
            .unwrap_err()
            .to_string();
        assert!(err.contains("parent_coordinator.iroh.url"), "expected setting name: {err}");
        assert!(err.contains("localhost:7964"), "expected bad value: {err}");
    }

    #[test]
    fn parse_relay_url_rejects_other_schemes() {
        let err = parse_relay_url("test", "ftp://x").unwrap_err().to_string();
        assert!(err.contains("must be an http:// or https:// URL"), "expected error: {err}");
    }

    #[test]
    fn parse_relay_url_rejects_garbage() {
        let err = parse_relay_url("test", "not a url").unwrap_err().to_string();
        assert!(err.contains("test = \"not a url\""), "expected error: {err}");
    }

    #[test]
    fn parent_relay_url_is_none_without_url() {
        let config = IrohParentConfig { url: None };
        assert_eq!(parent_relay_url(&config).unwrap(), None);
    }
}
