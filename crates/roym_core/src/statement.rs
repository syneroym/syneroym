//! Agreement statement export types and pure build logic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    deal::{CommitmentKind, DealTerms, LocalText, ResourceKind},
    eventlog::{Direction, EventLogEntry, LogEvent},
    transaction::DealRole,
};

#[cfg(test)]
mod tests;

pub const STATEMENT_VERSION: u32 = 1;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991; // 2^53 - 1

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatementError {
    #[error("bad-period")]
    BadPeriod,
    #[error("statement-total-overflow")]
    TotalOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatementRow {
    pub agreement: String,
    pub commitment_id: String,
    pub agreed_at_secs: u64,
    pub settled_at_secs: u64,
    pub direction: Direction,
    pub counterparty_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterparty_name: Option<String>,
    pub resource_kind: ResourceKind,
    pub commitment_kind: CommitmentKind,
    pub description: LocalText,
    pub currency: String,
    pub declared_value_minor: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_note: Option<LocalText>,
    pub records: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatementOpenRow {
    pub agreement: String,
    pub commitment_id: String,
    pub agreed_at_secs: u64,
    pub direction: Direction,
    pub counterparty_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterparty_name: Option<String>,
    pub resource_kind: ResourceKind,
    pub commitment_kind: CommitmentKind,
    pub description: LocalText,
    pub currency: String,
    pub declared_value_minor: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_note: Option<LocalText>,
    pub records: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatementTotal {
    pub currency: String,
    pub direction: Direction,
    pub resource_kind: ResourceKind,
    pub sum_minor: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EepRole {
    Issuer,
    Holder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EepPosition {
    pub eep_id: u32,
    pub version: u32,
    pub role: EepRole,
    pub issuer_did: String,
    pub holder_did: String,
    pub currency: String,
    pub face_value_minor: i64,
    pub expires_at_secs: u64,
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum EepSection {
    NotAvailable,
    Unavailable {
        reason: String,
    },
    Available {
        outstanding_at_start: Vec<EepPosition>,
        outstanding_at_end: Vec<EepPosition>,
        events: Vec<serde_json::Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    pub statement_version: u32,
    pub owner_did: String,
    pub from_secs: u64,
    pub to_secs: u64,
    pub generated_at_secs: u64,
    pub rows: Vec<StatementRow>,
    pub open_at_end: Vec<StatementOpenRow>,
    pub totals: Vec<StatementTotal>,
    pub eeps: EepSection,
}

#[derive(Debug, Clone)]
pub struct AgreementSource<'a> {
    pub agreement: &'a str,
    pub counterparty_did: &'a str,
    pub owner_role: DealRole,
    pub terms: &'a DealTerms,
    pub agreed_at_secs: Option<u64>,
}

pub fn build(
    owner_did: &str,
    from_secs: u64,
    to_secs: u64,
    now_secs: u64,
    entries: &[EventLogEntry],
    agreements: &[AgreementSource<'_>],
) -> Result<Statement, StatementError> {
    if from_secs >= to_secs {
        return Err(StatementError::BadPeriod);
    }

    let rows = build_rows(from_secs, to_secs, entries, agreements);
    let open_at_end = build_open_at_end(to_secs, entries, agreements);
    let totals = build_totals(&rows)?;

    Ok(Statement {
        statement_version: STATEMENT_VERSION,
        owner_did: owner_did.to_string(),
        from_secs,
        to_secs,
        generated_at_secs: now_secs,
        rows,
        open_at_end,
        totals,
        eeps: EepSection::NotAvailable,
    })
}

fn build_rows(
    from_secs: u64,
    to_secs: u64,
    entries: &[EventLogEntry],
    agreements: &[AgreementSource<'_>],
) -> Vec<StatementRow> {
    let mut rows = Vec::new();

    for entry in entries {
        if entry.event != LogEvent::CommitmentSettled {
            continue;
        }
        if entry.occurred_at_secs < from_secs || entry.occurred_at_secs >= to_secs {
            continue;
        }

        let agr = agreements.iter().find(|a| a.agreement == entry.agreement);
        let comm_id = entry.commitment_id.as_deref().unwrap_or_default();
        let comm =
            agr.and_then(|a| a.terms.commitments.iter().find(|c| c.commitment_id == comm_id));

        let agreed_at_secs = entries
            .iter()
            .find(|e| e.agreement == entry.agreement && e.event == LogEvent::AgreementFormed)
            .map(|e| e.occurred_at_secs)
            .or_else(|| agr.and_then(|a| a.agreed_at_secs))
            .unwrap_or(0);

        let description = comm.map(|c| c.description.clone()).unwrap_or_else(|| {
            LocalText::plain(syneroym_text_lang::LangTag::parse("en").unwrap(), "")
        });

        let resource_kind = entry
            .resource_kind
            .or_else(|| comm.map(|c| c.resource_kind))
            .unwrap_or(ResourceKind::GoodsOrService);

        let commitment_kind = entry
            .commitment_kind
            .or_else(|| comm.map(|c| c.kind))
            .unwrap_or(CommitmentKind::Fulfilment);

        let declared_value_minor = entry
            .declared_value_minor
            .or_else(|| comm.map(|c| c.declared_value_minor))
            .unwrap_or(0);

        rows.push(StatementRow {
            agreement: entry.agreement.clone(),
            commitment_id: comm_id.to_string(),
            agreed_at_secs,
            settled_at_secs: entry.occurred_at_secs,
            direction: entry.direction.unwrap_or(Direction::Received),
            counterparty_did: entry.counterparty_did.clone(),
            counterparty_name: None,
            resource_kind,
            commitment_kind,
            description,
            currency: entry.currency.clone(),
            declared_value_minor,
            settlement_note: entry.settlement_note.clone(),
            records: entry.records.clone(),
        });
    }

    rows
}

fn build_open_at_end(
    to_secs: u64,
    entries: &[EventLogEntry],
    agreements: &[AgreementSource<'_>],
) -> Vec<StatementOpenRow> {
    let mut open = Vec::new();

    for agr in agreements {
        let formed_time = entries
            .iter()
            .find(|e| e.agreement == agr.agreement && e.event == LogEvent::AgreementFormed)
            .map(|e| e.occurred_at_secs)
            .or(agr.agreed_at_secs);

        let Some(agreed_at_secs) = formed_time else {
            continue;
        };
        if agreed_at_secs >= to_secs {
            continue;
        }

        for comm in &agr.terms.commitments {
            let settled_before_to = entries.iter().any(|e| {
                e.agreement == agr.agreement
                    && e.commitment_id.as_deref() == Some(&comm.commitment_id)
                    && e.event == LogEvent::CommitmentSettled
                    && e.occurred_at_secs < to_secs
            });
            if settled_before_to {
                continue;
            }

            let has_final_entry = entries.iter().any(|e| {
                e.agreement == agr.agreement
                    && e.commitment_id.as_deref() == Some(&comm.commitment_id)
                    && matches!(
                        e.event,
                        LogEvent::CommitmentFailed
                            | LogEvent::CommitmentCancelled
                            | LogEvent::CommitmentEndedUnconfirmed
                    )
            });
            if has_final_entry {
                continue;
            }

            let direction =
                if agr.owner_role == comm.giver { Direction::Given } else { Direction::Received };

            open.push(StatementOpenRow {
                agreement: agr.agreement.to_string(),
                commitment_id: comm.commitment_id.clone(),
                agreed_at_secs,
                direction,
                counterparty_did: agr.counterparty_did.to_string(),
                counterparty_name: None,
                resource_kind: comm.resource_kind,
                commitment_kind: comm.kind,
                description: comm.description.clone(),
                currency: agr.terms.currency.clone(),
                declared_value_minor: comm.declared_value_minor,
                settlement_note: None,
                records: Vec::new(),
            });
        }
    }

    open
}

fn build_totals(rows: &[StatementRow]) -> Result<Vec<StatementTotal>, StatementError> {
    let mut groups: BTreeMap<(String, u8, u8), (Direction, ResourceKind, i64)> = BTreeMap::new();

    for row in rows {
        let key = (row.currency.clone(), row.direction as u8, row.resource_kind as u8);
        let entry = groups.entry(key).or_insert((row.direction, row.resource_kind, 0));
        let next_sum =
            entry.2.checked_add(row.declared_value_minor).ok_or(StatementError::TotalOverflow)?;
        if next_sum > MAX_SAFE_INTEGER {
            return Err(StatementError::TotalOverflow);
        }
        entry.2 = next_sum;
    }

    let mut totals = Vec::with_capacity(groups.len());
    for ((currency, _, _), (direction, resource_kind, sum_minor)) in groups {
        totals.push(StatementTotal { currency, direction, resource_kind, sum_minor });
    }

    Ok(totals)
}
