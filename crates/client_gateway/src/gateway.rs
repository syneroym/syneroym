//! HTTP Client Gateway
//!
//! Proxies external client requests into the internal Syneroym network,
//! managing routing, protocol translation, and error boundaries.

use std::{
    fmt::{self, Debug, Formatter},
    fs, str,
    sync::Arc,
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use httparse::{EMPTY_HEADER, Request, Status};
use syneroym_app_orchestration::{LogicalResolver, StaticInventory};
use syneroym_core::{
    config::SubstrateConfig,
    protocol_utils::{ROUTING_KEY_HEADER, TargetHost, parse_target_host},
    util::load_or_generate_node_identity,
};
use syneroym_identity::Identity;
use syneroym_sdk::{
    SyneroymClient,
    topology::{
        AppHostResolver, CredentialWarning, RegistryTier1Lookup, RegistryTopologyFetcher,
        Tier2Fetch, credential_warning,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, oneshot, oneshot::Sender},
};
use tracing::{debug, error, info, warn};

// The node identity this gateway presents as caller DID (M04A Slice B0,
// ADR-0016 §0.5) is loaded by
// `syneroym_core::util::load_or_generate_node_identity` -- the same key file
// `syneroym_substrate::identity::setup_substrate_identity` loads, by the same
// path-resolution rule, so whichever of the gateway or the connection router
// runs first creates the on-disk key and the other loads the same one back.
// Lifted there in S3 (D-S3-6) rather than kept as a private copy here, since
// the WebRTC coordinator needs the identical node identity to satisfy
// `[iam].grant_resolve_to_node_did`.
//
// TODO(post-B0): present the substrate-owner (controller) DID as caller by
// carrying an owner->node DelegationCertificate here (verify_preamble
// already resolves master_did from it). `ControllerAgreement` now exists,
// but that is a mutual attestation binding the two DIDs, not a delegation
// the node can present on the wire -- provisioning a separate owner-signed
// delegation for this purpose is still not built. B0 uses node DID.
// Consequence, now that an unowned substrate fails closed: the gateway
// proxies as a DID that holds nothing node-wide -- harmless today only
// because it proxies to deployed services, never to `orchestrator`/
// `security` (flagged, not fixed; see the deferred backlog).

/// Reads a `CapabilityToken` off disk, the same shape `roymctl`'s own
/// `--ucan` loading uses.
fn read_capability_token(path: &std::path::Path) -> Result<syneroym_rpc::CapabilityToken> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read UCAN token at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("invalid UCAN token JSON at {}", path.display()))
}

#[derive(Debug)]
struct GatewayState {
    registry_url: String,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
    /// The node's own identity (M04A Slice B0, ADR-0016 §0.5) -- presented
    /// as the caller DID for every proxied request. Reconstructed per
    /// downstream `SyneroymClient` from the same key bytes (`Identity` is
    /// deliberately not `Clone`), rather than shared as a single instance.
    identity: Identity,
    /// The app-scoped (`-a…-s…`) host resolver (S3, D-S3-7, D-S3-11):
    /// `sdk::topology::AppHostResolver`, shared with the WebRTC coordinator
    /// rather than reimplemented, since the two are the pair most likely
    /// to drift subtly apart on its D-S3-5 binding checks. Deliberately
    /// not the node's own `LogicalResolver`: `ClientGateway::init` runs
    /// before `setup_connection_router` builds that one, and the two key
    /// spaces are disjoint by type (`AppScope`) anyway.
    app_host_resolver: AppHostResolver,
}

/// `ClientGateway`: Acts as an entry point for local HTTP/WebSocket clients to
/// reach the wider Syneroym network.
///
/// It accepts TCP traffic, reads the HTTP headers to extract the routing target
/// from the `Host` header, and streams the raw bytes over the Syneroym network.
pub struct ClientGateway {
    port: u16,
    state: Arc<GatewayState>,
    shutdown_tx: Option<Sender<()>>,
}

impl Debug for ClientGateway {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientGateway")
            .field("port", &self.port)
            .field("state", &self.state)
            .field("shutdown_tx", &self.shutdown_tx.as_ref().map(|_| "oneshot::Sender"))
            .finish()
    }
}

impl ClientGateway {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing client gateway");

        let port = config.roles.client_gateway.as_ref().map_or(7000, |g| g.http_port);
        let registry_url = config.substrate.registry_url.clone().unwrap_or_default();
        let identity = load_or_generate_node_identity(config)?;

        // D-S3-7: the gateway owns its own resolver and fetcher. It cannot
        // share the node's own `LogicalResolver` -- `ClientGateway::init`
        // runs before `setup_connection_router` constructs that one, an
        // order with a measured startup cost attached -- and the two key
        // spaces are disjoint by type (`AppScope`) anyway.
        let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
        let resolve_ucan_path =
            config.roles.client_gateway.as_ref().and_then(|g| g.resolve_ucan.as_ref());
        let fetcher = if registry_url.is_empty() {
            None
        } else {
            let mut f = RegistryTopologyFetcher::new(registry_url.clone()).with_identity(&identity);
            if let Some(path) = resolve_ucan_path {
                f = f.with_ucan(read_capability_token(path)?);
            }
            match credential_warning(
                resolve_ucan_path.is_some(),
                config.iam.grant_resolve_to_node_did,
            ) {
                Some(CredentialWarning::NeitherConfigured) => warn!(
                    "client gateway has neither `roles.client_gateway.resolve_ucan` nor \
                     `iam.grant_resolve_to_node_did`; app-scoped (-a…-s…) hostnames will be \
                     refused by any supervisor they reach. Unscoped (-s only) hostnames are \
                     unaffected."
                ),
                Some(CredentialWarning::OnlyTheSameNodeGate) => debug!(
                    "client gateway has no `resolve_ucan`; app-scoped hostnames will resolve only \
                     for apps supervised by this node"
                ),
                None => {}
            }
            Some(Box::new(f) as Box<dyn Tier2Fetch>)
        };
        let tier1 = Box::new(RegistryTier1Lookup::new(registry_url.clone()));
        let app_host_resolver = AppHostResolver::new(tier1, fetcher, resolver);

        let state = Arc::new(GatewayState {
            registry_url,
            clients: DashMap::new(),
            identity,
            app_host_resolver,
        });

        Ok(Self { port, state, shutdown_tx: None })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running client gateway on port {}", self.port);
        let state = self.state.clone();

        // TODO: For now, basic security via access from local machine only instead of
        // 0.0.0.0 interface
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;

        let (tx, mut rx) = oneshot::channel();
        self.shutdown_tx = Some(tx);

        loop {
            tokio::select! {
                Ok((stream, _)) = listener.accept() => {
                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state_clone).await {
                            error!("Connection handling error: {}", e);
                        }
                    });
                }
                _ = &mut rx => {
                    break;
                }
            }
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down client gateway");
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Shutdown all cached clients to close their Iroh endpoints gracefully
        for entry in &self.state.clients {
            let mut client = entry.value().lock().await;
            let _ = client.shutdown().await;
        }

        Ok(())
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<GatewayState>) -> Result<()> {
    // Limit header reads to 8 KB — the conventional maximum for HTTP/1.1 headers.
    // Requests with larger headers (e.g. very large JWTs) will receive a 400
    // response.
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    let mut buf = [0u8; MAX_HEADER_BYTES];
    let mut bytes_read = 0;

    // Read enough to find the end of the HTTP headers
    loop {
        let n = stream.read(&mut buf[bytes_read..]).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("Connection closed before headers finished"));
        }
        bytes_read += n;
        debug!("gateway read {} bytes, total {}", n, bytes_read);

        let mut headers = [EMPTY_HEADER; 64];
        let mut req = Request::new(&mut headers);

        match req.parse(&buf[..bytes_read]) {
            Ok(Status::Complete(_header_len)) => {
                let host_header = req
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("host"))
                    .map_or("", |h| str::from_utf8(h.value).unwrap_or(""));
                let target = match parse_target_host(host_header) {
                    Some(t) => t,
                    None => {
                        return write_json_rpc_error(
                            &mut stream,
                            400,
                            "Missing or invalid Host header",
                        )
                        .await;
                    }
                };
                let routing_key: Option<Vec<u8>> = req
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case(ROUTING_KEY_HEADER))
                    .map(|h| h.value.to_vec());

                let (service_id, interface) = match target {
                    TargetHost::Service { lookup_alias, interface } => (lookup_alias, interface),
                    TargetHost::App {
                        app_lookup_alias,
                        app_did_hash,
                        service_name_hash,
                        interface,
                    } => {
                        match state
                            .app_host_resolver
                            .resolve_app_host(
                                &app_lookup_alias,
                                &app_did_hash,
                                &service_name_hash,
                                routing_key.as_deref(),
                            )
                            .await
                        {
                            Ok(member_did) => (member_did, interface),
                            Err(e) => {
                                error!(
                                    "gateway failed to resolve logical host '{host_header}': {e:#}"
                                );
                                return write_json_rpc_error(&mut stream, 502, "Bad Gateway").await;
                            }
                        }
                    }
                };

                debug!(
                    "Proxying to interface (hash): {}, service_id (alias): {}",
                    interface, service_id
                );

                let client_arc = state
                    .clients
                    .entry(service_id.clone())
                    .or_insert_with(|| {
                        // Reconstructed from the same key bytes rather than
                        // shared, since `Identity` is deliberately not
                        // `Clone` -- every downstream client presents the
                        // same node DID.
                        let identity = Identity::from_bytes(&state.identity.to_bytes());
                        Arc::new(Mutex::new(SyneroymClient::new_with_identity(
                            service_id.clone(),
                            state.registry_url.clone(),
                            identity,
                        )))
                    })
                    .clone();

                let (conn, service_id) = {
                    let mut client = client_arc.lock().await;
                    if let Err(e) = client.connect().await {
                        error!("Gateway failed to connect to service {}: {}", service_id, e);
                        return write_json_rpc_error(&mut stream, 502, "Bad Gateway").await;
                    }
                    (
                        client.connection().ok_or_else(|| anyhow::anyhow!("Connection lost"))?,
                        client.service_id().to_string(),
                    )
                };

                let passthrough_identity = Identity::from_bytes(&state.identity.to_bytes());
                SyneroymClient::passthrough_with_conn(
                    conn,
                    &service_id,
                    &interface,
                    &buf[..bytes_read],
                    &mut stream,
                    &passthrough_identity,
                )
                .await?;
                return Ok(());
            }
            Ok(Status::Partial) => {
                if bytes_read >= buf.len() {
                    return write_json_rpc_error(&mut stream, 400, "Headers too large").await;
                }
                continue;
            }
            Err(e) => {
                return write_json_rpc_error(
                    &mut stream,
                    400,
                    &format!("Invalid HTTP request: {e}"),
                )
                .await;
            }
        }
    }
}

/// Writes a JSON-RPC error response as an HTTP response.
async fn write_json_rpc_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    let body =
        format!(r#"{{"jsonrpc":"2.0","error":{{"code":-32603,"message":{message:?}}},"id":null}}"#);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 82: the gateway's own regression pin, at unit scale over the
    /// parse + target selection, no network -- an unscoped host still
    /// produces the same `(lookup_alias, interface)` pair `handle_connection`
    /// fed into `state.clients` before this slice.
    #[test]
    fn an_unscoped_host_reaches_the_same_service_it_did_before() {
        let sh = syneroym_core::util::short_hash("did:key:zSvc");
        let ih = syneroym_core::util::short_hash("default");
        let host = format!("my-svc-s{sh}-i{ih}.localhost");

        let target = parse_target_host(&host).unwrap();
        let TargetHost::Service { lookup_alias, interface } = target else {
            panic!("expected a Service target");
        };
        assert_eq!(lookup_alias, format!("my-svc-{sh}"));
        assert_eq!(interface, ih);
    }

    // Tests 83-89 (the D-S3-5 binding checks, task.md budgets 1 and 2, the
    // routing-key header, and the Sharded-with-no-key error) exercise
    // `AppHostResolver::resolve_app_host` itself, which now lives in
    // `syneroym_sdk::topology` (D-S3-11) -- see that module's own test
    // suite rather than duplicating them here against a thin wrapper.

    /// Test 90: D-S3-6, in the shape S1's no-registry warning test uses --
    /// against `credential_warning`, the pure decision `ClientGateway::init`
    /// makes, since building a full `SubstrateConfig` by hand (most of its
    /// nested config structs have no `Default`) would test config plumbing
    /// this crate does not own rather than the decision itself. The paired
    /// case: no warning when only the same-node gate is on.
    #[test]
    fn a_gateway_with_neither_credential_warns_at_init_naming_both_config_keys() {
        assert_eq!(
            credential_warning(false, false),
            Some(CredentialWarning::NeitherConfigured),
            "neither `resolve_ucan` nor the same-node gate: must warn"
        );
        assert_eq!(
            credential_warning(false, true),
            Some(CredentialWarning::OnlyTheSameNodeGate),
            "the same-node gate alone: a quieter debug, not a warning"
        );
        assert_eq!(credential_warning(true, false), None, "a resolve_ucan token: no warning");
        assert_eq!(credential_warning(true, true), None, "both configured: no warning");
    }
}
