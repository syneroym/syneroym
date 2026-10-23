//! Handing a call that waited at the service's mailbox to the service.

use serde_json::Value;
use syneroym_async_queue::FirstOutcome;
use syneroym_mailbox_format::MessageContent;
use syneroym_rpc::{CallerContext, JsonRpcRequest, ProxyError};

use super::{
    RouteHandler,
    dispatch::{Fenced, frame_as_outcome},
};
use crate::{
    preamble::RoutePreamble,
    routing::{RoutePipeline, ServiceStage},
};

/// What became of a call handed to a service from its mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeferredOutcome {
    /// The service answered with a result, now or before (the fence
    /// replayed it). The result itself is not kept.
    Succeeded,
    /// The call ran and answered an error, now or before. The fence keeps
    /// this answer, so a direct call with the same key gets it too.
    Failed { code: i32 },
    /// Nothing ran, for a reason that may pass.
    NotRun(String),
    /// This service cannot take a JSON-RPC call (a TCP or container one).
    NotServed,
}

impl RouteHandler {
    /// Hands a call that waited at the service's mailbox to the service,
    /// through the same fence and dispatch a call from the wire meets.
    /// `interface` is the canonical name. `caller` is the original sender.
    ///
    /// `ProxyRouter::invoke_local` is not used here: its WASM arm passes no
    /// caller, so a guest would see itself as the caller instead of the
    /// sender.
    pub async fn dispatch_deferred(
        &self,
        service_id: &str,
        interface: &str,
        content: &MessageContent,
        caller: &CallerContext,
    ) -> DeferredOutcome {
        let Some((endpoint, interface)) = self.inner.registry.lookup(service_id, interface) else {
            return DeferredOutcome::NotRun("the service does not serve this interface".into());
        };
        let preamble = RoutePreamble::binary_json_rpc(service_id, &interface);
        let pipeline = self.plan_pipeline(&preamble, &endpoint);
        if !is_json_rpc_service(&pipeline) {
            return DeferredOutcome::NotServed;
        }
        let Some(guard) = &self.inner.dedup_guard else {
            return DeferredOutcome::NotRun("this node keeps no per-service storage".into());
        };
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: content.method.clone(),
            params: content.params.clone(),
            id: Some(Value::from(1)),
            idempotency_key: Some(content.idempotency_key.clone()),
        };
        let Ok(body) = serde_json::to_vec(&request) else {
            return DeferredOutcome::NotRun("the call does not encode".into());
        };
        let fenced = self
            .dispatch_fenced(
                guard,
                &pipeline,
                &preamble,
                Some(caller),
                &body,
                &content.idempotency_key,
            )
            .await;
        match fenced {
            Fenced::Refused(e) => DeferredOutcome::NotRun(e.to_string()),
            Fenced::Replayed(outcome) => outcome_of(outcome),
            Fenced::Ran(Ok(frame)) => answer_of(&Ok(frame)),
            Fenced::Ran(Err(e)) => DeferredOutcome::NotRun(e.to_string()),
        }
    }
}

fn is_json_rpc_service(pipeline: &RoutePipeline) -> bool {
    matches!(
        pipeline.service,
        ServiceStage::NativeService { .. } | ServiceStage::WasmComponent { .. }
    )
}

/// A stored first answer, as the mailbox receiver sees it.
fn outcome_of(outcome: FirstOutcome) -> DeferredOutcome {
    match outcome {
        FirstOutcome::Success(_) | FirstOutcome::SuccessNotRetained => DeferredOutcome::Succeeded,
        FirstOutcome::CalleeError { code, .. } => DeferredOutcome::Failed { code },
    }
}

/// A fresh response frame. A frame that does not parse was made by the
/// target, so it counts as an error answer.
fn answer_of(frame: &anyhow::Result<Vec<u8>>) -> DeferredOutcome {
    match frame_as_outcome(frame) {
        Ok(_) => DeferredOutcome::Succeeded,
        Err(ProxyError::Callee { code, .. }) => DeferredOutcome::Failed { code },
        Err(_) => DeferredOutcome::Failed { code: -32603 },
    }
}
