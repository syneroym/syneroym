use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::{
    JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse, NativeInvocation,
    NativeResponse,
};

pub struct JsonRpcConverter;

impl JsonRpcConverter {
    pub fn json_to_native(
        interface: &str,
        frame: &[u8],
    ) -> Result<(JsonRpcRequest, NativeInvocation)> {
        let request: JsonRpcRequest =
            serde_json::from_slice(frame).map_err(|e| anyhow!("JSON parse error: {}", e))?;

        let invocation = NativeInvocation {
            interface: interface.to_string(),
            method: request.method.clone(),
            params: request.params.clone(),
        };

        Ok((request, invocation))
    }

    pub fn native_to_json(request: &JsonRpcRequest, response: NativeResponse) -> Result<Vec<u8>> {
        let json_response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: response.payload,
            id: request.id.clone(),
        };
        let mut payload = serde_json::to_vec(&json_response)?;
        payload.push(b'\n');
        Ok(payload)
    }

    pub fn json_error<T: ToString>(id: Option<Value>, code: i32, message: T) -> Result<Vec<u8>> {
        let error_response = JsonRpcErrorResponse {
            jsonrpc: "2.0".to_string(),
            error: JsonRpcError { code, message: message.to_string(), data: None },
            id,
        };
        let mut payload = serde_json::to_vec(&error_response)?;
        payload.push(b'\n');
        Ok(payload)
    }
}
