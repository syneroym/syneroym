//! JSON-RPC type conversion helpers
//!
//! Defines standard bidirectional mapping and type conversion adapters
//! across different data representations in the RPC layer.

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::{
    CallerContext, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse,
    NativeInvocation, NativeResponse,
};

#[derive(Debug)]
pub struct JsonRpcConverter;

impl JsonRpcConverter {
    pub fn json_to_native(
        interface: &str,
        caller: CallerContext,
        frame: &[u8],
    ) -> Result<(JsonRpcRequest, NativeInvocation)> {
        let request: JsonRpcRequest =
            serde_json::from_slice(frame).map_err(|e| anyhow!("JSON parse error: {e}"))?;

        let invocation = NativeInvocation {
            interface: interface.to_string(),
            method: request.method.clone(),
            params: request.params.clone(),
            caller,
        };

        Ok((request, invocation))
    }

    pub fn native_to_json(request: &JsonRpcRequest, response: NativeResponse) -> Result<Vec<u8>> {
        let json_response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: response.payload,
            id: request.id.clone(),
        };
        serde_json::to_vec(&json_response).map_err(Into::into)
    }

    pub fn json_error<T: ToString>(id: Option<Value>, code: i32, message: T) -> Result<Vec<u8>> {
        Self::json_error_with_data(id, code, message, None)
    }

    /// Like [`Self::json_error`], keeping the `data` a native service put in
    /// its error. A caller may need it to act, for example a mailbox's
    /// longest time-to-live in a `ttl_too_long` answer.
    pub fn json_error_with_data<T: ToString>(
        id: Option<Value>,
        code: i32,
        message: T,
        data: Option<Value>,
    ) -> Result<Vec<u8>> {
        let error_response = JsonRpcErrorResponse {
            jsonrpc: "2.0".to_string(),
            error: JsonRpcError { code, message: message.to_string(), data },
            id,
        };
        serde_json::to_vec(&error_response).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(Value::Number(1.into())),
            idempotency_key: None,
        }
    }

    #[test]
    fn test_json_to_native_roundtrip() {
        let req = make_request("greet", serde_json::json!(["world"]));
        let frame = serde_json::to_vec(&req).unwrap();
        let (parsed_req, invocation) = JsonRpcConverter::json_to_native(
            "health",
            CallerContext::service_system("health"),
            &frame,
        )
        .unwrap();
        assert_eq!(parsed_req.method, "greet");
        assert_eq!(invocation.interface, "health");
        assert_eq!(invocation.method, "greet");
        assert_eq!(invocation.params, serde_json::json!(["world"]));
    }

    #[test]
    fn test_native_to_json() {
        let req = make_request("ping", serde_json::json!({}));
        let resp = NativeResponse { payload: serde_json::json!({"status": "ok"}) };
        let bytes = JsonRpcConverter::native_to_json(&req, resp).unwrap();
        let parsed: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.jsonrpc, "2.0");
        assert_eq!(parsed.result, serde_json::json!({"status": "ok"}));
        assert_eq!(parsed.id, Some(Value::Number(1.into())));
        // Must not have a trailing newline (framing layer is responsible for framing)
        assert!(!bytes.ends_with(b"\n"));
    }

    #[test]
    fn test_json_error() {
        let bytes =
            JsonRpcConverter::json_error(Some(Value::Number(42.into())), -32601, "Not found")
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"]["code"], -32601);
        assert_eq!(parsed["error"]["message"], "Not found");
        assert_eq!(parsed["id"], 42);
        assert!(!bytes.ends_with(b"\n"));
    }

    #[test]
    fn a_native_error_keeps_its_data_on_the_wire() {
        let data = serde_json::json!({ "max_ttl_secs": 3 });
        let bytes =
            JsonRpcConverter::json_error_with_data(None, -32050, "too long", Some(data)).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"]["data"]["max_ttl_secs"], 3);

        let plain = JsonRpcConverter::json_error(None, -32601, "Not found").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert!(parsed["error"].get("data").is_none_or(serde_json::Value::is_null));
    }

    #[test]
    fn test_json_to_native_invalid_json() {
        let result = JsonRpcConverter::json_to_native(
            "iface",
            CallerContext::service_system("iface"),
            b"not json",
        );
        assert!(result.is_err());
    }
}
