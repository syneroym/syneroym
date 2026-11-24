//! Backup import checks for the "agree as listed" rows: the tap's local
//! record, the owner's answer row, and the counterparty's automatic-acceptance
//! fence.

use std::collections::HashMap;

use serde_json::Value;
use syneroym_roym_core::{envelope::Response, listing, transaction::Answers};

use super::{
    super::{
        as_listed_ops::{AnswerRow, AnswerState, AutoAcceptRow},
        enquiry_ops::as_listed::AsListedRow,
    },
    QuotesByRecordId,
    sections::ValidatedLedgerRow,
};

/// Each stored tap must name an enquiry this node holds, and the offering
/// envelope it kept must still verify. The row is a local record: a bundle
/// that carries one without its enquiry, or with a forged offering, is not
/// imported.
pub(super) fn validate_as_listed(
    rows: Vec<Value>,
    enquiries_by_record_id: &HashMap<String, String>,
    now: u64,
) -> Result<Vec<(String, AsListedRow)>, Response> {
    let mut validated = Vec::new();
    for r in rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: AsListedRow = serde_json::from_value(payload)
            .map_err(|e| Response::invalid_params(format!("as_listed '{id}': invalid row: {e}")))?;
        if id != row.enquiry_record_id {
            return Err(Response::invalid_params(format!(
                "as_listed '{id}': row names enquiry '{}'",
                row.enquiry_record_id
            )));
        }
        if !enquiries_by_record_id.contains_key(&row.enquiry_record_id) {
            return Err(Response::invalid_params(format!(
                "as_listed '{id}': names unheld enquiry '{}'",
                row.enquiry_record_id
            )));
        }
        let verdict = listing::verify_envelope(&row.envelope, now);
        if !verdict.verified {
            return Err(Response::invalid_params(format!(
                "as_listed '{id}': offering does not verify: {:?}",
                verdict.reason
            )));
        }
        if verdict.record_id.as_deref() != Some(row.record_id.as_str()) {
            return Err(Response::invalid_params(format!(
                "as_listed '{id}': offering record id does not match the stored row"
            )));
        }
        validated.push((id, row));
    }
    Ok(validated)
}

/// One tap's automatic acceptance: it must name a held enquiry and a held
/// quote that answers it.
pub(super) fn validate_auto_accept_entry(
    id: &str,
    enquiry_record_id: &str,
    payload: Value,
    quotes_by_record_id: &QuotesByRecordId,
    enquiries_by_record_id: &HashMap<String, String>,
) -> Result<ValidatedLedgerRow, Response> {
    let row: AutoAcceptRow = serde_json::from_value(payload).map_err(|e| {
        Response::invalid_params(format!("ledger '{id}': invalid autoaccept row: {e}"))
    })?;
    if row.enquiry_record_id != enquiry_record_id {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': row names enquiry '{}'",
            row.enquiry_record_id
        )));
    }
    if !enquiries_by_record_id.contains_key(&row.enquiry_record_id) {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': names unheld enquiry '{}'",
            row.enquiry_record_id
        )));
    }
    let Some(quote) = quotes_by_record_id.get(&row.quote_record_id) else {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': names unheld quote '{}'",
            row.quote_record_id
        )));
    };
    let answers_this = matches!(
        &quote.payload.answers,
        Answers::Enquiry { record_id } if record_id == &row.enquiry_record_id
    );
    if !answers_this {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': the quote does not answer this enquiry"
        )));
    }
    Ok(ValidatedLedgerRow::AutoAccept(row))
}

/// One tap's answer row: it must name an enquiry this node holds, and an
/// answered row must name a held quote that answers that enquiry.
pub(super) fn validate_answer_entry(
    id: &str,
    enquiry_record_id: &str,
    payload: Value,
    quotes_by_record_id: &QuotesByRecordId,
    enquiries_by_record_id: &HashMap<String, String>,
) -> Result<ValidatedLedgerRow, Response> {
    let row: AnswerRow = serde_json::from_value(payload)
        .map_err(|e| Response::invalid_params(format!("ledger '{id}': invalid answer row: {e}")))?;
    if row.enquiry_record_id != enquiry_record_id {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': row names enquiry '{}'",
            row.enquiry_record_id
        )));
    }
    if !enquiries_by_record_id.contains_key(&row.enquiry_record_id) {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': names unheld enquiry '{}'",
            row.enquiry_record_id
        )));
    }
    if row.state == AnswerState::Answered {
        let Some(quote_record_id) = &row.quote_record_id else {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': an answered row names no quote"
            )));
        };
        let Some(quote) = quotes_by_record_id.get(quote_record_id) else {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': names unheld quote '{quote_record_id}'"
            )));
        };
        let answers_this = matches!(
            &quote.payload.answers,
            Answers::Enquiry { record_id } if record_id == &row.enquiry_record_id
        );
        if !answers_this || quote.payload.quote_id != row.quote_id {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': the quote does not answer this enquiry"
            )));
        }
    }
    Ok(ValidatedLedgerRow::Answer(row))
}
