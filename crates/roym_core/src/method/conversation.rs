use serde::{Deserialize, Serialize};

pub use crate::conversation::{
    AdmissionState, ConversationGroupItem, ConversationListItem, ConversationRowKind, MessageView,
    StoredState,
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    pub conversation: String,
    pub body: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendOutput {
    pub message_id: String,
    pub state: StoredState,
    pub sender_timestamp_ms: i64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChangesInput {
    pub conversation: String,
    pub after_seq: Option<u64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangesOutput {
    pub messages: Vec<MessageView>,
    pub last_seq: u64,
    pub kind: ConversationRowKind,
}
