//! RPC core data types
//!
//! Definitions for messages, headers, statuses, and envelope types
//! used in RPC message serialization.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default = "default_params")]
    pub params: Value,
    pub id: Option<Value>,
}

fn default_params() -> Value {
    Value::Object(Map::new())
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

/// Method name for a broker-pushed `messaging/subscribe` notification
/// frame -- see [`MessagingNotification`].
pub const MESSAGING_MESSAGE_METHOD: &str = "messaging/message";

/// The `params` shape of a `messaging/message` notification frame pushed
/// by the router to a live `messaging/subscribe` stream. Shared by the
/// router (which builds it) and the SDK (which parses it), so a
/// field-name drift between the two fails to compile instead of silently
/// dropping every message client-side.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagingNotification {
    pub topic: String,
    pub payload: Vec<u8>,
}
