//! Roym's own copy of conversation content: the row types, the one
//! ordering rule, and the reserved deletion-request content type.
//!
//! This copy is what export, search, and delete act on. It stores the
//! message body and is bounded by the host's own retention caps -- so the
//! on-disk cost is two copies of every message, and the product says so.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::types::conversation::DeliveryState;

pub const CONVERSATION_SCHEMA_VERSION: u32 = 1;

/// Reserved for the one message a person never reads. A client that does
/// not understand it ignores it, which is the honest failure mode: this is
/// a request, and the other side's copy is theirs.
pub const DELETION_REQUEST_CONTENT_TYPE: &str = "application/vnd.roym.deletion-request+json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRow {
    /// The host's own conversation id.
    pub id: String,
    pub peer_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_person_did: Option<String>,
    pub opened_at_secs: u64,
    /// From the newest message's sender timestamp.
    pub last_activity_ms: i64,
    pub message_count: u64,
}

/// What the host said, never what this service hoped. `Pending` is what a
/// freshly sent message is, from the host's own return value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoredState {
    Pending,
    Delivered,
    Failed,
}

impl From<DeliveryState> for StoredState {
    fn from(s: DeliveryState) -> Self {
        match s {
            DeliveryState::Pending => StoredState::Pending,
            DeliveryState::Delivered => StoredState::Delivered,
            DeliveryState::Failed => StoredState::Failed,
        }
    }
}

impl From<StoredState> for DeliveryState {
    fn from(s: StoredState) -> Self {
        match s {
            StoredState::Pending => DeliveryState::Pending,
            StoredState::Delivered => DeliveryState::Delivered,
            StoredState::Failed => DeliveryState::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    Incoming,
    Outgoing,
}

/// `Utf8` for a text content type, `Base64` otherwise. Two encodings and a
/// discriminator, so a search knows which rows it can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BodyEncoding {
    Utf8,
    Base64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRow {
    /// The host's own message id.
    pub id: String,
    pub conversation: String,
    pub author: String,
    pub direction: Direction,
    pub sender_timestamp_ms: i64,
    pub content_type: String,
    pub body_encoding: BodyEncoding,
    /// Absent once deleted. The row itself is the durable deletion record:
    /// the spec's rule is that deleting removes the local copy and writes
    /// a record, not that the row disappears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub state: StoredState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at_secs: Option<u64>,
    pub stored_at_secs: u64,
}

impl MessageRow {
    /// Drops the body and stamps the deletion. The id, timestamp and
    /// author are kept -- the row is the durable deletion record.
    pub fn tombstone(&mut self, now_secs: u64) {
        self.body = None;
        self.deleted_at_secs = Some(now_secs);
    }

    #[must_use]
    pub fn is_deleted(&self) -> bool {
        self.deleted_at_secs.is_some()
    }
}

/// ADR-0013 section 5's rule, applied to this service's own copy so the
/// order a person sees here is the order the host would give and the order
/// every other participant computes: `(sender-timestamp, author, id)`.
#[must_use]
pub fn sort_key(m: &MessageRow) -> (i64, &str, &str) {
    (m.sender_timestamp_ms, m.author.as_str(), m.id.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeletionRequestError {
    #[error("deletion request body is not valid JSON: {0}")]
    Json(String),
    #[error("deletion request must be a JSON object with a string message_id and nothing else")]
    Shape,
}

/// Parses a `DELETION_REQUEST_CONTENT_TYPE` body. It must be exactly
/// `{ "message_id": "<id>" }` -- no other keys, no other shapes.
pub fn parse_deletion_request(body: &[u8]) -> Result<String, DeletionRequestError> {
    let v: Value =
        serde_json::from_slice(body).map_err(|e| DeletionRequestError::Json(e.to_string()))?;
    let obj = v.as_object().ok_or(DeletionRequestError::Shape)?;
    if obj.len() != 1 {
        return Err(DeletionRequestError::Shape);
    }
    obj.get("message_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or(DeletionRequestError::Shape)
}

/// The body a deletion request carries for `target_message_id`.
#[must_use]
pub fn deletion_request_body(target_message_id: &str) -> Vec<u8> {
    // `Value::to_string` is infallible, unlike `serde_json::to_vec`.
    serde_json::json!({ "message_id": target_message_id }).to_string().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, ts: i64, author: &str) -> MessageRow {
        MessageRow {
            id: id.to_string(),
            conversation: "conv".to_string(),
            author: author.to_string(),
            direction: Direction::Incoming,
            sender_timestamp_ms: ts,
            content_type: "text/plain".to_string(),
            body_encoding: BodyEncoding::Utf8,
            body: Some("hi".to_string()),
            state: StoredState::Delivered,
            last_error: None,
            deleted_at_secs: None,
            stored_at_secs: 100,
        }
    }

    #[test]
    fn sort_key_is_arrival_order_independent() {
        let a = row("m-a", 10, "did:key:zA");
        let b = row("m-b", 20, "did:key:zB");
        let mut forward = [a.clone(), b.clone()];
        let mut backward = [b, a];
        forward.sort_by(|x, y| sort_key(x).cmp(&sort_key(y)));
        backward.sort_by(|x, y| sort_key(x).cmp(&sort_key(y)));
        assert_eq!(
            forward.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            backward.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        );
        assert_eq!(forward[0].id, "m-a");
    }

    #[test]
    fn sort_key_breaks_ties_by_author_then_id() {
        let mut rows =
            [row("m-2", 5, "did:key:zB"), row("m-1", 5, "did:key:zA"), row("m-0", 5, "did:key:zA")];
        rows.sort_by(|x, y| sort_key(x).cmp(&sort_key(y)));
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["m-0", "m-1", "m-2"]);
    }

    #[test]
    fn a_tombstoned_row_keeps_identity_and_loses_its_body() {
        let mut r = row("m-a", 10, "did:key:zA");
        r.tombstone(500);
        assert_eq!(r.id, "m-a");
        assert_eq!(r.sender_timestamp_ms, 10);
        assert_eq!(r.author, "did:key:zA");
        assert_eq!(r.body, None);
        assert_eq!(r.deleted_at_secs, Some(500));
        assert!(r.is_deleted());
    }

    #[test]
    fn stored_state_round_trips_through_delivery_state() {
        for s in [StoredState::Pending, StoredState::Delivered, StoredState::Failed] {
            assert_eq!(StoredState::from(DeliveryState::from(s)), s);
        }
        for d in [DeliveryState::Pending, DeliveryState::Delivered, DeliveryState::Failed] {
            assert_eq!(DeliveryState::from(StoredState::from(d)), d);
        }
    }

    #[test]
    fn deletion_request_parse_is_strict() {
        assert_eq!(parse_deletion_request(&deletion_request_body("msg:abc")).unwrap(), "msg:abc");
        assert!(matches!(parse_deletion_request(b"not json"), Err(DeletionRequestError::Json(_))));
        assert_eq!(
            parse_deletion_request(br#"{"message_id":"x","extra":1}"#).unwrap_err(),
            DeletionRequestError::Shape
        );
        assert_eq!(
            parse_deletion_request(br#"{"message_id":42}"#).unwrap_err(),
            DeletionRequestError::Shape
        );
        assert_eq!(
            parse_deletion_request(br#"["msg:abc"]"#).unwrap_err(),
            DeletionRequestError::Shape
        );
        assert_eq!(
            parse_deletion_request(br#"{"message_id":""}"#).unwrap_err(),
            DeletionRequestError::Shape
        );
    }
}
