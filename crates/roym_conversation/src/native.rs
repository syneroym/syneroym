#![cfg(not(target_arch = "wasm32"))]
//! Native service wiring for conversation service.

use std::fmt;

use serde_json::Value;
use syneroym_app_host::{
    AppHost, ConversationSink,
    types::conversation::{DeliveryState, Message},
};
use syneroym_roym_core::dual_build::{extract_request_param, handle_invoke};
use syneroym_rpc::{
    CallerContext, NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult,
};

pub struct NativeConversation<H: AppHost + 'static> {
    service_id: String,
    host_for: Box<dyn Fn(CallerContext) -> H + Send + Sync>,
}

impl<H: AppHost + 'static> fmt::Debug for NativeConversation<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConversation")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl<H: AppHost + 'static> NativeConversation<H> {
    pub fn new(
        service_id: String,
        host_for: impl Fn(CallerContext) -> H + Send + Sync + 'static,
    ) -> Self {
        Self { service_id, host_for: Box::new(host_for) }
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> NativeService for NativeConversation<H> {
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
impl<H: AppHost + 'static> ConversationSink for NativeConversation<H> {
    async fn on_message(&self, msg: Message) -> Result<(), String> {
        // The same identity the WASM delivery path uses, so an elevated
        // caller cannot arrive with a delivered message.
        let host = (self.host_for)(CallerContext::service_system(&self.service_id));
        crate::app::on_message(&host, msg).await
    }

    async fn on_delivery_state(&self, message: String, state: DeliveryState) -> Result<(), String> {
        let host = (self.host_for)(CallerContext::service_system(&self.service_id));
        crate::app::on_delivery_state(&host, message, state).await
    }
}
