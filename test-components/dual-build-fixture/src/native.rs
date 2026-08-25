#![cfg(not(target_arch = "wasm32"))]
//! The fixture as a substrate-dispatchable native service. Generic over the
//! host so it names no shim type -- the embedder supplies one, along with
//! the service id it was registered under.

use std::fmt;

use serde_json::Value;
use syneroym_app_host::{
    AppHost, ConversationSink, MessageSink,
    types::conversation::{DeliveryState, Message},
};
use syneroym_rpc::{CallerContext, NativeInvocation, NativeResponse, RpcError, RpcResult};

/// Exported so the substrate's registration call site and this crate's own
/// tests cannot drift on the interface name.
pub const FIXTURE_INTERFACE: &str = "syneroym-test:dual-build-fixture/test-driver@0.1.0";

pub struct NativeFixture<H: AppHost + 'static> {
    service_id: String,
    host_for: Box<dyn Fn(CallerContext) -> H + Send + Sync>,
}

/// `NativeService`/`MessageSink` require `Debug`, and a boxed closure has
/// none. Hand-written rather than derived.
impl<H: AppHost + 'static> fmt::Debug for NativeFixture<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFixture")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl<H: AppHost + 'static> NativeFixture<H> {
    pub fn new(
        service_id: String,
        host_for: impl Fn(CallerContext) -> H + Send + Sync + 'static,
    ) -> Self {
        Self { service_id, host_for: Box::new(host_for) }
    }
}

/// The same two shapes `json_to_wasm_params` accepts on the WASM side:
/// positional `["<json>"]` or named `{"request": "<json>"}` -- one client
/// frame drives both builds.
fn extract_request_param(params: &Value) -> Option<String> {
    match params {
        Value::Array(items) => items.first()?.as_str().map(str::to_string),
        Value::Object(map) => map.get("request")?.as_str().map(str::to_string),
        _ => None,
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_rpc::NativeService for NativeFixture<H> {
    async fn dispatch(&self, inv: NativeInvocation) -> RpcResult<NativeResponse> {
        if inv.method != "run" {
            return Err(RpcError::MethodNotFound(inv.method));
        }
        let request = extract_request_param(&inv.params)
            .ok_or_else(|| RpcError::InvalidParams("expected one string param".to_string()))?;
        let host = (self.host_for)(inv.caller);
        match crate::app::run(&host, &request).await {
            Ok(payload) => Ok(NativeResponse { payload: Value::String(payload) }),
            Err(e) => Err(RpcError::InternalError(e)), // mirrors the WASM `Err` arm's -32603
        }
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> MessageSink for NativeFixture<H> {
    async fn handle_message(&self, topic: String, payload: Vec<u8>) -> Result<(), String> {
        // Same identity the WASM delivery path uses for a subscribed
        // component's `handle-message` invocation: an elevated caller here
        // would let every delivered message pass the `execute-ddl` admin
        // gate.
        let host = (self.host_for)(CallerContext::service_system(&self.service_id));
        crate::app::on_message(&host, topic, payload).await
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> ConversationSink for NativeFixture<H> {
    async fn on_message(&self, msg: Message) -> Result<(), String> {
        let host = (self.host_for)(CallerContext::service_system(&self.service_id));
        crate::app::on_conversation_message(&host, msg).await
    }

    async fn on_delivery_state(&self, message: String, state: DeliveryState) -> Result<(), String> {
        let host = (self.host_for)(CallerContext::service_system(&self.service_id));
        crate::app::on_conversation_state(&host, message, state).await
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_app_host_native::HttpSink for NativeFixture<H> {
    async fn handle_request(
        &self,
        caller: CallerContext,
        request: syneroym_app_host::types::http::HttpRequest,
    ) -> Result<syneroym_app_host::types::http::HttpResponse, String> {
        let host = (self.host_for)(caller);
        crate::app::handle_http(&host, request).await
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_app_host_native::WebSocketSink for NativeFixture<H> {
    async fn on_open(&self, caller: CallerContext, conn: String) {
        let host = (self.host_for)(caller);
        crate::app::on_ws_open(&host, conn).await;
    }

    async fn on_message(
        &self,
        caller: CallerContext,
        conn: String,
        frame: Vec<u8>,
        kind: syneroym_app_host::types::http::FrameKind,
    ) {
        let host = (self.host_for)(caller);
        crate::app::on_ws_message(&host, conn, frame, kind).await;
    }

    async fn on_close(&self, caller: CallerContext, conn: String) {
        let host = (self.host_for)(caller);
        crate::app::on_ws_close(&host, conn).await;
    }
}
