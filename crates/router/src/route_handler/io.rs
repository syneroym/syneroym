//! Async I/O copy loops and bridge utilities
//!
//! Handles bidirectional copy tasks and framing adapters for bridged streams.

use super::RouteHandler;
use crate::preamble::RoutePreamble;
use crate::route_handler::encryption::{OwnedStream, apply_encryption_stage};
use crate::routing::{RoutePipeline, ServiceStage, TransportStage};
use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tracing::debug;

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
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        // 1. Parse preamble
        let (read_half, write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let writer = write_half;

        debug!("[Router] Reading preamble from incoming stream");
        let mut preamble = read_preamble(&mut reader).await?;
        debug!(
            "[Router] Preamble received: transport={:?} protocol={:?} interface='{}' service_id='{}' enc={:?}",
            preamble.transport,
            preamble.protocol,
            preamble.interface,
            preamble.service_id,
            preamble.enc
        );

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

            let info = self.inner.registry_client.lookup(&preamble.service_id, true).await?;

            // 2. Extract Iroh EndpointAddr from mechanisms
            let mut iroh_addr = None;
            for mech in info.info.mechanisms {
                if let syneroym_core::community_registry::EndpointMechanism::Iroh {
                    endpoint_addr_bytes,
                    relay_url,
                } = mech
                {
                    let mut addr: iroh::EndpointAddr =
                        serde_json::from_slice(&endpoint_addr_bytes)?;
                    if let Some(r_url_str) = relay_url
                        && let Ok(relay_url) = r_url_str.parse::<iroh::RelayUrl>()
                    {
                        addr = addr.with_relay_url(relay_url);
                    }
                    iroh_addr = Some(addr);
                    break;
                }
            }

            let next_hop_addr = iroh_addr
                .ok_or_else(|| anyhow!("No valid Iroh mechanism found for next hop in registry"))?;

            // 3. Connect outbound to next hop
            let ep = self
                .inner
                .iroh_endpoint
                .as_ref()
                .ok_or_else(|| anyhow!("No Iroh endpoint configured for relay forwarding"))?;
            debug!("[Router] Relay connecting to next hop: {:?}", next_hop_addr.id);
            let conn = ep.connect(next_hop_addr, super::super::SYNEROYM_ALPN).await?;
            let (mut out_send, out_recv) = conn.open_bi().await?;

            // 4. Send original preamble
            debug!("[Router] Forwarding original preamble: {}", preamble.to_string());
            out_send.write_all(preamble.to_preamble_line().as_bytes()).await?;

            // 5. Blind bidirectional pipe
            let mut inbound = super::encryption::ReaderWriter { reader, writer };
            let mut outbound = crate::net_iroh::IrohStream::new(out_send, out_recv);
            tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;
            debug!("[Router] Relay copy completed successfully");
            return Ok(());
        };

        preamble.interface = canonical_interface;
        debug!("[Router] Registry lookup complete: endpoint={:?}", endpoint);

        // 3. Plan the pipeline stages
        let pipeline = self.plan_pipeline(&preamble, &endpoint);
        super::dispatch::log_pipeline(&preamble, &pipeline, &endpoint);

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
            TransportStage::Raw => self.handle_raw_stream(stream, &pipeline).await,
            TransportStage::Http => {
                let io = TokioIo::new(stream);
                self.handle_http_stream(io, preamble, pipeline).await
            }
            TransportStage::Binary => {
                let (r, mut w) = (stream.reader, stream.writer);
                self.handle_json_rpc_loop(BufReader::new(r), &mut w, &preamble, &pipeline).await
            }
        }
    }

    /// Handles a raw bidirectional stream passthrough to a `ServiceStage`.
    async fn handle_raw_stream(&self, stream: OwnedStream, pipeline: &RoutePipeline) -> Result<()> {
        match &pipeline.service {
            ServiceStage::TcpProxy { host, port } => {
                debug!("[Router] TcpProxy: connecting to {}:{}", host, port);
                let mut target = tokio::net::TcpStream::connect(format!("{host}:{port}"))
                    .await
                    .map_err(|e| anyhow!("Failed to connect to TCP target {host}:{port}: {e}"))?;
                debug!("[Router] TCP connection to {}:{} established", host, port);

                let mut client = stream;
                tokio::io::copy_bidirectional(&mut client, &mut target)
                    .await
                    .map_err(|e| anyhow!("Error in bidirectional copy for {host}:{port}: {e}"))?;
                Ok(())
            }
            ServiceStage::WasmComponent { service_id } => {
                // TODO(wRPC): passthrough not yet implemented
                debug!("Passthrough stream to Wasm channel: {}", service_id);
                tracing::warn!(
                    service_id = %service_id,
                    "wRPC passthrough not yet implemented; dropping stream"
                );
                Ok(())
            }
            _ => Err(anyhow!(
                "ServiceStage {:?} is not supported for Raw transport",
                pipeline.service
            )),
        }
    }
}
