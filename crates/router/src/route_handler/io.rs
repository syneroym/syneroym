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

        match preamble.transport {
            RouteTransport::Http => {
                // Reunite reader + writer into a single I/O type for hyper.
                let io = TokioIo::new(ReaderWriter { reader, writer });
                return self.handle_http_stream(io, preamble).await;
            }
            RouteTransport::Binary => {
                let resolved_route = self.resolve_route(preamble)?;
                let routing_plan = self.plan_route(&resolved_route);
                super::dispatch::log_route(&resolved_route, &routing_plan);

                use crate::routing::RouteExecution;
                match &routing_plan.execution {
                    RouteExecution::NativeJsonRpc { .. }
                    | RouteExecution::ExecuteWasm { .. }
                    | RouteExecution::Adapted { .. } => {
                        self.handle_json_rpc_loop(
                            reader,
                            &mut writer,
                            &resolved_route,
                            &routing_plan,
                        )
                        .await?;
                    }
                    RouteExecution::WasmWrpcPassthrough { channel_id } => {
                        // wRPC passthrough is not yet implemented; log and continue.
                        debug!("Passthrough wRPC stream to Wasm channel: {}", channel_id);
                        tracing::warn!(
                            channel_id = %channel_id,
                            "wRPC passthrough not yet implemented; dropping stream"
                        );
                    }
                    RouteExecution::Unsupported => {
                        tracing::warn!(
                            protocol = %resolved_route.request.protocol,
                            interface = resolved_route.request.interface.as_str(),
                            service_id = resolved_route.request.service_id.as_str(),
                            "unsupported routing combination"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}
