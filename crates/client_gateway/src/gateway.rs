//! HTTP Client Gateway
//!
//! Proxies external client requests into the internal Syneroym network,
//! managing routing, protocol translation, and error boundaries.

use std::{
    fmt::{self, Debug, Formatter},
    fs,
    path::{Path, PathBuf},
    str,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use httparse::{EMPTY_HEADER, Request, Status};
use syneroym_app_orchestration::{LogicalResolver, StaticInventory};
use syneroym_core::{
    config::{SubstrateConfig, default_session_ttl_secs},
    dht_registry::{MasterAnchorResolver, RegistryClient},
    protocol_utils::{ROUTING_KEY_HEADER, SESSION_COOKIE_NAME, TargetHost, parse_target_host},
    util::load_or_generate_node_identity,
};
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_ROUTING, substrate};
use syneroym_rpc::CapabilityToken;
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
    time,
};
use tracing::{debug, error, info, warn};

use crate::session::{
    self, MAX_SESSION_BODY_BYTES, RequestKind, SessionRoute, SessionStore, WhoamiResponse,
};

// When no local person session is attached, the gateway presents the node DID
// as the caller DID (ADR-0016 §0.5), which downstream handlers see as
// `self-asserted`. When a client presents an active person session token, the
// gateway attaches the owner->node delegation certificate to the route
// preamble, which downstream handlers see as `delegated` and resolve to the
// person's master DID.

/// Reads a `CapabilityToken` off disk, the same shape `roymctl`'s own
/// `--ucan` loading uses.
fn read_capability_token(path: &Path) -> Result<CapabilityToken> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read UCAN token at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("invalid UCAN token JSON at {}", path.display()))
}

#[derive(Debug)]
struct GatewayState {
    registry_url: String,
    enable_bep0044_dht: bool,
    person_identities_dir: Option<PathBuf>,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
    /// The node's own identity (ADR-0016 §0.5) -- used for establishing
    /// connections and as the fallback self-asserted caller identity when
    /// no person session is active.
    identity: Identity,
    /// The app-scoped (`-a…-s…`) host resolver:
    /// `sdk::topology::AppHostResolver`, shared with the WebRTC coordinator
    /// rather than reimplemented, since the two are the pair most likely
    /// to drift subtly apart on binding checks. Deliberately
    /// not the node's own `LogicalResolver`: `ClientGateway::init` runs
    /// before `setup_connection_router` builds that one, and the two key
    /// spaces are disjoint by type (`AppScope`) anyway.
    app_host_resolver: AppHostResolver,
    /// Local person sessions. Empty at boot and after every restart by design.
    sessions: SessionStore,
    /// Used at login only to resolve the person's master anchor once and
    /// refuse a session that could never have worked.
    anchor_lookup: Arc<dyn MasterAnchorResolver>,
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
        let enable_bep0044_dht = config.substrate.enable_bep0044_dht;
        let person_identities_dir =
            config.roles.client_gateway.as_ref().and_then(|g| g.person_identities_dir.clone());
        let identity = load_or_generate_node_identity(config)?;

        let session_ttl_secs = config
            .roles
            .client_gateway
            .as_ref()
            .map_or_else(default_session_ttl_secs, |g| g.session_ttl_secs);
        let node_did = substrate::derive_did_key(&identity.public_key());
        let sessions = SessionStore::new(node_did, session_ttl_secs);

        let anchor_lookup: Arc<dyn MasterAnchorResolver> = Arc::new(RegistryClient::new(
            enable_bep0044_dht,
            config.substrate.registry_url.clone(),
        ));

        // The gateway owns its own resolver and fetcher. It cannot
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
                     refused by any supervisor they reach, unless the app declares that logical \
                     service `open` (ADR-0022 §5). Unscoped (-s only) hostnames are unaffected."
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
            enable_bep0044_dht,
            person_identities_dir,
            clients: DashMap::new(),
            identity,
            app_host_resolver,
            sessions,
            anchor_lookup,
        });

        Ok(Self { port, state, shutdown_tx: None })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running client gateway on port {}", self.port);
        let state = self.state.clone();

        // Loopback bind: the client gateway operates on 127.0.0.1 for local machine
        // security under the local person session model, rather than an internet-facing
        // multi-tenant gateway.
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

const READ_TIMEOUT: Duration = Duration::from_secs(10);

async fn handle_connection(mut stream: TcpStream, state: Arc<GatewayState>) -> Result<()> {
    // Limit header reads to 8 KB — the conventional maximum for HTTP/1.1 headers.
    // Requests with larger headers (e.g. very large JWTs) will receive a 400
    // response.
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    let mut buf = [0u8; MAX_HEADER_BYTES];
    let mut bytes_read = 0;

    enum HeaderRead {
        Complete { header_len: usize, bytes_read: usize },
        TooLarge,
        ParseError(String),
        Closed,
    }

    // Read headers with an overall deadline to mitigate slowloris attacks
    let read_res: Result<Result<HeaderRead, anyhow::Error>, _> =
        time::timeout(READ_TIMEOUT, async {
            loop {
                let n = stream.read(&mut buf[bytes_read..]).await?;
                if n == 0 {
                    return Ok(HeaderRead::Closed);
                }
                bytes_read += n;
                debug!("gateway read {} bytes, total {}", n, bytes_read);

                let mut headers = [EMPTY_HEADER; 64];
                let mut req = Request::new(&mut headers);

                match req.parse(&buf[..bytes_read]) {
                    Ok(Status::Complete(len)) => {
                        return Ok(HeaderRead::Complete { header_len: len, bytes_read });
                    }
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

    let (header_len, total_bytes_read) = match read_res {
        Ok(Ok(HeaderRead::Complete { header_len, bytes_read })) => (header_len, bytes_read),
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
    let _ = req.parse(&buf[..header_len]);

    let method = req.method.unwrap_or("");
    let path = req.path.unwrap_or("");
    debug!("gateway parsed request method='{}' path='{}'", method, path);
    let kind = session::classify(method, path);
    let credential = session::extract_credential(req.headers);
    let sec_fetch_site = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("sec-fetch-site"))
        .and_then(|h| str::from_utf8(h.value).ok())
        .map(str::to_string);

    if let RequestKind::Session(route) = kind {
        let has_expect_100 = req.headers.iter().any(|h| {
            h.name.eq_ignore_ascii_case("expect")
                && str::from_utf8(h.value)
                    .map(|v| v.trim().eq_ignore_ascii_case("100-continue"))
                    .unwrap_or(false)
        });
        if has_expect_100 {
            stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }

        let content_length: usize = req
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("content-length"))
            .and_then(|h| str::from_utf8(h.value).ok())
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);

        if content_length > MAX_SESSION_BODY_BYTES {
            return write_json(
                &mut stream,
                413,
                &serde_json::json!({"error": "request body too large"}),
                None,
            )
            .await;
        }

        let mut body = buf[header_len..total_bytes_read].to_vec();
        let mut chunk = [0u8; 1024];
        let body_read_res = time::timeout(READ_TIMEOUT, async {
            while body.len() < content_length {
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    return Err(anyhow::anyhow!("truncated body"));
                }
                body.extend_from_slice(&chunk[..n]);
            }
            Ok::<(), anyhow::Error>(())
        })
        .await;

        match body_read_res {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return write_json(
                    &mut stream,
                    400,
                    &serde_json::json!({"error": "truncated body"}),
                    None,
                )
                .await;
            }
            Err(_) => {
                return write_json(
                    &mut stream,
                    408,
                    &serde_json::json!({"error": "timed out reading request body"}),
                    None,
                )
                .await;
            }
        }
        body.truncate(content_length);

        return handle_session_request(
            &mut stream,
            &state,
            route,
            credential.as_ref().map(|(t, _)| t.as_str()),
            sec_fetch_site.as_deref(),
            &body,
        )
        .await;
    }

    let host_header = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("host"))
        .map_or("", |h| str::from_utf8(h.value).unwrap_or(""));
    let target = match parse_target_host(host_header) {
        Some(t) => t,
        None => {
            return write_json_rpc_error(&mut stream, 400, "Missing or invalid Host header").await;
        }
    };
    // Read once, from the first request on this TCP connection.
    // `passthrough_with_conn` below hands the whole socket to
    // one iroh stream for the connection's lifetime, so the
    // member this key selects covers every later request an
    // HTTP keep-alive reuses the connection for too -- a
    // per-connection decision, not a per-request one. Tracked
    // in `deferred-backlog.md`; see the developer guide's
    // gateway-hostname section for the caller-facing note.
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

    let (forwarded_bytes, delegation) = match credential {
        Some((token, src)) => {
            let stripped =
                session::strip_credential(&buf[..total_bytes_read], header_len, &token, src);
            let s_opt = state.sessions.lookup(&token);
            if let Some(ref s) = s_opt {
                debug!(person = %s.person_did, "gateway proxying under a person session");
            }
            (stripped, s_opt.map(|s| s.delegation))
        }
        None => (None, None),
    };
    let bytes_to_forward =
        prepare_forwarded_bytes(&buf[..total_bytes_read], header_len, forwarded_bytes);

    let passthrough_identity = Identity::from_bytes(&state.identity.to_bytes());
    SyneroymClient::passthrough_with_conn(
        conn,
        &service_id,
        &interface,
        &bytes_to_forward,
        &mut stream,
        &passthrough_identity,
        delegation.as_ref(),
    )
    .await?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn prepare_forwarded_bytes(raw: &[u8], header_len: usize, stripped: Option<Vec<u8>>) -> Vec<u8> {
    let source_bytes = stripped.as_deref().unwrap_or(raw);
    let mut headers = [EMPTY_HEADER; 64];
    let mut req = Request::new(&mut headers);
    let effective_header_len = if let Ok(Status::Complete(len)) = req.parse(source_bytes) {
        len
    } else {
        header_len.min(source_bytes.len())
    };

    // WebSocket upgrade requests carry `Connection: Upgrade`; rewriting it to
    // `Connection: close` kills the handshake. Return the bytes unmodified so
    // the upstream receives the correct upgrade negotiation.
    let is_ws_upgrade = req.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("upgrade") && h.value.eq_ignore_ascii_case(b"websocket")
    });
    if is_ws_upgrade {
        return source_bytes.to_vec();
    }

    let head_bytes = &source_bytes[..effective_header_len];
    let tail_bytes = &source_bytes[effective_header_len..];

    let mut out_head = Vec::with_capacity(head_bytes.len() + 32);
    let mut has_conn = false;

    let mut remaining = head_bytes;
    while !remaining.is_empty() {
        let (line, rest) = match remaining.windows(2).position(|w| w == b"\r\n") {
            Some(pos) => (&remaining[..pos], &remaining[pos + 2..]),
            None => (remaining, &[][..]),
        };
        remaining = rest;

        if line.is_empty() {
            break;
        }

        if line.len() >= 11 && line[..11].eq_ignore_ascii_case(b"connection:") {
            out_head.extend_from_slice(b"Connection: close\r\n");
            has_conn = true;
        } else {
            out_head.extend_from_slice(line);
            out_head.extend_from_slice(b"\r\n");
        }
    }

    if !has_conn {
        out_head.extend_from_slice(b"Connection: close\r\n");
    }
    out_head.extend_from_slice(b"\r\n");

    let mut result = Vec::with_capacity(out_head.len() + tail_bytes.len());
    result.extend_from_slice(&out_head);
    result.extend_from_slice(tail_bytes);
    result
}

async fn write_login_grant(stream: &mut TcpStream, grant: session::LoginResponse) -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let remaining_ttl = grant.expires_at_secs.saturating_sub(now);
    let cookie = format!(
        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict",
        SESSION_COOKIE_NAME, grant.token, remaining_ttl
    );
    let body_val = serde_json::to_value(&grant)?;
    write_json(stream, 200, &body_val, Some(&cookie)).await
}

async fn handle_session_request(
    stream: &mut TcpStream,
    state: &GatewayState,
    route: SessionRoute,
    credential: Option<&str>,
    sec_fetch_site: Option<&str>,
    body: &[u8],
) -> Result<()> {
    match route {
        SessionRoute::Challenge => {
            let ch = state.sessions.issue_challenge();
            let body_val = serde_json::to_value(&ch)?;
            write_json(stream, 200, &body_val, None).await
        }
        SessionRoute::Login => {
            let req: session::LoginRequest = match serde_json::from_slice(body) {
                Ok(r) => r,
                Err(_) => {
                    return write_json(
                        stream,
                        400,
                        &serde_json::json!({"error": "malformed login request"}),
                        None,
                    )
                    .await;
                }
            };
            match state.sessions.login(&req, state.anchor_lookup.as_ref()).await {
                Ok(grant) => write_login_grant(stream, grant).await,
                Err(e) => {
                    write_json(
                        stream,
                        e.http_status(),
                        &serde_json::json!({"error": e.message()}),
                        None,
                    )
                    .await
                }
            }
        }
        SessionRoute::Identities => {
            let dir = match &state.person_identities_dir {
                Some(d) => d,
                None => {
                    return write_json(
                        stream,
                        404,
                        &serde_json::json!({"error": "local person identities are not configured"}),
                        None,
                    )
                    .await;
                }
            };
            let names = session::list_person_identities(dir);
            let resp = session::IdentitiesResponse { identities: names };
            let body_val = serde_json::to_value(&resp)?;
            write_json(stream, 200, &body_val, None).await
        }
        SessionRoute::LoginLocal => {
            if let Some(site) = sec_fetch_site
                && site != "same-origin"
                && site != "none"
            {
                return write_json(
                    stream,
                    403,
                    &serde_json::json!({"error": "cross-site local login is not allowed"}),
                    None,
                )
                .await;
            }
            let dir = match &state.person_identities_dir {
                Some(d) => d,
                None => {
                    return write_json(
                        stream,
                        404,
                        &serde_json::json!({"error": "local person identities are not configured"}),
                        None,
                    )
                    .await;
                }
            };
            let req: session::LocalLoginRequest = match serde_json::from_slice(body) {
                Ok(r) => r,
                Err(_) => {
                    return write_json(
                        stream,
                        400,
                        &serde_json::json!({"error": "malformed login request"}),
                        None,
                    )
                    .await;
                }
            };
            let available = session::list_person_identities(dir);
            if !available.contains(&req.identity) {
                return write_json(
                    stream,
                    404,
                    &serde_json::json!({"error": "no such local identity"}),
                    None,
                )
                .await;
            }
            let key_path = dir.join("identities").join(format!("{}.key", req.identity));
            let person = match Identity::load_from_path(&key_path) {
                Ok(id) => id,
                Err(e) => {
                    return write_json(
                        stream,
                        500,
                        &serde_json::json!({"error": format!("could not load identity file {}: {e}", key_path.display())}),
                        None,
                    )
                    .await;
                }
            };
            let person_did = substrate::derive_did_key(&person.public_key());
            let ch = state.sessions.issue_challenge();
            let node = match substrate::resolve_did_key(&ch.node_did) {
                Ok(pk) => pk,
                Err(_) => {
                    return write_json(
                        stream,
                        500,
                        &serde_json::json!({"error": "invalid node DID in challenge"}),
                        None,
                    )
                    .await;
                }
            };
            let cert = match DelegationCertificate::issue(
                &person,
                node,
                state.sessions.ttl_secs(),
                SCOPE_ROUTING.to_string(),
            ) {
                Ok(c) => c,
                Err(e) => {
                    return write_json(
                        stream,
                        500,
                        &serde_json::json!({"error": format!("could not issue delegation certificate: {e}")}),
                        None,
                    )
                    .await;
                }
            };
            let assertion_val = session::assertion_value(&ch.node_did, &ch.nonce, &person_did);
            let sig = match person.sign_json(&assertion_val) {
                Ok(s) => s,
                Err(e) => {
                    return write_json(
                        stream,
                        500,
                        &serde_json::json!({"error": format!("could not sign assertion: {e}")}),
                        None,
                    )
                    .await;
                }
            };

            if !state.registry_url.is_empty() {
                let reg_client =
                    RegistryClient::new(state.enable_bep0044_dht, Some(state.registry_url.clone()));
                if let Err(e) = reg_client.refresh_master_anchor(&person).await {
                    return write_json(
                        stream,
                        502,
                        &serde_json::json!({"error": format!("could not publish this person's anchor: {e}")}),
                        None,
                    )
                    .await;
                }
            }

            let login_req = session::LoginRequest {
                person_did,
                nonce: ch.nonce,
                signature: sig,
                delegation: cert,
            };

            match state.sessions.login(&login_req, state.anchor_lookup.as_ref()).await {
                Ok(grant) => write_login_grant(stream, grant).await,
                Err(e) => {
                    write_json(
                        stream,
                        e.http_status(),
                        &serde_json::json!({"error": e.message()}),
                        None,
                    )
                    .await
                }
            }
        }
        SessionRoute::Logout => {
            let token = match credential {
                Some(t) => t,
                None => {
                    return write_json(
                        stream,
                        401,
                        &serde_json::json!({"error": "no session"}),
                        None,
                    )
                    .await;
                }
            };
            state.sessions.logout(token);
            let cookie =
                format!("{}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict", SESSION_COOKIE_NAME);
            write_json(stream, 200, &serde_json::json!({"status": "ended"}), Some(&cookie)).await
        }
        SessionRoute::Whoami => {
            let token = match credential {
                Some(t) => t,
                None => {
                    return write_json(
                        stream,
                        401,
                        &serde_json::json!({"error": "no session"}),
                        None,
                    )
                    .await;
                }
            };
            let session = match state.sessions.lookup(token) {
                Some(s) => s,
                None => {
                    return write_json(
                        stream,
                        401,
                        &serde_json::json!({"error": "no session"}),
                        None,
                    )
                    .await;
                }
            };
            let resp = WhoamiResponse {
                person_did: session.person_did,
                auth: "delegated",
                expires_at_secs: session.expires_at_secs,
            };
            let body_val = serde_json::to_value(&resp)?;
            write_json(stream, 200, &body_val, None).await
        }
        SessionRoute::Unknown => {
            write_json(stream, 404, &serde_json::json!({"error": "unknown gateway endpoint"}), None)
                .await
        }
    }
}

/// Writes a JSON body with an explicit status, optionally with one
/// `Set-Cookie`. Always `Connection: close`.
async fn write_json(
    stream: &mut TcpStream,
    status: u16,
    body: &serde_json::Value,
    set_cookie: Option<&str>,
) -> Result<()> {
    let body_bytes = serde_json::to_vec(body)?;
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n",
        body_bytes.len()
    );
    if let Some(cookie) = set_cookie {
        response.push_str(&format!("Set-Cookie: {cookie}\r\n"));
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(&body_bytes).await?;
    Ok(())
}

/// The decision `handle_connection` makes from a parsed `TargetHost`: an
/// unscoped host's `(lookup_alias, interface)` pair needs no resolution;
/// an app-scoped one is resolved through `AppHostResolver` first. Pulled
/// out of the connection-handling loop so a unit test can drive it
/// directly against a fake Tier 1/Tier 2, without a real TCP connection
/// (finding C6 -- the previous "gateway's own regression pin" never
/// touched this decision at all, only `protocol_utils::parse_target_host`,
/// already covered by that module's own tests).
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
    use syneroym_core::dht_registry::{MasterAnchorPayload, SignedEndpointInfo};
    use syneroym_sdk::topology::Tier1Lookup;

    use super::*;

    /// Test 82: `parse_target_host`'s own output, unaffected by S3's
    /// hostname reshape. **Renamed from "the gateway's own regression
    /// pin" (finding C6)**: this exercises only `protocol_utils::
    /// parse_target_host`, a copy of that module's own test 60 -- it never
    /// touches `handle_connection`'s actual target-selection decision.
    /// `resolve_target_*` below are the gateway's real regression pins.
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

    /// A `Tier1Lookup` that must never be called -- used where the test
    /// expects `resolve_target` to fail (or succeed) before Tier 1 is
    /// ever reached.
    #[derive(Debug)]
    struct UnreachableTier1;

    #[async_trait::async_trait]
    impl Tier1Lookup for UnreachableTier1 {
        async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
            Err(anyhow::anyhow!("Tier1Lookup::lookup must not be called here"))
        }
    }

    #[derive(Debug)]
    struct UnreachableMasterAnchorResolver;

    #[async_trait::async_trait]
    impl MasterAnchorResolver for UnreachableMasterAnchorResolver {
        async fn resolve_master_anchor(
            &self,
            _master_id: &str,
        ) -> Result<MasterAnchorPayload, anyhow::Error> {
            Err(anyhow::anyhow!("UnreachableMasterAnchorResolver must not be called in this test"))
        }
    }

    fn test_state(fetcher: Option<Box<dyn Tier2Fetch>>) -> GatewayState {
        let identity = Identity::generate().unwrap();
        let node_did = substrate::derive_did_key(&identity.public_key());
        GatewayState {
            registry_url: String::new(),
            enable_bep0044_dht: false,
            person_identities_dir: None,
            clients: DashMap::new(),
            identity,
            app_host_resolver: AppHostResolver::new(
                Box::new(UnreachableTier1),
                fetcher,
                LogicalResolver::new(Arc::new(StaticInventory::new())),
            ),
            sessions: SessionStore::new(node_did, 3600),
            anchor_lookup: Arc::new(UnreachableMasterAnchorResolver),
        }
    }

    /// Finding C6: the actual `TargetHost -> (service_id, interface)`
    /// decision `handle_connection` makes, exercised directly rather than
    /// only through `parse_target_host`'s own output. An unscoped host
    /// needs no resolver at all -- `resolve_target` must pass it through
    /// unresolved, and `state`'s `app_host_resolver` (wired to a Tier 1
    /// that panics if called) is the proof nothing was.
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

    /// The other branch: an app-scoped host is routed through
    /// `AppHostResolver`, and a resolution failure surfaces as
    /// `resolve_target`'s own `Err` rather than being swallowed --
    /// exactly what `handle_connection` turns into its 502. Uses the
    /// gateway's own no-registry-configured shape (finding C7's sibling,
    /// at this layer) since it needs no signed document to reach.
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
