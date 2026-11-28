use serde::{Deserialize, Serialize};
use syneroym_text_lang::LangTag;

use crate::chat::ForwardMode;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<LangTag>,
    /// Absent means "send as today", a plain `text/plain` message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_mode: Option<ForwardMode>,
}

/// The person's default forward mode, and whether the Hub has asked the
/// first-message question. One row per installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardSettings {
    pub default_mode: ForwardMode,
    pub asked: bool,
}

impl Default for ForwardSettings {
    fn default() -> Self {
        Self { default_mode: ForwardMode::NotForwardable, asked: false }
    }
}

/// One method for read and write: with no field set it only reads.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub default_mode: Option<ForwardMode>,
    pub asked: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ForwardInput {
    /// The source message.
    pub message_id: String,
    /// Where the forward goes.
    pub conversation: String,
}

/// A signed record sent as itself, with the type name that says how to
/// check it.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ForwardRecordInput {
    /// Where the record goes.
    pub conversation: String,
    /// `listing`, `profile` or `quote`.
    pub record_type: String,
    /// The signed record, exactly as its issuer signed it.
    pub envelope: String,
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
