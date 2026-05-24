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
    /// 4. Apply encryption stage -> OwnedStream
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
        let (endpoint, canonical_interface) =
            self.inner.registry.lookup(&preamble.service_id, &preamble.interface).ok_or_else(
                || {
                    anyhow!(
                        "Interface '{}' not found for service '{}'",
                        preamble.interface,
                        preamble.service_id
                    )
                },
            )?;

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

    /// Handles a raw bidirectional stream passthrough to a ServiceStage.
    async fn handle_raw_stream(&self, stream: OwnedStream, pipeline: &RoutePipeline) -> Result<()> {
        match &pipeline.service {
            ServiceStage::TcpProxy { host, port } => {
                debug!("[Router] TcpProxy: connecting to {}:{}", host, port);
                let mut target = tokio::net::TcpStream::connect(format!("{}:{}", host, port))
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
                // wRPC passthrough
                debug!("Passthrough wRPC stream to Wasm channel: {}", service_id);
                tracing::warn!(
                    service_id = %service_id,
                    "wRPC passthrough not yet implemented; dropping stream"
                );
                Ok(())
            }
            ServiceStage::RelayHop { next_hop_id } => {
                tracing::warn!(
                    next_hop_id = ?next_hop_id,
                    "RelayHop raw passthrough is not implemented yet; dropping stream"
                );
                Err(anyhow!("RelayHop raw passthrough not implemented"))
            }
            _ => Err(anyhow!(
                "ServiceStage {:?} is not supported for Raw transport",
                pipeline.service
            )),
        }
    }
}
