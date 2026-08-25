//! The native build's inbound HTTP entry point. Mirrors
//! `syneroym:http/incoming-handler` and `syneroym:http/websocket-handler`.
//!
//! An app implements [`HttpSink`] and optionally [`WebSocketSink`]. The
//! substrate's router reaches both through [`NativeHttpAdapter`], which
//! implements [`syneroym_rpc::NativeHttpService`] by delegating to this
//! factory's registered sinks with a fresh `NativeAppHost` per call.

use std::{
    fmt::Debug,
    sync::{Arc, Weak},
};

use syneroym_app_host::types::http::{FrameKind, HttpRequest, HttpResponse};
use syneroym_rpc::{CallerContext, NativeHttpService};

use crate::factory::NativeHostFactory;

/// The host -> app direction for HTTP requests.
#[async_trait::async_trait]
pub trait HttpSink: Send + Sync + Debug {
    async fn handle_request(
        &self,
        caller: CallerContext,
        request: HttpRequest,
    ) -> Result<HttpResponse, String>;
}

/// The host -> app direction for WebSocket events.
#[async_trait::async_trait]
pub trait WebSocketSink: Send + Sync + Debug {
    async fn on_open(&self, caller: CallerContext, conn: String);
    async fn on_message(
        &self,
        caller: CallerContext,
        conn: String,
        frame: Vec<u8>,
        kind: FrameKind,
    );
    async fn on_close(&self, caller: CallerContext, conn: String);
}

/// Adapts the app's registered [`HttpSink`] and [`WebSocketSink`] onto the
/// substrate router's [`NativeHttpService`] dispatch trait.
#[derive(Debug)]
pub struct NativeHttpAdapter {
    factory: Arc<NativeHostFactory>,
    http_sink: Weak<dyn HttpSink>,
    websocket_sink: Weak<dyn WebSocketSink>,
}

impl NativeHttpAdapter {
    #[must_use]
    pub fn new(
        factory: Arc<NativeHostFactory>,
        http_sink: Weak<dyn HttpSink>,
        websocket_sink: Weak<dyn WebSocketSink>,
    ) -> Self {
        Self { factory, http_sink, websocket_sink }
    }

    fn caller_for(&self, caller: Option<CallerContext>) -> CallerContext {
        caller.unwrap_or_else(|| CallerContext::service_system(self.factory.service_id()))
    }
}

#[async_trait::async_trait]
impl NativeHttpService for NativeHttpAdapter {
    async fn handle_request(
        &self,
        request: HttpRequest,
        caller: Option<CallerContext>,
    ) -> Result<HttpResponse, String> {
        let Some(sink) = self.http_sink.upgrade() else {
            return Err("HTTP sink has been dropped".to_string());
        };
        let host_caller = self.caller_for(caller);
        sink.handle_request(host_caller, request).await
    }

    async fn on_websocket_open(&self, conn: String, caller: Option<CallerContext>) {
        if let Some(sink) = self.websocket_sink.upgrade() {
            let host_caller = self.caller_for(caller);
            sink.on_open(host_caller, conn).await;
        }
    }

    async fn on_websocket_message(
        &self,
        conn: String,
        frame: Vec<u8>,
        kind: FrameKind,
        caller: Option<CallerContext>,
    ) {
        if let Some(sink) = self.websocket_sink.upgrade() {
            let host_caller = self.caller_for(caller);
            sink.on_message(host_caller, conn, frame, kind).await;
        }
    }

    async fn on_websocket_close(&self, conn: String, caller: Option<CallerContext>) {
        if let Some(sink) = self.websocket_sink.upgrade() {
            let host_caller = self.caller_for(caller);
            sink.on_close(host_caller, conn).await;
        }
    }

    fn service_id(&self) -> Option<&str> {
        Some(self.factory.service_id())
    }
}
