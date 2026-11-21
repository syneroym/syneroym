//! Backup checks for the ledger rows of reservation upkeep: the `confirmed:`
//! and `failure:` rows, and the per-commitment form of the `released:` row.

use serde_json::Value;
use syneroym_roym_core::{
    commitment_failure::verify_commitment_failure, envelope::Response, transaction::DealRole,
};

use super::{
    super::ledger::{FailureRow, LedgerKind, LedgerRow, confirmed_id, failure_id},
    ImportedQuote, QuotesByRecordId,
    sections::ValidatedLedgerRow,
};

fn invalid(id: &str, what: impl std::fmt::Display) -> Response {
    Response::invalid_params(format!("ledger '{id}': {what}"))
}

/// Splits the rest of an id of the form `<agreement>:<commitment id>`.
fn split_ids<'a>(id: &str, rest: &'a str) -> Result<(&'a str, &'a str), Response> {
    rest.split_once(':').ok_or_else(|| invalid(id, "id has no commitment id"))
}

/// The held quote that `agreement` names, and checks that it has the
/// commitment.
fn held_commitment<'a>(
    id: &str,
    quotes: &'a QuotesByRecordId,
    agreement: &str,
    commitment_id: &str,
) -> Result<(&'a ImportedQuote, DealRole), Response> {
    let quote = quotes
        .get(agreement)
        .ok_or_else(|| invalid(id, format!("names unheld agreement '{agreement}'")))?;
    let commitment = quote
        .payload
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == commitment_id)
        .ok_or_else(|| invalid(id, format!("unknown commitment id '{commitment_id}'")))?;
    Ok((quote, commitment.giver))
}

/// A `released:<agreement>` row, or the per-commitment
/// `released:<agreement>:<commitment id>` row.
pub(super) fn validate_released_entry(
    id: &str,
    rest: &str,
    payload: Value,
    quotes: &QuotesByRecordId,
) -> Result<ValidatedLedgerRow, Response> {
    let row: LedgerRow = serde_json::from_value(payload)
        .map_err(|e| invalid(id, format!("invalid released row: {e}")))?;
    let (agreement, commitment_id) = match rest.split_once(':') {
        Some((agreement, cid)) => (agreement, Some(cid)),
        None => (rest, None),
    };
    if row.kind != LedgerKind::Released
        || row.agreement != agreement
        || row.commitment_id.as_deref() != commitment_id
    {
        return Err(invalid(id, "row does not match its id"));
    }
    match commitment_id {
        Some(cid) => held_commitment(id, quotes, agreement, cid).map(|_| ())?,
        None if quotes.contains_key(agreement) => {}
        None => return Err(invalid(id, format!("names unheld agreement '{agreement}'"))),
    }
    Ok(ValidatedLedgerRow::Fence(row))
}

pub(super) fn validate_confirmed_entry(
    id: &str,
    rest: &str,
    payload: Value,
    quotes: &QuotesByRecordId,
) -> Result<ValidatedLedgerRow, Response> {
    let row: LedgerRow = serde_json::from_value(payload)
        .map_err(|e| invalid(id, format!("invalid confirmed row: {e}")))?;
    let (agreement, cid) = split_ids(id, rest)?;
    if row.kind != LedgerKind::Confirmed
        || row.agreement != agreement
        || row.commitment_id.as_deref() != Some(cid)
        || id != confirmed_id(agreement, cid)
    {
        return Err(invalid(id, "row does not match its id"));
    }
    held_commitment(id, quotes, agreement, cid)?;
    Ok(ValidatedLedgerRow::Fence(row))
}

/// A `failure:` row. The record verifies, it is the one the row says, and its
/// issuer is the giver of that commitment of a quote in the same bundle.
pub(super) fn validate_failure_entry(
    id: &str,
    rest: &str,
    payload: Value,
    quotes: &QuotesByRecordId,
    now: u64,
) -> Result<ValidatedLedgerRow, Response> {
    let row: FailureRow = serde_json::from_value(payload)
        .map_err(|e| invalid(id, format!("invalid failure row: {e}")))?;
    let (agreement, cid) = split_ids(id, rest)?;
    if row.kind != LedgerKind::Failure
        || row.agreement != agreement
        || row.commitment_id != cid
        || id != failure_id(agreement, cid)
    {
        return Err(invalid(id, "row does not match its id"));
    }
    let (quote, giver) = held_commitment(id, quotes, agreement, cid)?;
    let verdict = verify_commitment_failure(&row.envelope, now);
    let Some(payload) = verdict.payload.as_ref().filter(|_| verdict.verified) else {
        return Err(invalid(id, format!("failure does not verify: {:?}", verdict.reason)));
    };
    if payload.agreement != row.agreement
        || payload.commitment_id != row.commitment_id
        || payload.conversation != row.conversation
        || payload.conversation != quote.payload.conversation
    {
        return Err(invalid(id, "payload mismatch with row"));
    }
    if verdict.record_id.as_deref() != Some(row.record_id.as_str()) {
        return Err(invalid(id, "record id mismatch with envelope"));
    }
    let expected = match giver {
        DealRole::QuoteAuthor => &quote.quote_author_did,
        DealRole::Counterparty => &quote.payload.counterparty_did,
    };
    if verdict.issuer.as_deref() != Some(expected.as_str()) {
        return Err(invalid(id, "issuer is not the giver of the commitment"));
    }
    Ok(ValidatedLedgerRow::Failure(row))
}
