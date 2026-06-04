//! HTTP router request interception
//!
//! Handles incoming HTTP traffic, performing URL host rewrite parsing and proxy
//! forwarding.

use std::{convert::Infallible, sync::Arc};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Method, Request, Response, StatusCode, body::Incoming, header::CONTENT_TYPE, service};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as AutoBuilder,
};
use syneroym_rpc::{JsonRpcError, JsonRpcErrorResponse};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::error;

use super::RouteHandler;
use crate::{preamble::RoutePreamble, routing::RoutePipeline};

/// A handler for HTTP-based JSON-RPC requests.
///
/// It wraps a `RouteHandler`, a connection-level `RoutePreamble`, and the
/// planned `RoutePipeline`.
pub struct HttpHandler {
    pub route_handler: RouteHandler,
    pub preamble: RoutePreamble,
    pub pipeline: RoutePipeline,
}

impl RouteHandler {
    /// Upgrades a raw stream to an HTTP server and handles incoming requests.
    ///
    /// This uses `hyper` to serve JSON-RPC over HTTP/1.1.
    pub async fn handle_http_stream<I>(
        self,
        io: TokioIo<I>,
        preamble: RoutePreamble,
        pipeline: RoutePipeline,
    ) -> Result<()>
    where
        I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let handler = Arc::new(HttpHandler { route_handler: self, preamble, pipeline });

        AutoBuilder::new(TokioExecutor::new())
            .serve_connection(
                io,
                service::service_fn(move |req| {
                    let h = handler.clone();
                    async move { h.handle_http_request(req).await }
                }),
            )
            .await
            .map_err(|e| anyhow!("HTTP connection error: {e}"))
    }
}

impl HttpHandler {
    /// The entry point for a single HTTP request.
    ///
    /// This is called by `hyper` for every incoming request on the stream.
    pub async fn handle_http_request(
        &self,
        req: Request<Incoming>,
    ) -> std::result::Result<Response<Full<Bytes>>, Infallible> {
        let response = self.try_handle_http_request(req).await.unwrap_or_else(|e| {
            error!("HTTP JSON-RPC handler error: {e}");
            http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        });
        Ok(response)
    }

    async fn try_handle_http_request(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>> {
        if req.method() != Method::POST {
            return Ok(http_error(StatusCode::METHOD_NOT_ALLOWED, "Only POST is supported".into()));
        }

        let content_type =
            req.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
        if !content_type.starts_with("application/json") {
            return Ok(http_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json".into(),
            ));
        }

        use http_body_util::BodyExt;
        let body_bytes =
            req.collect().await.map_err(|e| anyhow!("Failed to read HTTP body: {e}"))?.to_bytes();

        if body_bytes.is_empty() {
            return Ok(http_error(StatusCode::BAD_REQUEST, "Empty request body".into()));
        }

        match self
            .route_handler
            .dispatch_json_rpc_once(&self.pipeline, &self.preamble, &body_bytes)
            .await
        {
            Ok(payload) => {
                let res = Response::builder()
                    .status(StatusCode::OK)
                    .header(hyper::header::CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from(payload)));
                Ok(res.unwrap_or_else(|_| Response::default()))
            }
            Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }
}

/// Formats a JSON-RPC error response within an HTTP response.
pub fn http_error(status: StatusCode, message: String) -> Response<Full<Bytes>> {
    let body = JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        error: JsonRpcError { code: -32603, message, data: None },
        id: None,
    };
    let body_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body_bytes)))
        .unwrap_or_else(|_| Response::default())
}
