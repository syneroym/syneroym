//! The `invoke` request/response vocabulary.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One `invoke` request. Carries no caller field, deliberately -- see
/// `D-C2-4`. Anything a sibling needs to know about who is asking has to
/// come from a mechanism the receiving guest can itself verify, and no
/// such mechanism exists yet for this interface shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// One JSON-RPC error payload inside a response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// One `invoke` response. Exactly one of `result`/`error` is present.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(result: Value) -> Self {
        Self { result: Some(result), error: None }
    }

    pub fn err(code: i64, message: impl Into<String>) -> Self {
        Self { result: None, error: Some(RpcError { code, message: message.into(), data: None }) }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::err(-32601, format!("Method '{method}' not found"))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::err(-32602, message)
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::err(-32603, message)
    }
}
