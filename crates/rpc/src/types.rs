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
    /// Caller-supplied fence for at-least-once delivery (ADR-0023 §4): the
    /// receiving node remembers it and answers a duplicate from its stored
    /// first outcome instead of running the target twice.
    ///
    /// Not a JSON-RPC 2.0 member. It rides on the body rather than on the
    /// route preamble because a same-node call has no preamble at all, and
    /// one receiver-side guard has to serve both the local and the remote
    /// entry point. `#[serde(default)]` plus the skip keeps both directions
    /// compatible: a frame without a key is byte-identical to one built
    /// before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
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

#[derive(Serialize, Deserialize, Debug, thiserror::Error)]
#[error("JSON-RPC error {code}: {message}")]
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn request(idempotency_key: Option<&str>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "greet".to_string(),
            params: Value::Null,
            id: Some(Value::from(1)),
            idempotency_key: idempotency_key.map(str::to_string),
        }
    }

    #[test]
    fn an_idempotency_key_survives_the_json_rpc_round_trip() {
        let bytes = serde_json::to_vec(&request(Some("msg-7"))).unwrap();
        let parsed: JsonRpcRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.idempotency_key.as_deref(), Some("msg-7"));
    }

    /// Asserted on the produced bytes, not on a re-parse: a receiver built
    /// before this field existed must see a byte-identical frame, which an
    /// emitted `"idempotency_key": null` would not be.
    #[test]
    fn a_request_without_a_key_serializes_exactly_as_it_does_today() {
        let json = serde_json::to_string(&request(None)).unwrap();
        assert!(!json.contains("idempotency_key"), "the absent key must be skipped, got: {json}");
        assert_eq!(json, r#"{"jsonrpc":"2.0","method":"greet","params":null,"id":1}"#);
    }

    #[test]
    fn a_frame_from_a_sender_that_never_sends_a_key_parses() {
        let frame = br#"{"jsonrpc":"2.0","method":"greet","params":null,"id":1}"#;
        let parsed: JsonRpcRequest = serde_json::from_slice(frame).unwrap();
        assert_eq!(parsed.idempotency_key, None);
        assert_eq!(parsed.method, "greet");
    }
}
