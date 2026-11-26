//! Agreement event log types and pure derivation logic.

use serde::{Deserialize, Serialize};

use crate::{
    deal::{CommitmentKind, DealTerms, LocalText, ResourceKind, transition::CommitmentState},
    progress::{AgreementState, CommitmentProgress},
    transaction::DealRole,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogEvent {
    AgreementFormed,
    AgreementClosed,
    AgreementCancelled,
    AgreementNotFormed,
    CommitmentSettled,
    CommitmentFailed,
    CommitmentEndedUnconfirmed,
    CommitmentCancelled,
}

impl LogEvent {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AgreementFormed => "agreement-formed",
            Self::AgreementClosed => "agreement-closed",
            Self::AgreementCancelled => "agreement-cancelled",
            Self::AgreementNotFormed => "agreement-not-formed",
            Self::CommitmentSettled => "commitment-settled",
            Self::CommitmentFailed => "commitment-failed",
            Self::CommitmentEndedUnconfirmed => "commitment-ended-unconfirmed",
            Self::CommitmentCancelled => "commitment-cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    Given,
    Received,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventLogEntry {
    pub id: String,
    pub occurred_at_secs: u64,
    pub recorded_at_secs: u64,
    pub event: LogEvent,
    pub agreement: String,
    pub conversation: String,
    pub counterparty_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<ResourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_kind: Option<CommitmentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_state: Option<CommitmentState>,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_value_minor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_note: Option<LocalText>,
    pub records: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormedFact<'a> {
    pub quote_author_issued_at_secs: Option<u64>,
    pub receipt_halves: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSnapshot<'a> {
    pub seq: u32,
    pub issued_at_secs: u64,
    pub record_id: &'a str,
    pub state: AgreementState,
    pub commitments: &'a [CommitmentProgress],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlingHalf<'a> {
    pub commitment_id: &'a str,
    pub records: &'a [String],
    pub settling_issued_at_secs: Option<u64>,
    pub settlement_note: Option<&'a LocalText>,
}

#[derive(Debug, Clone)]
pub struct DeriveInput<'a> {
    pub owner_role: DealRole,
    pub agreement: &'a str,
    pub conversation: &'a str,
    pub counterparty_did: &'a str,
    pub terms: &'a DealTerms,
    pub state: Option<AgreementState>,
    pub commitments: &'a [CommitmentProgress],
    pub agreed: Option<FormedFact<'a>>,
    pub snapshots: &'a [HeldSnapshot<'a>],
    pub settling: &'a [SettlingHalf<'a>],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub entries: Vec<EventLogEntry>,
    pub complete: bool,
}

#[must_use]
pub fn derive(input: &DeriveInput<'_>) -> Derived {
    let mut entries = Vec::new();
    let mut complete = true;

    if !derive_formed(input, &mut entries) {
        complete = false;
    }

    if !derive_commitments(input, &mut entries) {
        complete = false;
    }

    if !derive_terminal(input, &mut entries) {
        complete = false;
    }

    Derived { entries, complete }
}

fn derive_formed(input: &DeriveInput<'_>, entries: &mut Vec<EventLogEntry>) -> bool {
    let Some(ref formed) = input.agreed else {
        return true;
    };
    let Some(occurred_at_secs) = formed.quote_author_issued_at_secs else {
        return false;
    };
    entries.push(EventLogEntry {
        id: format!("{}:-:{}", input.agreement, LogEvent::AgreementFormed.as_str()),
        occurred_at_secs,
        recorded_at_secs: 0,
        event: LogEvent::AgreementFormed,
        agreement: input.agreement.to_string(),
        conversation: input.conversation.to_string(),
        counterparty_did: input.counterparty_did.to_string(),
        commitment_id: None,
        direction: None,
        resource_kind: None,
        commitment_kind: None,
        final_state: None,
        currency: input.terms.currency.clone(),
        declared_value_minor: None,
        settlement_note: None,
        records: formed.receipt_halves.to_vec(),
    });
    true
}

fn derive_commitments(input: &DeriveInput<'_>, entries: &mut Vec<EventLogEntry>) -> bool {
    let mut all_complete = true;
    for comm in &input.terms.commitments {
        let progress = input.commitments.iter().find(|c| c.commitment_id == comm.commitment_id);
        let Some(progress) = progress else {
            continue;
        };

        let direction =
            if input.owner_role == comm.giver { Direction::Given } else { Direction::Received };

        match progress.state {
            CommitmentState::Fulfilled | CommitmentState::Acknowledged => {
                let settling_info =
                    input.settling.iter().find(|s| s.commitment_id == comm.commitment_id);
                if let Some(settling_info) = settling_info
                    && let Some(occurred_at_secs) = settling_info.settling_issued_at_secs
                {
                    entries.push(EventLogEntry {
                        id: format!(
                            "{}:{}:{}",
                            input.agreement,
                            comm.commitment_id,
                            LogEvent::CommitmentSettled.as_str()
                        ),
                        occurred_at_secs,
                        recorded_at_secs: 0,
                        event: LogEvent::CommitmentSettled,
                        agreement: input.agreement.to_string(),
                        conversation: input.conversation.to_string(),
                        counterparty_did: input.counterparty_did.to_string(),
                        commitment_id: Some(comm.commitment_id.clone()),
                        direction: Some(direction),
                        resource_kind: Some(comm.resource_kind),
                        commitment_kind: Some(comm.kind),
                        final_state: Some(progress.state),
                        currency: input.terms.currency.clone(),
                        declared_value_minor: Some(comm.declared_value_minor),
                        settlement_note: settling_info.settlement_note.cloned(),
                        records: settling_info.records.to_vec(),
                    });
                } else {
                    all_complete = false;
                }
            }
            CommitmentState::Failed
            | CommitmentState::EndedUnconfirmed
            | CommitmentState::Cancelled => {
                let event = match progress.state {
                    CommitmentState::Failed => LogEvent::CommitmentFailed,
                    CommitmentState::EndedUnconfirmed => LogEvent::CommitmentEndedUnconfirmed,
                    CommitmentState::Cancelled => LogEvent::CommitmentCancelled,
                    _ => unreachable!(),
                };

                let earliest_snap = input
                    .snapshots
                    .iter()
                    .filter(|snap| {
                        snap.commitments.iter().any(|c| {
                            c.commitment_id == comm.commitment_id && c.state == progress.state
                        })
                    })
                    .min_by_key(|snap| snap.seq);

                if let Some(snap) = earliest_snap {
                    entries.push(EventLogEntry {
                        id: format!(
                            "{}:{}:{}",
                            input.agreement,
                            comm.commitment_id,
                            event.as_str()
                        ),
                        occurred_at_secs: snap.issued_at_secs,
                        recorded_at_secs: 0,
                        event,
                        agreement: input.agreement.to_string(),
                        conversation: input.conversation.to_string(),
                        counterparty_did: input.counterparty_did.to_string(),
                        commitment_id: Some(comm.commitment_id.clone()),
                        direction: Some(direction),
                        resource_kind: Some(comm.resource_kind),
                        commitment_kind: Some(comm.kind),
                        final_state: Some(progress.state),
                        currency: input.terms.currency.clone(),
                        declared_value_minor: Some(comm.declared_value_minor),
                        settlement_note: None,
                        records: vec![snap.record_id.to_string()],
                    });
                } else {
                    all_complete = false;
                }
            }
            CommitmentState::Promised
            | CommitmentState::PaymentRequested
            | CommitmentState::Claimed => {}
        }
    }
    all_complete
}

fn derive_terminal(input: &DeriveInput<'_>, entries: &mut Vec<EventLogEntry>) -> bool {
    let Some(state) = input.state else {
        return true;
    };
    let event = match state {
        AgreementState::Closed => Some(LogEvent::AgreementClosed),
        AgreementState::Cancelled => Some(LogEvent::AgreementCancelled),
        AgreementState::NotFormed => Some(LogEvent::AgreementNotFormed),
        AgreementState::Agreed => None,
    };

    let Some(event) = event else {
        return true;
    };

    let earliest_snap =
        input.snapshots.iter().filter(|snap| snap.state == state).min_by_key(|snap| snap.seq);

    if let Some(snap) = earliest_snap {
        entries.push(EventLogEntry {
            id: format!("{}:-:{}", input.agreement, event.as_str()),
            occurred_at_secs: snap.issued_at_secs,
            recorded_at_secs: 0,
            event,
            agreement: input.agreement.to_string(),
            conversation: input.conversation.to_string(),
            counterparty_did: input.counterparty_did.to_string(),
            commitment_id: None,
            direction: None,
            resource_kind: None,
            commitment_kind: None,
            final_state: None,
            currency: input.terms.currency.clone(),
            declared_value_minor: None,
            settlement_note: None,
            records: vec![snap.record_id.to_string()],
        });
        true
    } else {
        false
    }
}
