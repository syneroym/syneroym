#![cfg(not(target_arch = "wasm32"))]
//! Native service wiring for Web entrypoint service.

use std::fmt;

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::dual_build::{extract_request_param, handle_invoke};
use syneroym_rpc::{
    CallerContext, NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult,
};

type HostFn<H> = Box<dyn Fn(CallerContext) -> H + Send + Sync>;

pub struct NativeWeb<H: AppHost + 'static> {
    service_id: String,
    /// Builds a host for the RPC dispatch path -- a local call, so the
    /// guest sees `invocation.caller()` as `internal`.
    host_for: HostFn<H>,
    /// Builds a host for the inbound HTTP / websocket path. A guest HTTP
    /// request is always router ingress, never a local dispatch, so the
    /// guest must never see `internal` for one -- the WASM engine makes
    /// the identical choice unconditionally (`from_wire` on every guest
    /// HTTP request).
    http_host_for: HostFn<H>,
}

impl<H: AppHost + 'static> fmt::Debug for NativeWeb<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWeb").field("service_id", &self.service_id).finish_non_exhaustive()
    }
}

impl<H: AppHost + 'static> NativeWeb<H> {
    pub fn new(
        service_id: String,
        host_for: impl Fn(CallerContext) -> H + Send + Sync + 'static,
        http_host_for: impl Fn(CallerContext) -> H + Send + Sync + 'static,
    ) -> Self {
        Self { service_id, host_for: Box::new(host_for), http_host_for: Box::new(http_host_for) }
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> NativeService for NativeWeb<H> {
    async fn dispatch(&self, inv: NativeInvocation) -> RpcResult<NativeResponse> {
        let host = (self.host_for)(inv.caller);
        match inv.method.as_str() {
            "invoke" => {
                let req_str = extract_request_param(&inv.params).ok_or_else(|| {
                    RpcError::InvalidParams("expected one string param".to_string())
                })?;
                match handle_invoke(&host, &req_str, crate::app::invoke).await {
                    Ok(payload) => Ok(NativeResponse { payload: Value::String(payload) }),
                    Err(e) => Err(RpcError::InternalError(e)),
                }
            }
            "status" => match crate::app::status(&host).await {
                Ok(payload) => Ok(NativeResponse { payload: Value::String(payload) }),
                Err(e) => Err(RpcError::InternalError(e)),
            },
            other => Err(RpcError::MethodNotFound(other.to_string())),
        }
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_app_host_native::HttpSink for NativeWeb<H> {
    async fn handle_request(
        &self,
        caller: CallerContext,
        request: syneroym_app_host::types::http::HttpRequest,
    ) -> Result<syneroym_app_host::types::http::HttpResponse, String> {
        let host = (self.http_host_for)(caller);
        crate::app::handle_http(&host, request).await
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_app_host_native::WebSocketSink for NativeWeb<H> {
    async fn on_open(&self, caller: CallerContext, conn: String) {
        let host = (self.http_host_for)(caller);
        crate::app::on_ws_open(&host, conn).await;
    }

    async fn on_message(
        &self,
        caller: CallerContext,
        conn: String,
        frame: Vec<u8>,
        kind: syneroym_app_host::types::http::FrameKind,
    ) {
        let host = (self.http_host_for)(caller);
        crate::app::on_ws_message(&host, conn, frame, kind).await;
    }

    async fn on_close(&self, caller: CallerContext, conn: String) {
        let host = (self.http_host_for)(caller);
        crate::app::on_ws_close(&host, conn).await;
    }
}
