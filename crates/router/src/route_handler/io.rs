//! Async I/O copy loops and bridge utilities
//!
//! Handles bidirectional copy tasks and framing adapters for bridged streams.

use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use syneroym_core::local_registry::{EndpointRegistry, HTTP_NATIVE_INTERFACE, SubstrateEndpoint};
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
    preamble::{RoutePreamble, RouteTransport},
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

/// Whether `res`'s named substrate node is
/// *this* node (ADR-0015 A6). `synapp:...` service resources are always
/// local (evaluated locally by construction -- a cross-node proxy hop
/// re-verifies with a fresh `CallerContext` at the destination, per
/// ADR-0016 §6); a `substrate:<node_did>[/selector]` resource is local
/// only when it
/// names this node's own DID. This is the node-locality half of
/// `is_trusted_root`; the per-service half is `owning_service_id` below.
fn resource_is_local(res: &ResourceUri, node_did: &str) -> bool {
    match res.0.strip_prefix("substrate:") {
        Some(rest) => rest.split('/').next().unwrap_or(rest) == node_did,
        None => true,
    }
}

/// The `service_id` a resource names (ADR-0015 A6), for the
/// per-service `owner_of` root check -- both forms a live resource can take:
/// `synapp:<app_instance_id>:svc:<service_id>[/selector]` (in practice
/// `app_instance_id == service_id` today, since `CallerContext.app_instance`
/// is always `None` -- see `synsvc_native.rs`) and the orchestrator's
/// `substrate:<node_did>/app/<service_id>[...]` selector. `None` for every
/// other resource shape (e.g. a bare `substrate:<node_did>`, which
/// per-service ownership does not apply to).
fn owning_service_id(res: &ResourceUri) -> Option<&str> {
    if let Some(rest) = res.0.strip_prefix("synapp:") {
        let (_, after_svc) = rest.split_once(":svc:")?;
        return Some(after_svc.split('/').next().unwrap_or(after_svc));
    }
    if let Some(rest) = res.0.strip_prefix("substrate:") {
        let (_, selector) = rest.split_once('/')?;
        let mut parts = selector.split('/');
        if parts.next() == Some("app") {
            return parts.next();
        }
    }
    None
}

/// Builds the `CallerContext` for a verified handshake identity
/// (ADR-0016 §4.2). A caller whose master DID equals the configured
/// `[iam].admin_ucan_root` is granted `substrate/admin` on this node (the
/// direct-equality path). A presented `preamble.ucan` chain is
/// additionally verified and merged in -- rooted either at that same admin
/// root (node-wide, and only for a resource this node actually names)
/// or at a resource's own recorded owner (ADR-0015 A6: a
/// service owner is an independent root for their own service, regardless of
/// whether the substrate itself has an admin root at all) -- except for
/// `data-layer/admin` (or anything entailing it): an owner cannot self-root
/// that ability on their own service, only the node admin root can, so a
/// self-issued `data-layer/admin` grant does not silently open
/// `execute-ddl`/`query-raw` on every deployment
/// (`is_trusted_root`'s resource-only predicate was ability-agnostic and
/// would otherwise have admitted it; see
/// `unowned_substrate_does_not_grant_data_layer_admin`'s
/// sibling `owner_rooted_chain_does_not_grant_data_layer_admin`). `auth` is
/// upgraded to `AuthLevel::Ucan` only when the chain actually admitted at
/// least one capability -- a structurally valid but entirely untrusted chain
/// (e.g. self-issued, rooted nowhere) must not read as "holds a verified UCAN
/// capability" to any future code that checks `auth == Ucan` as a privilege
/// signal.
///
/// TODO: the identity gate only proves *an* identity is present.
/// "May this caller touch this service at all?" is Tier 1 -- a µs-scale
/// grant-layer capability check, NOT an FDAE policy question (ADR-0017
/// Open). The deploy-grant work implements it for `orchestrator`;
/// `security` is also gated on `substrate/admin`. The five data
/// native-capability interfaces remain
/// open -- today any verified identity reaches any native service there.
/// FDAE owns Tier 3 (rows/columns) only.
/// The `None` rejection below is correct and settled:
/// native interfaces reject anonymous callers, WASM guests admit them.
async fn build_caller(
    preamble: &RoutePreamble,
    id: &VerifiedIdentity,
    admin_root: Option<&str>,
    node_did: &str,
    registry: &EndpointRegistry,
    resolver: &dyn MasterAnchorResolver,
    grant_resolve_to_node_did: bool,
) -> CallerContext {
    let now = now_secs();
    let mut session = SessionContext {
        subject_did: id.master_did.clone(),
        verified_at_secs: now,
        ..Default::default()
    };
    let mut auth = AuthLevel::Delegated;

    // The substrate-owner capability is issued from this single site (there
    // is no "is this substrate owned?" branch anywhere
    // downstream). The unowned bootstrap
    // grant that used to sit here is now removed: an unowned substrate issued
    // `orchestrator/{deploy,undeploy,status}` to every verified caller,
    // which was defensible while one operator hand-deployed to their own
    // node and is not once substrates are unattended networked deploy
    // targets. Bootstrap now happens off the wire entirely -- `roymctl
    // substrate claim` mints a `ControllerAgreement` from the node's own
    // key file on the node's own host -- so an unowned substrate can fail
    // closed without becoming unrecoverable.
    if admin_root == Some(id.master_did.as_str()) {
        session.capabilities.push(Capability {
            // Bare `substrate:<node_did>` -- the node itself, node-wide.
            with: ResourceUri::substrate(node_did),
            can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
            caveats: None,
        });
    }

    // The same-node resolve grant. A caller whose verified DID
    // is this node's own is granted a bare `substrate:<node_did>`
    // capability whose ability is `supervisor/resolve`, deliberately not
    // `substrate/admin` -- the bare resource short-circuits
    // `Capability::grants` and so covers `synapp:<any-app-did>`, but the
    // ability check still gates it to resolution alone. This is what lets
    // a same-node client gateway or WebRTC coordinator resolve a logical
    // (`-a…-s…`) hostname for an app supervised here with no credential
    // file (ADR-0022 §7).
    if grant_resolve_to_node_did && id.master_did == node_did {
        session.capabilities.push(Capability {
            with: ResourceUri::substrate(node_did),
            can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
            caveats: None,
        });
    }

    // B1/B7b path: verify a presented UCAN chain addressed to this verified
    // connection identity, rooted either at the node admin root (node-wide,
    // and only for a resource this node actually names -- F6) or at a
    // resource's own recorded owner (ADR-0015 A6). Runs regardless of
    // whether this substrate has an admin root at all: an owner-rooted
    // per-service chain is independent of node-wide ownership.
    if let Some(token) = &preamble.ucan {
        let is_root = |iss: &str, cap: &Capability| {
            (admin_root == Some(iss) && resource_is_local(&cap.with, node_did))
                || (owning_service_id(&cap.with)
                    .is_some_and(|svc| registry.owner_of(svc).as_deref() == Some(iss))
                    && !cap.can.entails(&Ability(Ability::DATA_LAYER_ADMIN.to_string())))
        };
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
                session.anchor_did = verified.anchor_did;
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
                    &self.inner.registry,
                    self.inner.registry_client.as_ref(),
                    self.inner.grant_resolve_to_node_did,
                )
                .await,
            ),
            Err(e) => {
                // A *malformed* delegation is still a hard reject here: a
                // certificate whose `temporary_did` doesn't match the
                // preamble's own pubkey, that's expired, revoked, or carries
                // a scope outside TRANSPORT_SCOPES.
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
        //
        // An HTTP request with no interface hint (ADR-0022 §7's hostname omits
        // `-i`) must reach the inbound HTTP bridge when the service declares
        // guest routes or a static asset bundle: the bridge serves those, and
        // serving an asset dispatches `blob-store/open-download` -- a
        // native-capability interface, so the connection has to land on the
        // service's `http-native` `NativeHostChannel`. `resolve_interface`'s
        // empty case filters every native-capability name out, so left alone it
        // picks the app-declared WASM channel and the asset dispatch is
        // misrouted into the component (which imports, but never exports,
        // `blob-store`). Prefer `http-native` here -- but only when the empty
        // interface would otherwise resolve to a `WasmChannel`. A node-level
        // native service such as the auth service resolves its empty interface
        // to its own `NativeHostChannel` and must be left alone.
        if preamble.transport == RouteTransport::Http && preamble.interface.is_empty() {
            let service_id = preamble.service_id.as_str();
            let has_routes_or_assets = self.inner.http_routes.contains_key(service_id)
                || self.inner.assets.contains_key(service_id);
            if has_routes_or_assets {
                let empty_ep = self.inner.registry.lookup(service_id, "").map(|(ep, _)| ep);
                maybe_rewrite_http_native_interface(
                    preamble.transport,
                    &mut preamble.interface,
                    has_routes_or_assets,
                    empty_ep.as_ref(),
                );
            }
        }

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
            // Bidirectional stream protocols (ADR-0014):
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
        // distinguish `Declined` from `Completed` (unlike the HTTP
        // chunked-upload bridge, `crates/router/src/route_handler/http.rs`,
        // which maps `Declined` to HTTP 403).
        let _ = outcome;
        Ok(())
    }
}

pub(crate) fn maybe_rewrite_http_native_interface(
    transport: RouteTransport,
    interface: &mut String,
    has_http_routes_or_assets: bool,
    empty_interface_endpoint: Option<&SubstrateEndpoint>,
) -> bool {
    let bridged_over_http = transport == RouteTransport::Http
        && interface.is_empty()
        && has_http_routes_or_assets
        && matches!(empty_interface_endpoint, Some(SubstrateEndpoint::WasmChannel { .. }));
    if bridged_over_http {
        *interface = HTTP_NATIVE_INTERFACE.to_string();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
