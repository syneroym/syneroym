use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;

// --- JSON-RPC 2.0 Structures ---

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default = "default_params")]
    pub params: Value,
    pub id: Option<Value>,
}

fn default_params() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Value,
    pub id: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub error: JsonRpcError,
    pub id: Option<Value>,
}

// --- Native Invocation Structures ---

/// Represents a parsed and validated request ready for dispatch to a native service.
#[derive(Debug)]
pub struct NativeInvocation {
    pub interface: String,
    pub method: String,
    pub params: Value,
}

/// Represents a response from a native service.
#[derive(Debug)]
pub struct NativeResponse {
    pub payload: Value,
}

// --- Native Service Trait ---

#[async_trait::async_trait]
pub trait NativeService: Send + Sync + Debug {
    /// Dispatches a native invocation and returns a native response.
    async fn dispatch(&self, invocation: NativeInvocation) -> Result<NativeResponse>;
}

// --- Protocol Converter ---

pub struct JsonRpcConverter;

impl JsonRpcConverter {
    /// Parses a JSON-RPC request string into a `NativeInvocation`.
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

    /// Converts a `NativeResponse` back into a JSON-RPC response string.
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

    /// Creates a JSON-RPC error response payload.
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
