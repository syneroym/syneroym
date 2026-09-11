use serde_json::Value;
use syneroym_core::record_signer::{CallerBinding, SigningPrincipal};
use syneroym_rpc::{
    CallerContext, NativeInvocation, NativeResponse, PERMISSION_DENIED_CODE, RpcError, RpcResult,
};

use super::*;

pub(crate) fn parse_principal(v: &Value) -> RpcResult<SigningPrincipal> {
    if v == "service" {
        return Ok(SigningPrincipal::Service);
    }
    if let Some(obj) = v.as_object()
        && let Some(cert) = obj.get("delegated").and_then(|c| c.as_str())
    {
        return Ok(SigningPrincipal::Delegated { delegation_json: cert.to_string() });
    }
    Err(RpcError::InvalidParams(
        "principal must be \"service\" or {\"delegated\": \"...\"}".to_string(),
    ))
}

pub(crate) fn signing_error(err: syneroym_core::record_signer::SigningError) -> RpcError {
    use syneroym_core::record_signer::SigningError as SE;
    match err {
        SE::NoDelegation(msg) => RpcError::Custom(-32030, msg, None),
        SE::InvalidRecord(msg) => RpcError::InvalidParams(msg),
        SE::PermissionDenied => {
            RpcError::Custom(PERMISSION_DENIED_CODE, "permission denied".to_string(), None)
        }
        SE::Internal(msg) => RpcError::InternalError(msg),
    }
}

impl SynSvcNativeService {
    /// Admits privileged capabilities (`signing/sign-record` and
    /// `vault/reveal`).
    ///
    /// Accepts only synthetic system identities scoped to this service
    /// (`system:<id>`, `system:local-elevated:<id>`, `system:abac:<id>`) or
    /// the recorded owner.
    pub(super) fn admit_privileged_capability(&self, caller: &CallerContext) -> RpcResult<()> {
        let id = &self.service_id;
        let is_own_system = caller.caller_did == format!("system:{id}")
            || caller.caller_did == format!("system:local-elevated:{id}")
            || caller.caller_did == format!("system:abac:{id}");
        if is_own_system {
            return Ok(());
        }
        let owner = self
            .record_signer
            .get()
            .and_then(|s| s.identity(&self.service_id).ok())
            .and_then(|i| i.owner_did);
        match owner {
            Some(o) if o == caller.caller_did => Ok(()),
            _ => Err(RpcError::Custom(
                PERMISSION_DENIED_CODE,
                format!(
                    "'{}' may not reach this capability on service '{}': it is neither the \
                     service itself nor its recorded owner",
                    caller.caller_did, self.service_id
                ),
                None,
            )),
        }
    }

    pub(super) async fn dispatch_signing(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        let Some(signer) = self.record_signer.get().cloned() else {
            return Err(internal("this node has no record signer configured"));
        };
        match invocation.method.as_str() {
            "sign-record" => {
                self.admit_privileged_capability(&invocation.caller)?;
                let params = invocation
                    .params
                    .as_array()
                    .ok_or_else(|| RpcError::InvalidParams("params must be array".to_string()))?;
                if params.len() != 2 {
                    return Err(RpcError::InvalidParams(format!(
                        "sign-record expects [draft, principal], got {}",
                        params.len()
                    )));
                }
                let draft: syneroym_signed_record::RecordDraft =
                    serde_json::from_value(params[0].clone()).map_err(|e| {
                        RpcError::InvalidParams(format!("invalid RecordDraft: {e}"))
                    })?;
                let principal = parse_principal(&params[1])?;
                let caller = match invocation.caller.auth {
                    syneroym_rpc::AuthLevel::Delegated | syneroym_rpc::AuthLevel::Ucan => {
                        CallerBinding::Verified(&invocation.caller.caller_did)
                    }
                    _ => CallerBinding::Internal,
                };
                let signed_json = signer
                    .sign_record(&self.service_id, draft, &principal, caller)
                    .map_err(signing_error)?;
                to_payload(&signed_json)
            }
            "identity" => {
                let target_service_id =
                    serde_json::from_value::<(String,)>(invocation.params.clone())
                        .map(|(s,)| s)
                        .or_else(|_| serde_json::from_value::<String>(invocation.params.clone()))
                        .unwrap_or_else(|_| self.service_id.clone());
                let id = signer.identity(&target_service_id).map_err(signing_error)?;
                to_payload(&serde_json::json!({
                    "signing_did": id.signing_did,
                    "pubkey_hex": id.pubkey_hex,
                    "owner_did": id.owner_did,
                }))
            }
            other => Err(RpcError::MethodNotFound(format!("unknown method: {other}"))),
        }
    }
}
