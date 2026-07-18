//! Async I/O copy loops and bridge utilities
//!
//! Handles bidirectional copy tasks and framing adapters for bridged streams.

use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, CallerProof, Capability, CapabilityToken, ChainVerifyOpts,
    ResourceUri, SessionContext, framing,
};
use tokio::{
    io,
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time,
};
use tracing::debug;

/// Bound on how long an unauthenticated peer gets to finish sending the
/// route preamble and (if the route calls for one) its initial framed
/// payload -- both read before any capacity check or WASM instantiation, so
/// without this a slow/idle peer could hold a stream open indefinitely
/// (matches the 5s budget `HandshakeVerifier` already uses for its own
/// pre-auth network round trip in `crates/router/src/handshake.rs`).
const PRE_AUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on the route preamble line's byte length -- read before any
/// peer authentication happens, so without this an unauthenticated peer
/// could force arbitrarily large allocation via an oversized `delegation=`/
/// `ucan=`/etc. query param before `RoutePreamble::parse` ever runs. Sized
/// with headroom over the largest realistic legitimate preamble: a
/// `syneroym-ucan` chain at its own `MAX_CHAIN_NODES` cap (64 tokens),
/// hex-encoded, comes to roughly 100 KiB in the worst case; a `delegation=`
/// cert is a few hundred bytes.
const MAX_PREAMBLE_LINE_BYTES: u64 = 256 * 1024;

use super::{super::SYNEROYM_ALPN, RouteHandler, dispatch, encryption::ReaderWriter};
use crate::{
    handshake::{HandshakeVerifier, MasterAnchorResolver, VerifiedIdentity},
    net_iroh,
    net_iroh::{IrohStream, connect_with_retry},
    preamble::RoutePreamble,
    route_handler::encryption::{OwnedStream, apply_encryption_stage},
    routing::{RoutePipeline, ServiceStage, TransportStage},
    stop_signal::StopSignal,
};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Faithful generalization of `HandshakeVerifier::verify_preamble`'s
/// delegation-cert revocation check (`handshake.rs:74-84`) to a UCAN chain:
/// for each edge in the token tree, resolve the *issuer's* master anchor and
/// reject if the *audience* DID is in its `revoked_keys`. An unresolvable
/// anchor is treated as not-revoked, matching the delegation path (which
/// only hard-fails on *timeout*, not on a missing anchor for a key that
/// isn't revoked). Identical `(issuer, audience)` edges (a diamond-shaped
/// chain reusing the same proof, or a chain that simply repeats an issuer)
/// are resolved at most once -- `verify_chain`'s `MAX_CHAIN_NODES` cap
/// already bounds the total edge count, but de-duplicating avoids paying for
/// the same network round trip redundantly within that bound.
async fn ucan_chain_not_revoked(
    token: &CapabilityToken,
    resolver: &dyn MasterAnchorResolver,
) -> bool {
    let mut checked = HashSet::new();
    for edge @ (issuer, audience) in token.chain_edges() {
        if !checked.insert(edge) {
            continue;
        }
        if let Ok(anchor) = resolver.resolve_master_anchor(issuer).await
            && anchor.revoked_keys.iter().any(|k| k == audience)
        {
            return false;
        }
    }
    true
}

/// Builds the `CallerContext` for a verified handshake identity (ADR-0016
/// §4.2, Slice B1). A caller whose master DID equals the configured
/// `[iam].admin_ucan_root` is granted `substrate/admin` on this node (the B0
/// direct-equality path, kept). A presented `preamble.ucan` chain rooted at
/// that same admin root is additionally verified and merged in; `auth` is
/// upgraded to `AuthLevel::Ucan` only when the chain actually admitted at
/// least one capability -- a structurally valid but entirely untrusted chain
/// (e.g. self-issued, rooted nowhere) must not read as "holds a verified UCAN
/// capability" to any future code that checks `auth == Ucan` as a privilege
/// signal.
///
/// TODO(B7b / post-B7): B0's gate only proves *an* identity is present.
/// "May this caller touch this service at all?" is Tier 1 -- a µs-scale
/// grant-layer capability check, NOT an FDAE/M04B policy question (ADR-0017
/// Open, design §9.8; this comment previously mis-addressed it to M04B).
/// B7b implements it for `orchestrator`. `security` and the five data
/// native-capability interfaces remain open -- today any verified identity
/// reaches any native service. `security`'s gate is `substrate/admin`,
/// which is unholdable until a ControllerAgreement can be created, so it
/// ships with that tool (B7.md F3.1). M04B/FDAE owns Tier 3 (rows/columns)
/// only.
/// The `None` rejection below is correct and settled (design §6.1.2):
/// native interfaces reject anonymous callers, WASM guests admit them.
async fn build_caller(
    preamble: &RoutePreamble,
    id: &VerifiedIdentity,
    admin_root: Option<&str>,
    node_did: &str,
    resolver: &dyn MasterAnchorResolver,
) -> CallerContext {
    let now = now_secs();
    let mut session = SessionContext {
        subject_did: id.master_did.clone(),
        verified_at_secs: now,
        ..Default::default()
    };
    let mut auth = AuthLevel::Delegated;

    // M04A Slice B7a (F4): the substrate-owner capability is issued from
    // this single site, with no "is this substrate owned?" branch anywhere
    // downstream -- the unowned bootstrap posture is expressed as a real
    // issued capability, not a skipped check (design §6.1.1).
    let node_wide_abilities: Vec<&str> = match admin_root {
        // Owned, and this caller is the owner: substrate/admin (kept from
        // B0), entailing everything on the node.
        Some(root) if root == id.master_did => vec![Ability::SUBSTRATE_ADMIN],
        // Owned, but this caller is not the owner: nothing node-wide.
        Some(_) => vec![],
        // UNOWNED: no verified ControllerAgreement controller and no
        // [iam].admin_ucan_root -- nobody can root an orchestrator grant, so
        // default-deny would brick the substrate permanently (you could not
        // deploy the thing that would establish ownership). Every verified
        // caller therefore holds the orchestrator abilities here.
        //
        // NOT substrate/admin: that would entail data-layer/admin too
        // (`Ability::entails`'s substrate/admin short-circuit), opening
        // execute-ddl/query-raw to every verified caller -- strictly worse
        // than today's `admin_root: None`, where nobody holds it and DDL is
        // denied to all. Issuing the three orchestrator/* abilities instead
        // costs nothing extra: `orchestrator/deploy.entails(data-layer/admin)`
        // is false, so DDL stays denied exactly as today.
        None => vec![
            Ability::ORCHESTRATOR_DEPLOY,
            Ability::ORCHESTRATOR_UNDEPLOY,
            Ability::ORCHESTRATOR_STATUS,
        ],
    };
    for ability in node_wide_abilities {
        session.capabilities.push(Capability {
            // Bare `substrate:<node_did>` -- the resource is the node
            // itself, not the caller's own DID (B0 named it after the
            // caller; that was inert only because of the is_substrate_scope
            // wildcard, and becomes wrong the moment a selector-bearing
            // resource is evaluated against it, per B7b's F2/F6).
            with: ResourceUri::substrate(node_did),
            can: Ability(ability.to_string()),
            caveats: None,
        });
    }

    // B1 path: verify a presented UCAN chain rooted at the node admin root,
    // addressed to this verified connection identity.
    if let (Some(token), Some(root)) = (&preamble.ucan, admin_root) {
        let is_root = |iss: &str, _res: &ResourceUri| iss == root;
        let opts = ChainVerifyOpts {
            expected_audience_did: &id.master_did,
            is_trusted_root: &is_root,
            now_secs: now,
        };
        match SessionContext::from_verified_chain(token, &opts) {
            Ok(verified) if ucan_chain_not_revoked(token, resolver).await => {
                if !verified.capabilities.is_empty() {
                    auth = AuthLevel::Ucan;
                }
                session.capabilities.extend(verified.capabilities);
                for (k, v) in verified.claims {
                    session.claims.insert(k, v);
                }
            }
            Ok(_) => tracing::warn!("UCAN chain rejected: a chain DID is revoked"),
            Err(e) => tracing::warn!("UCAN chain verification failed: {e}"),
            // Fail-open to Delegated here is deliberate: a bad *authorization*
            // token does not sink an otherwise-verified *transport* identity;
            // the caller simply holds no UCAN capabilities. The admin/native
            // gates then fail closed downstream. (A malformed *delegation*
            // cert is still a hard reject in `handle_stream`, unchanged.)
        }
    }

    CallerContext {
        caller_did: id.master_did.clone(),
        app_instance: None,
        session,
        auth,
        proof: Some(CallerProof {
            pubkey_hex: preamble.pubkey.clone().unwrap_or_default(),
            delegation_json: preamble.delegation.as_ref().and_then(|cert| cert.to_json().ok()),
        }),
    }
}

/// Reads a single line from the reader and parses it as a `RoutePreamble`.
/// Bounded to `MAX_PREAMBLE_LINE_BYTES`: the read happens before any peer
/// authentication, so leaving it unbounded would let an anonymous peer force
/// arbitrary allocation.
pub async fn read_preamble<R>(reader: &mut BufReader<R>) -> Result<RoutePreamble>
where
    R: AsyncRead + Unpin,
{
    let mut raw_preamble = String::new();
    let read = reader.take(MAX_PREAMBLE_LINE_BYTES).read_line(&mut raw_preamble).await?;
    if read == 0 {
        return Err(anyhow!("Stream closed before reading preamble"));
    }
    if !raw_preamble.ends_with('\n') {
        return Err(anyhow!(
            "preamble line exceeds the maximum length of {MAX_PREAMBLE_LINE_BYTES} bytes"
        ));
    }

    RoutePreamble::parse(&raw_preamble)
}

impl RouteHandler {
    /// The main entry point for handling an incoming stream.
    ///
    /// It implements a clean 5-step routing pipeline:
    /// 1. Parse preamble
    /// 2. Registry lookup & normalization
    /// 3. Plan the pipeline stages
    /// 4. Apply encryption stage -> `OwnedStream`
    /// 5. Dispatch by transport stage
    pub async fn handle_stream<S>(self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + StopSignal + 'static,
    {
        // Captured before `io::split` erases the concrete stream type --
        // see `handle_messaging_subscribe`'s dead-subscriber detection.
        let stop_signal = stream.stop_signal();

        // 1. Parse preamble
        let (read_half, write_half) = io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut writer = write_half;

        debug!("[Router] Reading preamble from incoming stream");
        let mut preamble = time::timeout(PRE_AUTH_READ_TIMEOUT, read_preamble(&mut reader))
            .await
            .map_err(|_| anyhow!("timed out reading route preamble"))??;
        debug!(
            "[Router] Preamble received: transport={:?} protocol={:?} interface='{}' \
             service_id='{}' enc={:?} master_did={:?}",
            preamble.transport,
            preamble.protocol,
            preamble.interface,
            preamble.service_id,
            preamble.enc,
            preamble.delegation.as_ref().map(|d| &d.master_did)
        );

        // Handshake verification -- now mandatory for native-capability
        // dispatch. Always attempt; `None` means "no verifiable identity",
        // tolerated only by passthrough/relay paths (which never reach
        // native dispatch) -- the native dispatch arm (dispatch.rs) rejects
        // `None` (ADR-0016 §3).
        let caller = match HandshakeVerifier::verify_preamble(
            &preamble,
            self.inner.registry_client.as_ref(),
        )
        .await
        {
            Ok(id) => Some(
                build_caller(
                    &preamble,
                    &id,
                    self.inner.admin_ucan_root.as_deref(),
                    &self.inner.node_did,
                    self.inner.registry_client.as_ref(),
                )
                .await,
            ),
            Err(e) => {
                // A *malformed* delegation (cert for a different DID,
                // revoked, expired) is still a hard reject here, matching
                // the existing failure test "delegation cert for a
                // different service's DID -> rejected".
                if preamble.delegation.is_some() {
                    tracing::warn!("Handshake verification failed: {e}");
                    let _ = writer.write_all(b"Unauthorized\n").await;
                    let _ = writer.flush().await;
                    return Err(e);
                }
                // No delegation + unverifiable (e.g. missing pubkey) ->
                // anonymous.
                None
            }
        };

        // 2. Registry lookup & normalization
        let lookup_result = self.inner.registry.lookup(&preamble.service_id, &preamble.interface);

        let (endpoint, canonical_interface) = if let Some(res) = lookup_result {
            res
        } else {
            // Community registry / DHT lookup
            debug!(
                "[Router] Local miss for service '{}'. Falling back to community registry / DHT.",
                preamble.service_id
            );

            let next_hop_addr =
                net_iroh::resolve_iroh_addr(&self.inner.registry_client, &preamble.service_id)
                    .await?;

            // 3. Connect outbound to next hop
            let ep = self
                .inner
                .iroh_endpoint
                .as_ref()
                .ok_or_else(|| anyhow!("No Iroh endpoint configured for relay forwarding"))?;
            debug!("[Router] Relay connecting to next hop: {:?}", next_hop_addr.id);
            let conn =
                connect_with_retry(ep, next_hop_addr, SYNEROYM_ALPN, &self.inner.retry_policy)
                    .await?;
            let (mut out_send, out_recv) = conn.open_bi().await?;

            // 4. Send original preamble
            debug!("[Router] Forwarding original preamble: {}", preamble.to_string());
            out_send.write_all(preamble.to_preamble_line().as_bytes()).await?;

            // 5. Blind bidirectional pipe
            let mut inbound = ReaderWriter { reader, writer };
            let mut outbound = IrohStream::new(out_send, out_recv).with_conn(conn);
            if let Err(e) = io::copy_bidirectional(&mut inbound, &mut outbound).await {
                if super::is_expected_disconnect(&e) {
                    debug!(
                        "[Router] Relay tunnel for {} closed by peer ({e})",
                        preamble.service_id
                    );
                } else {
                    return Err(anyhow!("Error in relay copy for {}: {e}", preamble.service_id));
                }
            } else {
                debug!("[Router] Relay copy completed successfully");
            }
            return Ok(());
        };

        preamble.interface = canonical_interface;
        debug!("[Router] Registry lookup complete: endpoint={:?}", endpoint);

        // 3. Plan the pipeline stages
        let pipeline = self.plan_pipeline(&preamble, &endpoint);
        dispatch::log_pipeline(&preamble, &pipeline, &endpoint);

        // 4. Apply encryption stage -> OwnedStream
        let stream = apply_encryption_stage(
            reader,
            writer,
            &pipeline.encryption,
            &preamble,
            &self.inner.identity,
        )
        .await?;

        // 5. Dispatch by transport stage
        match pipeline.transport {
            TransportStage::Raw => self.handle_raw_stream(stream, &preamble, &pipeline).await,
            TransportStage::Http => {
                let io = TokioIo::new(stream);
                self.handle_http_stream(io, preamble, pipeline, caller).await
            }
            TransportStage::Binary => {
                let (r, w) = (stream.reader, stream.writer);
                self.handle_binary_stream(
                    BufReader::new(r),
                    w,
                    &preamble,
                    &pipeline,
                    caller,
                    stop_signal,
                )
                .await
            }
        }
    }

    /// Handles a raw bidirectional stream passthrough to a `ServiceStage`.
    async fn handle_raw_stream(
        &self,
        stream: OwnedStream,
        preamble: &RoutePreamble,
        pipeline: &RoutePipeline,
    ) -> Result<()> {
        match &pipeline.service {
            ServiceStage::TcpProxy { host, port } => {
                debug!("[Router] TcpProxy: connecting to {}:{}", host, port);
                let mut target = TcpStream::connect(format!("{host}:{port}"))
                    .await
                    .map_err(|e| anyhow!("Failed to connect to TCP target {host}:{port}: {e}"))?;
                debug!("[Router] TCP connection to {}:{} established", host, port);

                let mut client = stream;
                if let Err(e) = io::copy_bidirectional(&mut client, &mut target).await {
                    if super::is_expected_disconnect(&e) {
                        debug!("[Router] Proxy tunnel for {}:{} closed by peer ({e})", host, port);
                    } else {
                        return Err(anyhow!("Error in bidirectional copy for {host}:{port}: {e}"));
                    }
                }
                Ok(())
            }
            // M3B Slice 6B bidirectional stream protocols (ADR-0014):
            // `preamble.interface` carries the registered protocol name
            // (the WasmChannel endpoint was resolved via the same registry
            // `register-stream-protocol` writes into -- see the ADR's
            // "Where Registration Lives"). A guest that doesn't export the
            // relevant handler, or declines, is handled inside
            // `AppSandboxEngine::handle_stream_protocol_request` as a clean
            // close, not an error here.
            ServiceStage::WasmComponent { service_id } => {
                self.handle_stream_protocol_request(stream, preamble, service_id).await
            }
            _ => Err(anyhow!(
                "ServiceStage {:?} is not supported for Raw transport",
                pipeline.service
            )),
        }
    }

    /// `dir=` is validated strictly here, before any WASM instantiation
    /// (ADR-0014 item 1) -- a missing or invalid direction is rejected
    /// immediately rather than surfacing later as a confusing WASM-side
    /// failure. The single framed initial payload (the download request
    /// bytes, or the upload's metadata) is read here too, per the ADR's
    /// "one framed frame, then truly raw bytes" contract; everything after
    /// it flows unframed into
    /// `AppSandboxEngine::handle_stream_protocol_request`.
    async fn handle_stream_protocol_request(
        &self,
        stream: OwnedStream,
        preamble: &RoutePreamble,
        service_id: &str,
    ) -> Result<()> {
        const UNKNOWN_PEER_ID: &str = "unknown-peer";

        let Some(dir) = preamble.dir else {
            return Err(anyhow!(
                "raw:// stream request to {service_id}/{} missing or invalid `dir` query \
                 parameter (expected `dir=upload` or `dir=download`)",
                preamble.interface
            ));
        };

        let Some(app_sandbox_engine) = self.inner.app_sandbox_engine.clone() else {
            return Err(anyhow!(
                "app sandbox engine not available (coordinator mode) for stream request to \
                 {service_id}"
            ));
        };

        let peer_id = preamble
            .delegation
            .as_ref()
            .map(|d| d.master_did.clone())
            .unwrap_or_else(|| UNKNOWN_PEER_ID.to_string());

        let ReaderWriter { mut reader, writer } = stream;
        let initial_payload =
            time::timeout(PRE_AUTH_READ_TIMEOUT, framing::read_frame(&mut reader))
                .await
                .map_err(|_| anyhow!("timed out reading stream request's initial payload"))??;

        let outcome = app_sandbox_engine
            .handle_stream_protocol_request(
                service_id,
                &preamble.interface,
                &peer_id,
                dir,
                initial_payload,
                reader,
                writer,
            )
            .await?;
        // The raw-QUIC-stream path has no HTTP-style status code to map a
        // decline onto -- `run_stream_protocol_request` already closes the
        // stream cleanly either way, so the caller here doesn't need to
        // distinguish `Declined` from `Completed` (unlike Slice 7's HTTP
        // chunked-upload bridge, `crates/router/src/route_handler/http.rs`,
        // which maps `Declined` to HTTP 403).
        let _ = outcome;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use syneroym_core::dht_registry::MasterAnchorPayload;
    use syneroym_identity::{Identity, substrate::derive_did_key};
    use tokio::io::duplex;

    use super::*;

    /// A `MasterAnchorResolver` double whose `revoked_keys` are configured
    /// per-issuer, mirroring `handshake.rs`'s own test `MockResolver`.
    struct MockResolver {
        revoked: HashMap<String, Vec<String>>,
    }

    #[async_trait::async_trait]
    impl MasterAnchorResolver for MockResolver {
        async fn resolve_master_anchor(
            &self,
            master_id: &str,
        ) -> Result<MasterAnchorPayload, anyhow::Error> {
            let mut anchor = MasterAnchorPayload::default();
            if let Some(revoked) = self.revoked.get(master_id) {
                anchor.revoked_keys = revoked.clone();
            }
            Ok(anchor)
        }
    }

    /// A `MasterAnchorResolver` double that counts calls, used to verify
    /// `ucan_chain_not_revoked` de-duplicates identical `(issuer, audience)`
    /// edges rather than resolving each occurrence independently.
    #[derive(Default)]
    struct CountingResolver {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl MasterAnchorResolver for CountingResolver {
        async fn resolve_master_anchor(
            &self,
            _master_id: &str,
        ) -> Result<MasterAnchorPayload, anyhow::Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(MasterAnchorPayload::default())
        }
    }

    fn ucan_preamble(token: CapabilityToken) -> RoutePreamble {
        let mut preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        preamble.ucan = Some(token);
        preamble
    }

    /// Step 21 (reference scenario): a client presents a UCAN rooted at the
    /// node's admin root -> `build_caller` verifies the chain and the
    /// normalized capability lands in `CallerContext.session`, with `auth`
    /// upgraded to `Ucan`.
    #[tokio::test]
    async fn build_caller_admits_a_ucan_chain_rooted_at_admin_root() {
        let owner = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let admin_root = derive_did_key(&owner.public_key());
        let client_did = derive_did_key(&client.public_key());
        let resource = ResourceUri::service("app1", "svc1");

        let token = CapabilityToken::issue(
            &owner,
            &client_did,
            vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![],
        )
        .unwrap();

        let preamble = ucan_preamble(token);
        let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
        let resolver = MockResolver { revoked: HashMap::new() };

        let caller =
            build_caller(&preamble, &id, Some(&admin_root), "did:key:zNode", &resolver).await;

        assert_eq!(caller.auth, AuthLevel::Ucan);
        assert_eq!(caller.caller_did, id.master_did);
        assert!(
            caller
                .session
                .has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
        );
    }

    /// The same claim as
    /// `build_caller_admits_a_ucan_chain_rooted_at_admin_root`, but driven
    /// end to end from wire bytes rather than a hand-built `RoutePreamble`/
    /// `VerifiedIdentity`: the preamble is serialized then re-`parse`d
    /// (exercising the `ucan=`/`pubkey=` hex decode a real peer's
    /// bytes would go through), and the `VerifiedIdentity` comes from
    /// `HandshakeVerifier::verify_preamble` (the same call `handle_stream`
    /// makes) rather than being constructed directly.
    #[tokio::test]
    async fn parsed_wire_preamble_with_ucan_reaches_build_caller() {
        let owner = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let admin_root = derive_did_key(&owner.public_key());
        let client_did = derive_did_key(&client.public_key());
        let resource = ResourceUri::service("app1", "svc1");

        let token = CapabilityToken::issue(
            &owner,
            &client_did,
            vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![],
        )
        .unwrap();

        let mut preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        preamble.pubkey = Some(hex::encode(client.public_key().to_bytes()));
        preamble.ucan = Some(token);

        // Round-trip through the actual wire format, not the struct directly.
        let wire_line = preamble.to_preamble_line();
        let parsed = RoutePreamble::parse(&wire_line).unwrap();
        assert!(parsed.ucan.is_some(), "ucan= must survive the hex-encode/decode round trip");

        let resolver = MockResolver { revoked: HashMap::new() };
        let verified_id = HandshakeVerifier::verify_preamble(&parsed, &resolver)
            .await
            .expect("a self-asserted pubkey with no delegation cert must verify");
        assert_eq!(verified_id.master_did, client_did);

        let caller =
            build_caller(&parsed, &verified_id, Some(&admin_root), "did:key:zNode", &resolver)
                .await;

        assert_eq!(caller.auth, AuthLevel::Ucan);
        assert!(
            caller
                .session
                .has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
        );
    }

    /// A UCAN presented by a DID other than the one it's addressed to (the
    /// verified connection identity != `token.audience_did`) fails
    /// structural verification: no capability is admitted, and `auth` stays
    /// at the pre-UCAN `Delegated` level (fail-open on the transport
    /// identity, fail-closed on the bad authorization token).
    #[tokio::test]
    async fn build_caller_rejects_audience_mismatch() {
        let owner = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let impostor = Identity::generate().unwrap();
        let admin_root = derive_did_key(&owner.public_key());
        let client_did = derive_did_key(&client.public_key());
        let impostor_did = derive_did_key(&impostor.public_key());
        let resource = ResourceUri::service("app1", "svc1");

        let token = CapabilityToken::issue(
            &owner,
            &client_did,
            vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![],
        )
        .unwrap();

        let preamble = ucan_preamble(token);
        let id = VerifiedIdentity { master_did: impostor_did.clone(), temporary_did: impostor_did };
        let resolver = MockResolver { revoked: HashMap::new() };

        let caller =
            build_caller(&preamble, &id, Some(&admin_root), "did:key:zNode", &resolver).await;

        assert_eq!(caller.auth, AuthLevel::Delegated);
        assert!(
            !caller
                .session
                .has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
        );
    }

    /// A chain rooted at an issuer that is not the node's admin root grants
    /// nothing -- B1 has no other trust root (owner-rooted service chains
    /// are Slice B7). `auth` must not upgrade to `Ucan` either: the chain
    /// verified structurally but admitted zero capabilities, so it holds no
    /// more privilege than the pre-UCAN `Delegated` level.
    #[tokio::test]
    async fn build_caller_drops_capabilities_from_an_untrusted_root() {
        let owner = Identity::generate().unwrap();
        let alice = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let admin_root = derive_did_key(&owner.public_key());
        let client_did = derive_did_key(&client.public_key());
        let resource = ResourceUri::service("app1", "svc1");

        // Issued by `alice`, who is not the admin root.
        let token = CapabilityToken::issue(
            &alice,
            &client_did,
            vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![],
        )
        .unwrap();

        let preamble = ucan_preamble(token);
        let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
        let resolver = MockResolver { revoked: HashMap::new() };

        let caller =
            build_caller(&preamble, &id, Some(&admin_root), "did:key:zNode", &resolver).await;

        assert_eq!(caller.auth, AuthLevel::Delegated);
        assert!(
            !caller
                .session
                .has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
        );
    }

    /// A structurally valid chain whose audience DID has been revoked by
    /// the issuer's master anchor is rejected wholesale: no capability is
    /// admitted and `auth` does not upgrade to `Ucan`.
    #[tokio::test]
    async fn build_caller_rejects_a_revoked_chain() {
        let owner = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let admin_root = derive_did_key(&owner.public_key());
        let client_did = derive_did_key(&client.public_key());
        let resource = ResourceUri::service("app1", "svc1");

        let token = CapabilityToken::issue(
            &owner,
            &client_did,
            vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![],
        )
        .unwrap();

        let preamble = ucan_preamble(token);
        let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
        let resolver = MockResolver {
            revoked: HashMap::from([(admin_root.clone(), vec![id.master_did.clone()])]),
        };

        let caller =
            build_caller(&preamble, &id, Some(&admin_root), "did:key:zNode", &resolver).await;

        assert_eq!(caller.auth, AuthLevel::Delegated);
        assert!(
            !caller
                .session
                .has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
        );
    }

    /// A chain that reuses the same proof twice (a diamond shape) must
    /// resolve each distinct `(issuer, audience)` edge only once, not once
    /// per occurrence.
    #[tokio::test]
    async fn ucan_chain_not_revoked_dedupes_repeated_edges() {
        let owner = Identity::generate().unwrap();
        let alice = Identity::generate().unwrap();
        let bob = Identity::generate().unwrap();
        let alice_did = derive_did_key(&alice.public_key());
        let bob_did = derive_did_key(&bob.public_key());
        let resource = ResourceUri::service("app1", "svc1");

        let owner_to_alice = CapabilityToken::issue(
            &owner,
            &alice_did,
            vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![],
        )
        .unwrap();
        // The same proof embedded twice.
        let alice_to_bob = CapabilityToken::issue(
            &alice,
            &bob_did,
            vec![Capability {
                with: resource,
                can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
                caveats: None,
            }],
            serde_json::Map::new(),
            3600,
            vec![owner_to_alice.clone(), owner_to_alice],
        )
        .unwrap();

        let resolver = CountingResolver::default();
        assert!(ucan_chain_not_revoked(&alice_to_bob, &resolver).await);
        assert_eq!(
            resolver.calls.load(Ordering::SeqCst),
            2,
            "expected exactly one resolution each for the (alice, bob) and (owner, alice) edges, \
             despite (owner, alice) appearing twice"
        );
    }

    /// A peer that never sends anything must not hold `read_preamble` open
    /// forever -- with tokio's paused virtual clock this fires effectively
    /// instantly rather than costing real wall-clock time.
    #[tokio::test(start_paused = true)]
    async fn test_read_preamble_times_out_on_idle_stream() {
        let (client, server) = duplex(64);
        let mut reader = BufReader::new(server);

        let result = time::timeout(PRE_AUTH_READ_TIMEOUT, read_preamble(&mut reader)).await;

        assert!(result.is_err(), "expected a timeout since the peer never wrote a preamble");
        drop(client);
    }

    /// An unauthenticated peer sending a line with no newline anywhere
    /// within `MAX_PREAMBLE_LINE_BYTES` must be rejected, not read into
    /// memory without bound.
    #[tokio::test]
    async fn test_read_preamble_rejects_oversized_line() {
        let oversized_len = MAX_PREAMBLE_LINE_BYTES as usize + 1024;
        // Large enough that `write_all` completes without needing a
        // concurrent reader to drain it.
        let (mut client, server) = duplex(oversized_len + 1024);
        let mut reader = BufReader::new(server);

        client.write_all(&vec![b'a'; oversized_len]).await.unwrap();

        let err = read_preamble(&mut reader).await.unwrap_err();
        assert!(
            err.to_string().contains("exceeds the maximum length"),
            "expected the oversized-line error, got: {err}"
        );
    }

    /// `Take`'s capped view into the reader's buffered data must not cause
    /// bytes **after** the newline to be consumed/discarded -- only
    /// `read_line`'s own delimiter scan drives how much of the underlying
    /// buffer is actually consumed, so a pipelined payload following the
    /// preamble line in the same write (e.g. the initial framed request)
    /// must remain intact and correctly positioned for the next read.
    #[tokio::test]
    async fn test_read_preamble_preserves_bytes_after_the_line() {
        let (mut client, server) = duplex(4096);
        let mut reader = BufReader::new(server);

        let mut sent = b"json-rpc://health|substrate-123\n".to_vec();
        sent.extend_from_slice(b"TRAILING-PAYLOAD");
        client.write_all(&sent).await.unwrap();

        let preamble = read_preamble(&mut reader).await.unwrap();
        assert_eq!(preamble.service_id, "substrate-123");

        let mut trailing = vec![0u8; b"TRAILING-PAYLOAD".len()];
        reader.read_exact(&mut trailing).await.unwrap();
        assert_eq!(&trailing, b"TRAILING-PAYLOAD");
    }

    /// A peer that promptly sends a valid preamble line must not be
    /// penalized by the timeout wrapper.
    #[tokio::test]
    async fn test_read_preamble_succeeds_within_timeout() {
        let (mut client, server) = duplex(64);
        let mut reader = BufReader::new(server);

        client.write_all(b"json-rpc://health|substrate-123\n").await.unwrap();

        let result = time::timeout(PRE_AUTH_READ_TIMEOUT, read_preamble(&mut reader)).await;

        let preamble = result.expect("must not time out on a promptly-sent preamble").unwrap();
        assert_eq!(preamble.service_id, "substrate-123");
    }

    /// M04A Slice B7a (F4): on an unowned substrate (`admin_root: None`),
    /// every verified caller holds the three `orchestrator/*` abilities on
    /// the bare `substrate:<node_did>` resource -- the bootstrap posture.
    #[tokio::test]
    async fn unowned_substrate_grants_orchestrator_abilities_to_any_verified_caller() {
        let client = Identity::generate().unwrap();
        let client_did = derive_did_key(&client.public_key());
        let node_did = "did:key:zNodeUnowned";
        let node_resource = ResourceUri::substrate(node_did);

        let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
        let resolver = MockResolver { revoked: HashMap::new() };

        let caller = build_caller(&preamble, &id, None, node_did, &resolver).await;

        for ability in [
            Ability::ORCHESTRATOR_DEPLOY,
            Ability::ORCHESTRATOR_UNDEPLOY,
            Ability::ORCHESTRATOR_STATUS,
        ] {
            assert!(
                caller.has_capability(&node_resource, &Ability(ability.to_string())),
                "expected unowned substrate to grant {ability}"
            );
        }
    }

    /// The regression test for F4's over-grant trap: an unowned substrate
    /// must NOT grant `data-layer/admin` (or `substrate/admin`, which would
    /// entail it) to a verified caller -- `execute-ddl`/`query-raw` stay
    /// denied exactly as today. This is B7a's single most important test.
    #[tokio::test]
    async fn unowned_substrate_does_not_grant_data_layer_admin() {
        let client = Identity::generate().unwrap();
        let client_did = derive_did_key(&client.public_key());
        let node_did = "did:key:zNodeUnowned";
        let some_service = ResourceUri::service("app-1", "svc-a");

        let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
        let resolver = MockResolver { revoked: HashMap::new() };

        let caller = build_caller(&preamble, &id, None, node_did, &resolver).await;

        assert!(
            !caller.has_capability(&some_service, &Ability(Ability::DATA_LAYER_ADMIN.to_string())),
            "unowned substrate must not grant data-layer/admin -- the over-grant trap"
        );
        assert!(!caller.has_capability(
            &ResourceUri::substrate(node_did),
            &Ability(Ability::SUBSTRATE_ADMIN.to_string())
        ));
    }

    /// On an owned substrate, only the caller whose DID equals `admin_root`
    /// gets `substrate/admin`; anyone else gets no node-wide capability at
    /// all.
    #[tokio::test]
    async fn owned_substrate_grants_substrate_admin_only_to_the_owner() {
        let owner = Identity::generate().unwrap();
        let other = Identity::generate().unwrap();
        let owner_did = derive_did_key(&owner.public_key());
        let other_did = derive_did_key(&other.public_key());
        let node_did = "did:key:zNodeOwned";
        let node_resource = ResourceUri::substrate(node_did);

        let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        let resolver = MockResolver { revoked: HashMap::new() };

        let owner_id =
            VerifiedIdentity { master_did: owner_did.clone(), temporary_did: owner_did.clone() };
        let owner_caller =
            build_caller(&preamble, &owner_id, Some(&owner_did), node_did, &resolver).await;
        assert!(
            owner_caller
                .has_capability(&node_resource, &Ability(Ability::SUBSTRATE_ADMIN.to_string()))
        );

        let other_id = VerifiedIdentity { master_did: other_did.clone(), temporary_did: other_did };
        let other_caller =
            build_caller(&preamble, &other_id, Some(&owner_did), node_did, &resolver).await;
        assert!(
            !other_caller
                .has_capability(&node_resource, &Ability(Ability::SUBSTRATE_ADMIN.to_string()))
        );
        for ability in [
            Ability::ORCHESTRATOR_DEPLOY,
            Ability::ORCHESTRATOR_UNDEPLOY,
            Ability::ORCHESTRATOR_STATUS,
        ] {
            assert!(
                !other_caller.has_capability(&node_resource, &Ability(ability.to_string())),
                "an owned substrate must not fall back to the unowned grant for a non-owner"
            );
        }
    }

    /// The owner's `substrate/admin` capability names the node's own DID,
    /// not the caller's -- a B0 naming quirk that stayed inert only because
    /// of the `is_substrate_scope` wildcard (F2/F6, B7b).
    #[tokio::test]
    async fn substrate_admin_capability_names_the_node_not_the_caller() {
        let owner = Identity::generate().unwrap();
        let owner_did = derive_did_key(&owner.public_key());
        let node_did = "did:key:zNodeOwned";

        let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        let resolver = MockResolver { revoked: HashMap::new() };
        let id =
            VerifiedIdentity { master_did: owner_did.clone(), temporary_did: owner_did.clone() };

        let caller = build_caller(&preamble, &id, Some(&owner_did), node_did, &resolver).await;

        assert!(
            caller.session.capabilities.iter().any(|c| c.with == ResourceUri::substrate(node_did)),
            "expected the granted capability's resource to name the node DID"
        );
        assert!(
            !caller
                .session
                .capabilities
                .iter()
                .any(|c| c.with == ResourceUri::substrate(&owner_did)),
            "must not name the caller's own DID (which happens to equal the owner here, but the \
             resource must be node-scoped, not caller-scoped)"
        );
    }

    /// Post-commit review (B7a): task.md item 3 / F11 requires attribution
    /// to resolve to the delegation's `master_did`, not the ephemeral
    /// `temporary_did` -- the DID `ControlPlaneService::deploy` later
    /// records as a service's owner. Every other test in this module
    /// constructs `VerifiedIdentity { master_did == temporary_did }`, so none
    /// can actually distinguish a bug that swapped the two;
    /// `handshake.rs`'s own tests prove `HandshakeVerifier::verify_preamble`
    /// resolves a real wire handshake correctly, but nothing previously
    /// exercised `build_caller` itself with a genuinely distinct pair.
    #[tokio::test]
    async fn build_caller_uses_master_did_not_temporary_did_as_caller_did() {
        let client = Identity::generate().unwrap();
        let master_did = derive_did_key(&client.public_key());
        let temporary_did = "did:key:zSomeEphemeralTemporaryKey".to_string();

        let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
        let id = VerifiedIdentity {
            master_did: master_did.clone(),
            temporary_did: temporary_did.clone(),
        };
        let resolver = MockResolver { revoked: HashMap::new() };

        let caller = build_caller(&preamble, &id, None, "did:key:zNode", &resolver).await;

        assert_eq!(caller.caller_did, master_did);
        assert_ne!(caller.caller_did, temporary_did);
    }
}
