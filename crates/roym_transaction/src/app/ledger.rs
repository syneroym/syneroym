//! Ledger row shapes and identifiers for deals.

use serde::{Deserialize, Serialize};
use syneroym_roym_core::transaction::ReceiptHalf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LedgerKind {
    Decision,
    Countersign,
    Hold,
    Seat,
    Step,
    Fence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LedgerRow {
    pub(crate) kind: LedgerKind,
    pub(crate) agreement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) commitment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seat: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) step: Option<StepRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) half: Option<ReceiptHalf>,
    #[serde(default, alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StepRow {
    pub(crate) seq: u32,
    pub(crate) event: String,
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
}
