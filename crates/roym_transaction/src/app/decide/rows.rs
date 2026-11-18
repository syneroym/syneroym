//! Row types and keys for the writer's decision ledger.

use serde::{Deserialize, Serialize};
use syneroym_roym_core::{progress::AgreementProgressPayload, transaction::ReceiptHalf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DecisionState {
    Deciding,
    Agreed,
    NotFormed,
}

impl DecisionState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Deciding => "deciding",
            Self::Agreed => "agreed",
            Self::NotFormed => "not-formed",
        }
    }
}

pub(crate) fn decision_id(quote_id: &str) -> String {
    format!("decision:{quote_id}")
}

pub(crate) fn countersign_id(agreement: &str) -> String {
    format!("countersign:{agreement}")
}

pub(crate) fn snapshot_id(agreement: &str, seq: u32) -> String {
    format!("snapshot:{agreement}:{seq}")
}

pub(crate) fn refused_id(half_record_id: &str) -> String {
    format!("refused:{half_record_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RefusedRow {
    pub(crate) kind: String,
    pub(crate) quote_id: String,
    pub(crate) code: String,
    #[serde(alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DecisionRow {
    pub(crate) kind: String,
    pub(crate) state: DecisionState,
    pub(crate) agreement: String,
    pub(crate) quote_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CountersignRow {
    pub(crate) kind: String,
    pub(crate) agreement: String,
    pub(crate) half: ReceiptHalf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
    #[serde(alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SnapshotRow {
    pub(crate) seq: u32,
    pub(crate) agreement: String,
    pub(crate) conversation: String,
    pub(crate) snapshot: AgreementProgressPayload,
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
    pub(crate) stored_at_secs: u64,
}
