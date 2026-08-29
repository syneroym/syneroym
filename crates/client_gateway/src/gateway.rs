//! HTTP Client Gateway
//!
//! Proxies external client requests into the internal Syneroym network,
//! managing routing, protocol translation, and error boundaries.

use std::{
    fmt::{self, Debug, Formatter},
    fs,
    path::Path,
    str,
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use httparse::{EMPTY_HEADER, Request, Status};
use syneroym_app_orchestration::{LogicalResolver, StaticInventory};
use syneroym_core::{
    config::{IdentityMode, SubstrateConfig},
    protocol_utils::{
        AUTH_SERVICE_ALIAS, ROUTING_KEY_HEADER, SESSION_COOKIE_NAME, TargetHost, parse_target_host,
    },
    util::load_or_generate_node_identity,
};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_rpc::CapabilityToken;
use syneroym_sdk::{
    SyneroymClient,
    topology::{
        AppHostResolver, CredentialWarning, RegistryTopologyFetcher, Tier2Fetch, credential_warning,
    },
};
use syneroym_ucan::SessionToken;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, oneshot, oneshot::Sender},
    time,
};
use tracing::{debug, error, info, warn};

/// Reads a `CapabilityToken` off disk.
fn read_capability_token(path: &Path) -> Result<CapabilityToken> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read UCAN token at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("invalid UCAN token JSON at {}", path.display()))
}

#[derive(Debug)]
struct GatewayState {
    registry_url: String,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
    /// The node's own identity -- used for establishing connections.
    identity: Identity,
    /// The app-scoped (`-a…-s…`) host resolver.
    app_host_resolver: AppHostResolver,
    /// Configured identity mode (`open`, `login`, or `fixed`).
    identity_mode: IdentityMode,
    /// Fixed person master DID when `identity_mode == Fixed`.
    fixed_identity_did: Option<String>,
    /// Fixed delegation certificate when `identity_mode == Fixed`.
    fixed_delegation: Option<DelegationCertificate>,
    /// Optional connection gate in `login` mode.
    connection_auth_gate: bool,
    /// DID of the local auth service if available.
    auth_service_did: Arc<RwLock<Option<String>>>,
}

/// `ClientGateway`: Acts as an entry point for local HTTP/WebSocket clients to
/// reach the wider Syneroym network.
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
    pub fn set_auth_did(&self, did: Option<String>) {
        if let Ok(mut lock) = self.state.auth_service_did.write() {
            *lock = did;
        }
    }

    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing client gateway");

        let port = config.roles.client_gateway.as_ref().map_or(7000, |g| g.http_port);
        let registry_url = config.substrate.registry_url.clone().unwrap_or_default();
        let identity = load_or_generate_node_identity(config)?;

        let auth_service_did = Arc::new(RwLock::new(config.roles.auth.as_ref().and_then(|a| {
            if let Some(path) = &a.key_path
                && path.exists()
            {
                Identity::load_from_path(path)
                    .ok()
                    .map(|id| substrate::derive_did_key(&id.public_key()))
            } else {
                None
            }
        })));

        let role_cfg = config.roles.client_gateway.as_ref();
        let identity_mode = role_cfg.map_or(IdentityMode::Open, |g| g.identity_mode);
        let fixed_identity_did = role_cfg.and_then(|g| g.fixed_identity_did.clone());
        let fixed_delegation = if let Some(g) = role_cfg
            && let Some(del_path) = &g.fixed_delegation
        {
            if del_path.exists() {
                let content = fs::read_to_string(del_path)?;
                Some(DelegationCertificate::from_json(&content)?)
            } else {
                warn!(path = %del_path.display(), "fixed delegation certificate file not found");
                None
            }
        } else {
            None
        };
        let connection_auth_gate = role_cfg.is_some_and(|g| g.connection_auth_gate);

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
                     refused by any supervisor they reach, unless the app declares that logical \
                     service `open` (ADR-0022 §5). Unscoped (-s only) hostnames are unaffected."
                ),
                Some(CredentialWarning::OnlyTheSameNodeGate) => debug!(
                    "client gateway relying on `iam.grant_resolve_to_node_did`; cross-node \
                     app-scoped hostnames will be refused unless the target app is `open`"
                ),
                None => {}
            }
            Some(Box::new(f) as Box<dyn Tier2Fetch>)
        };

        let app_host_resolver = AppHostResolver::new(
            Box::new(syneroym_sdk::topology::RegistryTier1Lookup::new(registry_url.clone())),
            fetcher,
            resolver,
        );

        let state = Arc::new(GatewayState {
            registry_url,
            clients: DashMap::new(),
            identity,
            app_host_resolver,
            identity_mode,
            fixed_identity_did,
            fixed_delegation,
            connection_auth_gate,
            auth_service_did,
        });

        Ok(Self { port, state, shutdown_tx: None })
    }

    pub async fn run(&mut self) -> Result<()> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("failed to bind client gateway to {addr}"))?;
        info!("running client gateway");
        info!("client gateway listening on {}", addr);

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    info!("client gateway received shutdown signal");
                    break;
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, peer_addr)) => {
                            debug!("gateway accepted connection from {}", peer_addr);
                            let state = self.state.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, state).await {
                                    error!("error handling connection from {}: {:#}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("gateway accept error: {:#}", e);
                        }
                    }
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

        for entry in &self.state.clients {
            let mut client = entry.value().lock().await;
            let _ = client.shutdown().await;
        }

        Ok(())
    }
}

fn is_auth_service_alias(alias: &str) -> bool {
    alias == AUTH_SERVICE_ALIAS || alias == "auth-00000000"
}

const READ_TIMEOUT: Duration = Duration::from_secs(10);

async fn handle_connection(mut stream: TcpStream, state: Arc<GatewayState>) -> Result<()> {
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    let mut buf = [0u8; MAX_HEADER_BYTES];
    let mut bytes_read = 0;

    enum HeaderRead {
        Complete(usize),
        TooLarge,
        ParseError(String),
        Closed,
    }

    let read_res: Result<Result<HeaderRead, anyhow::Error>, _> =
        time::timeout(READ_TIMEOUT, async {
            loop {
                let n = stream.read(&mut buf[bytes_read..]).await?;
                if n == 0 {
                    return Ok(HeaderRead::Closed);
                }
                bytes_read += n;

                let mut headers = [EMPTY_HEADER; 64];
                let mut req = Request::new(&mut headers);

                match req.parse(&buf[..bytes_read]) {
                    Ok(Status::Complete(len)) => return Ok(HeaderRead::Complete(len)),
                    Ok(Status::Partial) => {
                        if bytes_read >= MAX_HEADER_BYTES {
                            return Ok(HeaderRead::TooLarge);
                        }
                    }
                    Err(e) => return Ok(HeaderRead::ParseError(e.to_string())),
                }
            }
        })
        .await;

    let _header_len = match read_res {
        Ok(Ok(HeaderRead::Complete(len))) => len,
        Ok(Ok(HeaderRead::TooLarge)) => {
            return write_json_rpc_error(&mut stream, 400, "Headers too large").await;
        }
        Ok(Ok(HeaderRead::ParseError(e))) => {
            return write_json_rpc_error(&mut stream, 400, &format!("Invalid HTTP request: {e}"))
                .await;
        }
        Ok(Ok(HeaderRead::Closed)) => return Ok(()),
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return write_json_rpc_error(
                &mut stream,
                408,
                "Timed out reading HTTP request headers",
            )
            .await;
        }
    };

    let mut headers = [EMPTY_HEADER; 64];
    let mut req = Request::new(&mut headers);
    let _ = req.parse(&buf[..bytes_read]);

    let path = req.path.unwrap_or("");

    if path.starts_with("/_syneroym/") && !path.starts_with("/_syneroym/session") {
        return write_json_rpc_error(&mut stream, 404, "Not Found").await;
    }

    let host_header = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("host"))
        .map_or("", |h| str::from_utf8(h.value).unwrap_or(""));

    let target = match parse_target_host(host_header) {
        Some(t) => t,
        None => {
            if path.starts_with("/_syneroym/session") {
                TargetHost::Service {
                    lookup_alias: AUTH_SERVICE_ALIAS.to_string(),
                    interface: String::new(),
                }
            } else {
                return write_json_rpc_error(&mut stream, 400, "Missing or invalid Host header")
                    .await;
            }
        }
    };

    let is_auth_host = match &target {
        TargetHost::Service { lookup_alias, .. } => is_auth_service_alias(lookup_alias),
        TargetHost::App { .. } => false,
    };

    // In fixed mode, if a whoami request arrives for auth, answer directly
    if state.identity_mode == IdentityMode::Fixed
        && is_auth_host
        && (path == "/_syneroym/session/whoami" || path == "/whoami")
    {
        let fixed_did = state.fixed_identity_did.clone().unwrap_or_default();
        let resp = serde_json::json!({
            "person_did": fixed_did,
            "auth": "fixed",
            "expires_at_secs": 9_999_999_999u64,
            "facts": {
                "auth_method": "fixed"
            }
        });
        let body = serde_json::to_vec(&resp)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
             {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).await?;
        stream.write_all(&body).await?;
        return Ok(());
    }

    // Connection auth gate in login mode: if enabled and not visiting auth service,
    // verify presence of a valid, signed, unexpired session credential
    if state.identity_mode == IdentityMode::Login && state.connection_auth_gate && !is_auth_host {
        let token_opt = req.headers.iter().find_map(|h| {
            if h.name.eq_ignore_ascii_case("cookie") {
                let v = str::from_utf8(h.value).unwrap_or("");
                for pair in v.split(';') {
                    let mut parts = pair.splitn(2, '=');
                    if let (Some(k), Some(val)) = (parts.next(), parts.next())
                        && k.trim() == SESSION_COOKIE_NAME
                    {
                        return Some(val.trim().to_string());
                    }
                }
            } else if h.name.eq_ignore_ascii_case("authorization") {
                let v = str::from_utf8(h.value).unwrap_or("");
                let trimmed = v.trim();
                if let Some(tok) = trimmed.strip_prefix("Bearer ") {
                    return Some(tok.trim().to_string());
                }
                if let Some(tok) = trimmed.strip_prefix("bearer ") {
                    return Some(tok.trim().to_string());
                }
            }
            None
        });

        let expected_auth_did = state.auth_service_did.read().ok().and_then(|g| g.clone());
        let is_valid = match (token_opt.as_deref(), expected_auth_did.as_deref()) {
            (Some(tok), Some(did)) => SessionToken::verify(tok, did).is_ok(),
            _ => false,
        };

        if !is_valid {
            let body = serde_json::json!({"error": "unauthorized: valid session required"});
            let body_bytes = serde_json::to_vec(&body)?;
            let response = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: \
                 {}\r\nConnection: close\r\n\r\n",
                body_bytes.len()
            );
            stream.write_all(response.as_bytes()).await?;
            stream.write_all(&body_bytes).await?;
            return Ok(());
        }
    }

    let routing_key: Option<Vec<u8>> = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(ROUTING_KEY_HEADER))
        .map(|h| h.value.to_vec());

    let (service_id, interface) = match resolve_target(&state, target, routing_key.as_deref()).await
    {
        Ok(pair) => pair,
        Err(e) => {
            error!("gateway failed to resolve logical host '{host_header}': {e:#}");
            return write_json_rpc_error(&mut stream, 502, "Bad Gateway").await;
        }
    };

    debug!("Proxying to interface (hash): {}, service_id (alias): {}", interface, service_id);

    let node_did = substrate::derive_did_key(&state.identity.public_key());
    let connect_service_id = if is_auth_host { node_did.clone() } else { service_id.clone() };

    let client_arc = state
        .clients
        .entry(connect_service_id.clone())
        .or_insert_with(|| {
            let identity = Identity::from_bytes(&state.identity.to_bytes());
            Arc::new(Mutex::new(SyneroymClient::new_with_identity(
                connect_service_id.clone(),
                state.registry_url.clone(),
                identity,
            )))
        })
        .clone();

    let (conn, target_service_id) = {
        let mut client = client_arc.lock().await;
        if let Err(e) = client.connect().await {
            error!("Gateway failed to connect to service {}: {}", connect_service_id, e);
            return write_json_rpc_error(&mut stream, 502, "Bad Gateway").await;
        }
        let resolved_id = if is_auth_host {
            AUTH_SERVICE_ALIAS.to_string()
        } else {
            client.service_id().to_string()
        };
        (client.connection().ok_or_else(|| anyhow::anyhow!("Connection lost"))?, resolved_id)
    };

    let delegation = match state.identity_mode {
        IdentityMode::Fixed => state.fixed_delegation.as_ref(),
        IdentityMode::Login | IdentityMode::Open => None,
    };

    // Forward the initial bytes untouched (cookie is not stripped)
    let passthrough_identity = Identity::from_bytes(&state.identity.to_bytes());
    SyneroymClient::passthrough_with_conn(
        conn,
        &target_service_id,
        &interface,
        &buf[..bytes_read],
        &mut stream,
        &passthrough_identity,
        delegation,
    )
    .await?;
    Ok(())
}

async fn resolve_target(
    state: &GatewayState,
    target: TargetHost,
    routing_key: Option<&[u8]>,
) -> Result<(String, String)> {
    match target {
        TargetHost::Service { lookup_alias, interface } => Ok((lookup_alias, interface)),
        TargetHost::App { app_lookup_alias, app_did_hash, service_name_hash, interface } => {
            let member_did = state
                .app_host_resolver
                .resolve_app_host(&app_lookup_alias, &app_did_hash, &service_name_hash, routing_key)
                .await?;
            Ok((member_did, interface))
        }
    }
}

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
    use syneroym_core::dht_registry::SignedEndpointInfo;
    use syneroym_sdk::topology::Tier1Lookup;

    use super::*;

    #[test]
    fn parsing_an_unscoped_host_yields_the_expected_alias_and_interface() {
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

    #[derive(Debug)]
    struct UnreachableTier1;

    #[async_trait::async_trait]
    impl Tier1Lookup for UnreachableTier1 {
        async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
            Err(anyhow::anyhow!("Tier1Lookup::lookup must not be called here"))
        }
    }

    fn test_state(fetcher: Option<Box<dyn Tier2Fetch>>) -> GatewayState {
        let identity = Identity::generate().unwrap();
        GatewayState {
            registry_url: String::new(),
            clients: DashMap::new(),
            identity,
            app_host_resolver: AppHostResolver::new(
                Box::new(UnreachableTier1),
                fetcher,
                LogicalResolver::new(Arc::new(StaticInventory::new())),
            ),
            identity_mode: IdentityMode::Open,
            fixed_identity_did: None,
            fixed_delegation: None,
            connection_auth_gate: false,
            auth_service_did: Arc::new(RwLock::new(None)),
        }
    }

    #[tokio::test]
    async fn resolve_target_passes_an_unscoped_host_through_unresolved() {
        let state = test_state(None);
        let target = TargetHost::Service {
            lookup_alias: "my-svc-alias".to_string(),
            interface: "some-interface-hash".to_string(),
        };

        let (service_id, interface) = resolve_target(&state, target, None).await.unwrap();
        assert_eq!(service_id, "my-svc-alias");
        assert_eq!(interface, "some-interface-hash");
    }

    #[tokio::test]
    async fn resolve_target_routes_an_app_scoped_host_through_the_resolver_and_surfaces_its_error()
    {
        let state = test_state(None);
        let target = TargetHost::App {
            app_lookup_alias: "my-app-abcdefgh".to_string(),
            app_did_hash: "abcdefgh".to_string(),
            service_name_hash: "ijklmnop".to_string(),
            interface: String::new(),
        };

        let err = resolve_target(&state, target, None).await.unwrap_err();
        assert!(err.to_string().contains("no community registry configured"), "{err}");
    }

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
