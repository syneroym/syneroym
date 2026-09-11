use std::sync::Weak;

use serde_json::Value;
use syneroym_rpc::{
    ConversationHost, NativeInvocation, NativeResponse, PERMISSION_DENIED_CODE, RpcError, RpcResult,
};

use super::*;

/// Mirrors `empty_service_proxy`: an always-empty `Weak<dyn
/// ConversationHost>` for a node running no conversation service.
pub(crate) fn empty_conversation_host() -> Weak<dyn ConversationHost> {
    #[derive(Debug)]
    struct NeverConstructed;
    #[async_trait::async_trait]
    impl ConversationHost for NeverConstructed {
        async fn open_direct(
            &self,
            _: &str,
            _: &str,
        ) -> Result<String, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn conversations(
            &self,
            _: &str,
        ) -> Result<Vec<syneroym_rpc::ConversationSummary>, syneroym_rpc::ConversationError>
        {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn send(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Vec<u8>,
        ) -> Result<String, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn history(
            &self,
            _: &str,
            _: &str,
            _: u32,
            _: Option<String>,
        ) -> Result<syneroym_rpc::ConversationHistoryPage, syneroym_rpc::ConversationError>
        {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn delivery_status(
            &self,
            _: &str,
            _: &str,
        ) -> Result<syneroym_rpc::ConversationDeliveryState, syneroym_rpc::ConversationError>
        {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn outbox(
            &self,
            _: &str,
        ) -> Result<Vec<syneroym_rpc::ConversationMessage>, syneroym_rpc::ConversationError>
        {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn retry(&self, _: &str, _: &str) -> Result<(), syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn create_group(&self, _: &str) -> Result<String, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn add_member(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn remove_member(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn members(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<String>, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn membership_history(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<syneroym_rpc::ConversationMembershipEvent>, syneroym_rpc::ConversationError>
        {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn sync_now(&self, _: &str, _: &str) -> Result<(), syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn group_push(
            &self,
            _: &str,
            _: &str,
            _: Vec<u8>,
        ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn group_sync(
            &self,
            _: &str,
            _: &str,
            _: Vec<u8>,
        ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn prekey_bundle(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn peer_deliver(
            &self,
            _: &str,
            _: &str,
            _: Vec<u8>,
        ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
    }
    Weak::<NeverConstructed>::new()
}

/// Maps `ConversationError` into the `RpcError` shape every other native
/// dispatch arm uses.
fn conversation_error(e: syneroym_rpc::ConversationError) -> RpcError {
    use syneroym_rpc::ConversationError as CE;
    match e {
        CE::PermissionDenied => {
            RpcError::Custom(PERMISSION_DENIED_CODE, "permission denied".to_string(), None)
        }
        CE::NotFound => RpcError::Custom(-32001, "not found".to_string(), None),
        CE::InvalidArgument(msg) => RpcError::InvalidParams(msg),
        CE::Unreachable(msg) => RpcError::Custom(-32002, msg, None),
        CE::QuotaExceeded => RpcError::Custom(-32003, "quota exceeded".to_string(), None),
        CE::Internal(msg) => internal(msg),
    }
}

impl SynSvcNativeService {
    pub(super) async fn dispatch_conversation(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        let Some(conversation) = self.current_conversation().upgrade() else {
            return Err(internal("this node runs no conversation service"));
        };
        match invocation.method.as_str() {
            "prekey-bundle" => {
                let bundle = conversation
                    .prekey_bundle(&self.service_id, &invocation.caller.caller_did)
                    .await
                    .map_err(conversation_error)?;
                to_payload(&serde_json::from_slice::<Value>(&bundle).map_err(internal)?)
            }
            "deliver" => {
                let envelope_bytes = serde_json::to_vec(&invocation.params).map_err(internal)?;
                let ack = conversation
                    .peer_deliver(&self.service_id, &invocation.caller.caller_did, envelope_bytes)
                    .await
                    .map_err(conversation_error)?;
                to_payload(&serde_json::from_slice::<Value>(&ack).map_err(internal)?)
            }
            "group-push" => {
                let bytes = serde_json::to_vec(&invocation.params).map_err(internal)?;
                let ack = conversation
                    .group_push(&self.service_id, &invocation.caller.caller_did, bytes)
                    .await
                    .map_err(conversation_error)?;
                to_payload(&serde_json::from_slice::<Value>(&ack).map_err(internal)?)
            }
            "group-sync" => {
                let bytes = serde_json::to_vec(&invocation.params).map_err(internal)?;
                let resp = conversation
                    .group_sync(&self.service_id, &invocation.caller.caller_did, bytes)
                    .await
                    .map_err(conversation_error)?;
                to_payload(&serde_json::from_slice::<Value>(&resp).map_err(internal)?)
            }
            other => Err(RpcError::MethodNotFound(format!("conversation/{other}"))),
        }
    }
}
