use super::RouteHandler;
use crate::preamble::{RoutePreamble, RouteTransport};
use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader, ReadBuf};
use tracing::debug;

/// A simple wrapper that combines an `AsyncRead` and `AsyncWrite` into a single type.
///
/// This is useful when you have split a stream into halves but need to pass them
/// as a single object to a library like `hyper`.
pub struct ReaderWriter<R, W> {
    pub reader: R,
    pub writer: W,
}

impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for ReaderWriter<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for ReaderWriter<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
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
    /// It first reads the `RoutePreamble` to determine the transport and protocol,
    /// then dispatches to the appropriate handler (HTTP or Binary).
    pub async fn handle_stream<S>(self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut writer = write_half;

        // All streams now start with a preamble identifying transport and protocol.
        let preamble = read_preamble(&mut reader).await?;

        let resolved_route = self.resolve_route(preamble)?;
        let routing_plan = self.plan_route(&resolved_route);
        super::dispatch::log_route(&resolved_route, &routing_plan);

        use crate::routing::{DeliveryMode, RouteExecution};

        // Handle Passthrough delivery first, regardless of transport.
        // This is used for raw TCP proxying and wRPC passthrough.
        if routing_plan.delivery_mode == DeliveryMode::PassThrough {
            match &routing_plan.execution {
                RouteExecution::TcpPassthrough { host, port } => {
                    debug!("Proxying raw TCP stream to {}:{}", host, port);
                    let mut target = tokio::net::TcpStream::connect(format!("{}:{}", host, port))
                        .await
                        .map_err(|e| {
                            anyhow!("Failed to connect to TCP target {host}:{port}: {e}")
                        })?;

                    let mut client = ReaderWriter { reader, writer };
                    tokio::io::copy_bidirectional(&mut client, &mut target).await.map_err(|e| {
                        anyhow!("Error in bidirectional copy for {host}:{port}: {e}")
                    })?;
                    return Ok(());
                }
                RouteExecution::WasmWrpcPassthrough { channel_id } => {
                    // wRPC passthrough is not yet implemented; log and continue.
                    debug!("Passthrough wRPC stream to Wasm channel: {}", channel_id);
                    tracing::warn!(
                        channel_id = %channel_id,
                        "wRPC passthrough not yet implemented; dropping stream"
                    );
                    return Ok(());
                }
                _ => {
                    return Err(anyhow!(
                        "Invalid execution plan for PassThrough delivery: {:?}",
                        routing_plan.execution
                    ));
                }
            }
        }

        match resolved_route.request.transport {
            RouteTransport::Http => {
                // Reunite reader + writer into a single I/O type for hyper.
                let io = TokioIo::new(ReaderWriter { reader, writer });
                return self.handle_http_stream(io, resolved_route.request).await;
            }
            RouteTransport::Binary => match &routing_plan.execution {
                RouteExecution::NativeJsonRpc { .. }
                | RouteExecution::ExecuteWasm { .. }
                | RouteExecution::Adapted { .. } => {
                    self.handle_json_rpc_loop(reader, &mut writer, &resolved_route, &routing_plan)
                        .await?;
                }
                RouteExecution::Unsupported => {
                    tracing::warn!(
                        protocol = %resolved_route.request.protocol,
                        interface = resolved_route.request.interface.as_str(),
                        service_id = resolved_route.request.service_id.as_str(),
                        "unsupported routing combination"
                    );
                }
                _ => {
                    return Err(anyhow!(
                        "Execution plan {:?} not supported for Binary transport",
                        routing_plan.execution
                    ));
                }
            },
            RouteTransport::Raw => {
                // We should have handled PassThrough above. If we're here with Raw transport
                // but not PassThrough, something is misconfigured.
                return Err(anyhow!(
                    "Raw transport requires PassThrough delivery, but plan has {:?}",
                    routing_plan.delivery_mode
                ));
            }
        }

        Ok(())
    }

    /// Dispatches a stream with a pre-parsed preamble.
    pub async fn dispatch<S>(self, stream: S, preamble: RoutePreamble) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let reader = BufReader::new(read_half);
        let mut writer = write_half;

        let resolved_route = self.resolve_route(preamble)?;
        let routing_plan = self.plan_route(&resolved_route);
        super::dispatch::log_route(&resolved_route, &routing_plan);

        use crate::routing::{DeliveryMode, RouteExecution};

        if routing_plan.delivery_mode == DeliveryMode::PassThrough {
            match &routing_plan.execution {
                RouteExecution::TcpPassthrough { host, port } => {
                    debug!("Proxying raw TCP stream to {}:{}", host, port);
                    let mut target = tokio::net::TcpStream::connect(format!("{}:{}", host, port))
                        .await
                        .map_err(|e| {
                            anyhow!("Failed to connect to TCP target {host}:{port}: {e}")
                        })?;

                    let mut client = ReaderWriter { reader, writer };
                    tokio::io::copy_bidirectional(&mut client, &mut target).await.map_err(|e| {
                        anyhow!("Error in bidirectional copy for {host}:{port}: {e}")
                    })?;
                    return Ok(());
                }
                _ => return Err(anyhow!("Unsupported PassThrough execution in dispatch")),
            }
        }

        match resolved_route.request.transport {
            RouteTransport::Http => {
                let io = TokioIo::new(ReaderWriter { reader, writer });
                return self.handle_http_stream(io, resolved_route.request).await;
            }
            RouteTransport::Binary => {
                self.handle_json_rpc_loop(reader, &mut writer, &resolved_route, &routing_plan)
                    .await?;
            }
            RouteTransport::Raw => {
                return Err(anyhow!("Raw transport not supported in dispatch yet"));
            }
        }

        Ok(())
    }
}
