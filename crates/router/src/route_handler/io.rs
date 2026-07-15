//! Async I/O copy loops and bridge utilities
//!
//! Handles bidirectional copy tasks and framing adapters for bridged streams.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use syneroym_rpc::{Ability, CallerContext, Capability, ResourceUri, SessionContext, framing};
use tokio::{
    io,
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
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

use super::{super::SYNEROYM_ALPN, RouteHandler, dispatch, encryption::ReaderWriter};
use crate::{
    handshake::{HandshakeVerifier, VerifiedIdentity},
    net_iroh,
    net_iroh::{IrohStream, connect_with_retry},
    preamble::RoutePreamble,
    route_handler::encryption::{OwnedStream, apply_encryption_stage},
    routing::{RoutePipeline, ServiceStage, TransportStage},
    stop_signal::StopSignal,
};

/// Builds the `CallerContext` for a verified handshake identity (ADR-0016
/// §4.2). A caller whose master DID equals the configured
/// `[iam].admin_ucan_root` is granted `substrate/admin` on this node --
/// interim path until Slice B1 wires full UCAN chain verification.
///
/// TODO(M04B/FDAE): B0 gate only proves *an* identity is present. Which
/// callers may actually reach a given native service (service-owner /
/// substrate-owner) and with what row/column scope is enforced by the FDAE
/// policy engine (M04B), evaluated against `caller.session`. Until then any
/// verified identity passes.
fn build_caller(
    preamble: &RoutePreamble,
    id: &VerifiedIdentity,
    admin_root: Option<&str>,
) -> CallerContext {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut session = SessionContext {
        subject_did: id.master_did.clone(),
        verified_at_secs: now,
        ..Default::default()
    };
    if admin_root == Some(id.master_did.as_str()) {
        session.capabilities.push(Capability {
            with: ResourceUri::substrate(&id.master_did),
            can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
            caveats: None,
        });
    }
    CallerContext {
        caller_did: id.master_did.clone(),
        app_instance: None,
        session,
        auth: syneroym_rpc::AuthLevel::Delegated,
        proof: Some(syneroym_rpc::CallerProof {
            pubkey_hex: preamble.pubkey.clone().unwrap_or_default(),
            delegation_json: preamble.delegation.as_ref().and_then(|cert| cert.to_json().ok()),
        }),
    }
}

/// Reads a single line from the reader and parses it as a `RoutePreamble`.
pub async fn read_preamble<R>(reader: &mut BufReader<R>) -> Result<RoutePreamble>
where
    R: AsyncRead + Unpin,
{
    let mut raw_preamble = String::new();
    let read = reader.read_line(&mut raw_preamble).await?;
    if read == 0 {
        return Err(anyhow!("Stream closed before reading preamble"));
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
            Ok(id) => Some(build_caller(&preamble, &id, self.inner.admin_ucan_root.as_deref())),
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
    use tokio::io::duplex;

    use super::*;

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
}
