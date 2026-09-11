use serde_json::Value;
use syneroym_mqtt_broker::namespace_topic_for_publish;
use syneroym_rpc::{NativeInvocation, NativeResponse, RpcError, RpcResult};
use syneroym_wit_interfaces::host::syneroym::{
    app_config::app_config::ConfigError, vault::vault::VaultError,
};

use super::*;

impl SynSvcNativeService {
    pub(super) async fn dispatch_vault(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        match invocation.method.as_str() {
            "reveal" => {
                self.admit_privileged_capability(&invocation.caller)?;
                #[derive(serde::Deserialize)]
                struct Req {
                    key: String,
                }
                let req: Req = parse_params(&invocation)?;
                let store = self
                    .storage_provider
                    .open_service_db(&self.service_id, &self.key_store)
                    .await
                    .map_err(internal)?;
                match store.reveal_secret(&req.key).await.map_err(internal)? {
                    Some(bytes) => to_payload(&bytes),
                    None => Err(internal(VaultError::NotFound.to_string())),
                }
            }
            other => Err(RpcError::MethodNotFound(format!("vault/{other}"))),
        }
    }

    // -- app-config ---------------------------------------------------------

    pub(super) async fn dispatch_app_config(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        // Generation is resolved fresh per call, the native-dispatch
        // equivalent of "pinned at invocation start" (ADR-0008) -- each RPC
        // call *is* its own invocation here, there's no longer-lived Store
        // to pin a generation on ahead of time the way a WASM guest's does.
        let generation = self
            .storage_provider
            .get_latest_config_generation(&self.service_id)
            .await
            .map_err(internal)?;

        match invocation.method.as_str() {
            "get" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    key: String,
                }
                let req: Req = parse_params(&invocation)?;
                let Some((_, blob)) = generation else {
                    return to_payload(&Option::<String>::None);
                };
                let json: Value = serde_json::from_str(&blob)
                    .map_err(|e| internal(ConfigError::Internal(e.to_string()).to_string()))?;
                let val = json.get(&req.key).and_then(|v| v.as_str()).map(str::to_string);
                to_payload(&val)
            }
            "get-section" | "get_section" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    prefix: String,
                }
                let req: Req = parse_params(&invocation)?;
                let Some((_, blob)) = generation else {
                    return to_payload(&Vec::<(String, String)>::new());
                };
                let json: Value = serde_json::from_str(&blob)
                    .map_err(|e| internal(ConfigError::Internal(e.to_string()).to_string()))?;
                let mut results = Vec::new();
                if let Value::Object(map) = json {
                    for (k, v) in map {
                        if (k == req.prefix || k.starts_with(&format!("{}.", req.prefix)))
                            && let Some(s) = v.as_str()
                        {
                            results.push((k, s.to_string()));
                        }
                    }
                }
                to_payload(&results)
            }
            other => Err(RpcError::MethodNotFound(format!("app-config/{other}"))),
        }
    }

    // -- blob-store -----------------------------------------------------

    pub(super) async fn dispatch_messaging(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        match invocation.method.as_str() {
            "publish" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    topic: String,
                    payload: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
                let namespaced = namespace_topic_for_publish(&self.service_id, &req.topic);
                self.messaging_broker.publish(namespaced, req.payload).await.map_err(internal)?;
                to_payload(&())
            }
            other => Err(RpcError::MethodNotFound(format!("messaging/{other}"))),
        }
    }
}
