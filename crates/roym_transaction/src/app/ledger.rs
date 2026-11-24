//! Ledger row shapes and identifiers for deals.

use serde::{Deserialize, Serialize};
use syneroym_roym_core::transaction::ReceiptHalf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LedgerKind {
    Decision,
    Countersign,
    Fence,
    /// The counterparty released its reservations after the quote expired.
    Released,
    /// The counterparty's own reservation of one commitment was confirmed.
    Confirmed,
    /// A signed `commitment-failure` record, with the state of its send.
    Failure,
    /// The owner's answer to one as-listed enquiry: the fence that keeps it to
    /// at most one quote.
    Answer,
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
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) half: Option<ReceiptHalf>,
    #[serde(default, alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

/// The ledger row that says this node confirmed its own reservation of one
/// commitment. It only saves a hook call on each later sync.
pub(crate) fn confirmed_id(agreement: &str, commitment_id: &str) -> String {
    format!("confirmed:{agreement}:{commitment_id}")
}

/// The ledger row of the `commitment-failure` record of one commitment. The
/// giver makes it once and sends it; the writer stores the one it files.
pub(crate) fn failure_id(agreement: &str, commitment_id: &str) -> String {
    format!("failure:{agreement}:{commitment_id}")
}

/// A signed `commitment-failure` record. The giver stores it before it sends
/// it, so a repeat sends the same record again. `message_id` is empty until
/// the send has worked, and the writer never sets it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FailureRow {
    pub(crate) kind: LedgerKind,
    pub(crate) agreement: String,
    pub(crate) commitment_id: String,
    pub(crate) conversation: String,
    pub(crate) record_id: String,
    pub(crate) envelope: String,
    pub(crate) created_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
}
