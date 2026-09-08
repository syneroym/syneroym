//! The `invoke` request/response vocabulary.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One `invoke` request. Carries no caller field, deliberately: a caller
/// string in the envelope would be an unverifiable self-claim. The
/// caller's origin instead comes from the host, out of band, through
/// `AppInvocation::caller` (`CallerOrigin`), which the guest can trust
/// because the router sets it. That origin still does not name which
/// sibling made an internal call; it only separates internal, verified
/// remote, and anonymous callers.
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

    /// Attaches a structured `data` payload to an error response. A no-op
    /// on a success response.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        if let Some(err) = self.error.as_mut() {
            err.data = Some(data);
        }
        self
    }

    pub fn method_not_found(method: &str) -> Self {
        let safe_method = truncate_method(method);
        Self::err(-32601, format!("Method '{safe_method}' not found"))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::err(-32602, message)
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::err(-32603, message)
    }
}

pub fn truncate_method(method: &str) -> String {
    if method.len() > 128 {
        let mut end = 125;
        while !method.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &method[..end])
    } else {
        method.to_string()
    }
}
