//! Backup section verification and import for payments, fulfilments, ledger,
//! and agreement progress.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{
        SECTION_AGREEMENT_PROGRESS, SECTION_AS_LISTED, SECTION_FULFILMENTS, SECTION_LEDGER,
        SECTION_PAYMENTS, SECTION_STEPS,
    },
    deal::{Commitment, CommitmentKind, ResourceKind, SettlementRole, SignedCommitment},
    envelope::Response,
    fulfilment, listing, payment, progress,
    transaction::{Answers, DealRole, ReceiptHalf},
};

use super::{
    super::{
        AGREEMENT_PROGRESS, AS_LISTED, AgreementRow, FULFILMENTS, FulfilmentsRow, LEDGER, PAYMENTS,
        PaymentsRow, STEPS,
        as_listed_ops::{AnswerRow, AnswerState},
        decide::{CountersignRow, DecisionRow, DecisionState, RefusedRow, SnapshotRow},
        decline_ops::DeclineRow,
        enquiry_ops::as_listed::AsListedRow,
        ledger::{FailureRow, LedgerRow},
        put_row,
        step_ops::StepRow,
    },
    ImportedQuote, QuotesByRecordId, failure,
    steps::{self, ValidatedSteps},
};

pub(super) type HistoryEntry = (String, String, String);
type ValidatedPayments = (Vec<(String, PaymentsRow)>, Vec<HistoryEntry>);
type ValidatedFulfilments = (Vec<(String, FulfilmentsRow)>, Vec<HistoryEntry>);
type ValidatedLedger = (Vec<(String, ValidatedLedgerRow)>, Vec<HistoryEntry>);
type ValidatedProgress = (Vec<(String, SnapshotRow)>, Vec<HistoryEntry>);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum ValidatedLedgerRow {
    Decision(DecisionRow),
    Countersign(CountersignRow),
    Fence(LedgerRow),
    Decline(DeclineRow),
    Answer(AnswerRow),
    Refused(RefusedRow),
    Failure(FailureRow),
}

pub(crate) struct ValidatedVerticalSections {
    pub(crate) payments: Vec<(String, PaymentsRow)>,
    pub(crate) fulfilments: Vec<(String, FulfilmentsRow)>,
    pub(crate) steps: Vec<(String, StepRow)>,
    pub(crate) ledger: Vec<(String, ValidatedLedgerRow)>,
    pub(crate) progress: Vec<(String, SnapshotRow)>,
    pub(crate) as_listed: Vec<(String, AsListedRow)>,
    pub(crate) history_entries: Vec<HistoryEntry>,
}

/// Each stored tap must name an enquiry this node holds, and the offering
/// envelope it kept must still verify. The row is a local record: a bundle
/// that carries one without its enquiry, or with a forged offering, is not
/// imported.
fn validate_as_listed(
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

fn validate_payments(
    payment_rows: Vec<Value>,
    agreements: &HashMap<String, AgreementRow>,
    now: u64,
) -> Result<ValidatedPayments, Response> {
    let mut validated = Vec::new();
    let mut history = Vec::new();
    for r in payment_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: PaymentsRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("payments '{id}': invalid row: {e}")));
            }
        };

        let agr = match agreements.get(&row.agreement) {
            Some(a) => a,
            None => {
                return Err(Response::invalid_params(format!(
                    "payments '{id}' references unknown agreement '{}'",
                    row.agreement
                )));
            }
        };

        if let Some(ref req_half) = row.request {
            let v = payment::verify_payment_request(&req_half.envelope, now);
            if !v.verified {
                return Err(Response::invalid_params(format!(
                    "payments '{id}' request does not verify: {:?}",
                    v.reason
                )));
            }
            let p = match v.payload.as_ref() {
                Some(p) => p,
                None => {
                    return Err(Response::invalid_params(format!(
                        "payments '{id}' request missing payload"
                    )));
                }
            };
            let c = match agr.terms.commitments.iter().find(|c| c.commitment_id == p.commitment_id)
            {
                Some(c) => c,
                None => {
                    return Err(Response::invalid_params(format!(
                        "payments '{id}' request references unknown commitment '{}'",
                        p.commitment_id
                    )));
                }
            };
            if c.kind != CommitmentKind::Currency {
                return Err(Response::invalid_params(format!(
                    "payments '{id}' request references non-currency commitment '{}'",
                    p.commitment_id
                )));
            }
            history.push((
                req_half.record_id.clone(),
                "payment-request".to_string(),
                req_half.envelope.clone(),
            ));
        }

        verify_payment_half_chain(
            &row.counterparty,
            DealRole::Counterparty,
            &agr.terms.commitments,
            id,
            &mut history,
            now,
        )?;
        verify_payment_half_chain(
            &row.quote_author,
            DealRole::QuoteAuthor,
            &agr.terms.commitments,
            id,
            &mut history,
            now,
        )?;

        validated.push((id.to_string(), row));
    }
    Ok((validated, history))
}

fn verify_payment_half_chain(
    halves: &[ReceiptHalf],
    expected_role: DealRole,
    commitments: &[Commitment],
    id: &str,
    history: &mut Vec<(String, String, String)>,
    now: u64,
) -> Result<(), Response> {
    let mut prev_payload = None;
    for half in halves {
        let v = payment::verify_payment_acknowledgement(&half.envelope, now);
        if !v.verified {
            return Err(Response::invalid_params(format!(
                "payments '{id}' {:?} acknowledgement does not verify",
                expected_role
            )));
        }
        let p = match v.payload.as_ref() {
            Some(p) => p,
            None => {
                return Err(Response::invalid_params(format!(
                    "payments '{id}' acknowledgement missing payload"
                )));
            }
        };
        let c = match commitments.iter().find(|c| c.commitment_id == p.commitment_id) {
            Some(c) => c,
            None => {
                return Err(Response::invalid_params(format!(
                    "payments '{id}' acknowledgement references unknown commitment '{}'",
                    p.commitment_id
                )));
            }
        };
        let expected_settlement_role =
            if expected_role == c.giver { SettlementRole::Giver } else { SettlementRole::Receiver };
        if p.role != expected_settlement_role {
            return Err(Response::invalid_params(format!(
                "payments '{id}' role mismatch in acknowledgement list"
            )));
        }
        if let Some(prev) = &prev_payload
            && !payment::is_valid_correction(prev, p)
        {
            return Err(Response::invalid_params(format!(
                "payments '{id}' invalid correction in {:?} chain",
                expected_role
            )));
        }
        prev_payload = Some(p.clone());
        history.push((
            half.record_id.clone(),
            "payment-acknowledgement".to_string(),
            half.envelope.clone(),
        ));
    }
    Ok(())
}

fn verify_fulfilment_half_entry(
    half: &ReceiptHalf,
    expected_deal_role: DealRole,
    agr: &AgreementRow,
    id: &str,
    label: &str,
    history: &mut Vec<HistoryEntry>,
    now: u64,
) -> Result<(), Response> {
    let v = fulfilment::verify_fulfilment_receipt(&half.envelope, now);
    if !v.verified {
        return Err(Response::invalid_params(format!(
            "fulfilments '{id}' {label} receipt does not verify"
        )));
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => {
            return Err(Response::invalid_params(format!(
                "fulfilments '{id}' {label} receipt missing payload"
            )));
        }
    };
    let c = match agr.terms.commitments.iter().find(|c| c.commitment_id == p.commitment_id) {
        Some(c) => c,
        None => {
            return Err(Response::invalid_params(format!(
                "fulfilments '{id}' {label} receipt references unknown commitment '{}'",
                p.commitment_id
            )));
        }
    };
    let expected_role = if expected_deal_role == c.giver {
        SettlementRole::Giver
    } else {
        SettlementRole::Receiver
    };
    if p.role != expected_role {
        return Err(Response::invalid_params(format!(
            "fulfilments '{id}' {label} receipt mismatch"
        )));
    }
    history.push((half.record_id.clone(), "fulfilment-receipt".to_string(), half.envelope.clone()));
    Ok(())
}

fn validate_fulfilments(
    fulfilment_rows: Vec<Value>,
    agreements: &HashMap<String, AgreementRow>,
    now: u64,
) -> Result<ValidatedFulfilments, Response> {
    let mut validated = Vec::new();
    let mut history = Vec::new();
    for r in fulfilment_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: FulfilmentsRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!(
                    "fulfilments '{id}': invalid row: {e}"
                )));
            }
        };

        let agr = match agreements.get(&row.agreement) {
            Some(a) => a,
            None => {
                return Err(Response::invalid_params(format!(
                    "fulfilments '{id}' references unknown agreement '{}'",
                    row.agreement
                )));
            }
        };

        if let Some(ref c_half) = row.counterparty {
            verify_fulfilment_half_entry(
                c_half,
                DealRole::Counterparty,
                agr,
                id,
                "counterparty",
                &mut history,
                now,
            )?;
        }

        if let Some(ref p_half) = row.quote_author {
            verify_fulfilment_half_entry(
                p_half,
                DealRole::QuoteAuthor,
                agr,
                id,
                "quote author",
                &mut history,
                now,
            )?;
        }

        validated.push((id.to_string(), row));
    }
    Ok((validated, history))
}

fn expected_issuer<'a>(
    commitment: &SignedCommitment,
    role: SettlementRole,
    quote: &'a ImportedQuote,
) -> &'a str {
    match (commitment.giver, role) {
        (DealRole::QuoteAuthor, SettlementRole::Giver)
        | (DealRole::Counterparty, SettlementRole::Receiver) => &quote.quote_author_did,
        (DealRole::Counterparty, SettlementRole::Giver)
        | (DealRole::QuoteAuthor, SettlementRole::Receiver) => &quote.payload.counterparty_did,
    }
}

fn validate_fence_entry(
    id: &str,
    payload: Value,
    quotes_by_record_id: &QuotesByRecordId,
    now: u64,
) -> Result<ValidatedLedgerRow, Response> {
    let row: LedgerRow = serde_json::from_value(payload)
        .map_err(|e| Response::invalid_params(format!("ledger '{id}': invalid fence row: {e}")))?;

    let quote = quotes_by_record_id.get(&row.agreement).ok_or_else(|| {
        Response::invalid_params(format!(
            "ledger '{id}': names unheld agreement '{}'",
            row.agreement
        ))
    })?;

    let cid = row.commitment_id.as_deref().unwrap_or_default();
    let commitment =
        quote.payload.terms.commitments.iter().find(|c| c.commitment_id == cid).ok_or_else(
            || Response::invalid_params(format!("ledger '{id}': unknown commitment id '{cid}'")),
        )?;

    if id.starts_with("payreq:") {
        if commitment.resource_kind != ResourceKind::Currency {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': payreq commitment '{cid}' is not currency"
            )));
        }
    } else if id.starts_with("ack:") {
        if commitment.resource_kind != ResourceKind::Currency {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': ack commitment '{cid}' is not currency"
            )));
        }
        if let Some(ref half) = row.half {
            let v = payment::verify_payment_acknowledgement(&half.envelope, now);
            if !v.verified {
                return Err(Response::invalid_params(format!(
                    "ledger '{id}': ack half does not verify: {:?}",
                    v.reason
                )));
            }
            if let Some(ref p) = v.payload {
                let expected = expected_issuer(commitment, p.role, quote);
                if half.issuer != expected {
                    return Err(Response::invalid_params(format!(
                        "ledger '{id}': ack issuer does not match role"
                    )));
                }
            }
        }
    } else if id.starts_with("fulfil:") {
        if commitment.resource_kind != ResourceKind::GoodsOrService
            && commitment.resource_kind != ResourceKind::ServiceInSlot
        {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': fulfil commitment '{cid}' is not goods-or-service"
            )));
        }
        if let Some(ref half) = row.half {
            let v = fulfilment::verify_fulfilment_receipt(&half.envelope, now);
            if !v.verified {
                return Err(Response::invalid_params(format!(
                    "ledger '{id}': fulfil half does not verify: {:?}",
                    v.reason
                )));
            }
            if let Some(ref p) = v.payload {
                let expected = expected_issuer(commitment, p.role, quote);
                if half.issuer != expected {
                    return Err(Response::invalid_params(format!(
                        "ledger '{id}': fulfil issuer does not match role"
                    )));
                }
            }
        }
    }
    Ok(ValidatedLedgerRow::Fence(row))
}

fn validate_decline_entry(
    id: &str,
    payload: Value,
    quotes_by_record_id: &QuotesByRecordId,
    enquiries_by_record_id: &HashMap<String, String>,
    now: u64,
) -> Result<ValidatedLedgerRow, Response> {
    let row: DeclineRow = serde_json::from_value(payload).map_err(|e| {
        Response::invalid_params(format!("ledger '{id}': invalid decline row: {e}"))
    })?;
    let v = syneroym_roym_core::transaction::verify_deal_decline(&row.envelope, now);
    if !v.verified {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': decline does not verify: {:?}",
            v.reason
        )));
    }
    let deal_from_id = id.strip_prefix("decline:").unwrap_or("");
    if row.deal != deal_from_id {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': deal id mismatch with row: '{}'",
            row.deal
        )));
    }
    let p = v.payload.ok_or_else(|| {
        Response::invalid_params(format!("ledger '{id}': decline missing payload"))
    })?;
    if p.deal != row.deal || p.declines != row.declines {
        return Err(Response::invalid_params(format!("ledger '{id}': payload mismatch with row")));
    }
    if v.record_id.as_deref() != Some(&row.record_id) {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': record id mismatch with envelope"
        )));
    }
    let matches_quote =
        quotes_by_record_id.get(&row.declines).is_some_and(|q| q.payload.quote_id == row.deal);
    let matches_enquiry =
        enquiries_by_record_id.get(&row.declines).is_some_and(|enq_id| enq_id == &row.deal);
    if !matches_quote && !matches_enquiry {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': names unheld target '{}'",
            row.declines
        )));
    }
    Ok(ValidatedLedgerRow::Decline(row))
}

fn validate_refused_entry(
    id: &str,
    payload: Value,
    quotes_by_record_id: &QuotesByRecordId,
    agreements_map: &HashMap<String, AgreementRow>,
) -> Result<ValidatedLedgerRow, Response> {
    let row: RefusedRow = serde_json::from_value(payload).map_err(|e| {
        Response::invalid_params(format!("ledger '{id}': invalid refused row: {e}"))
    })?;
    if row.quote_id.is_empty() {
        return Err(Response::invalid_params(format!("ledger '{id}': empty quote_id")));
    }
    if row.code.is_empty() {
        return Err(Response::invalid_params(format!("ledger '{id}': empty refusal code")));
    }
    let half_record_id = id.strip_prefix("refused:").unwrap_or("");
    let quote_exists = quotes_by_record_id.values().any(|q| q.payload.quote_id == row.quote_id);
    if !quote_exists {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': quote_id '{}' not in backup",
            row.quote_id
        )));
    }
    let half_held = agreements_map.values().any(|a| {
        a.quote_id == row.quote_id
            && a.counterparty.as_ref().is_some_and(|cp| cp.record_id == half_record_id)
    });
    if !half_held {
        return Err(Response::invalid_params(format!(
            "ledger '{id}': names unheld counterparty half '{half_record_id}'"
        )));
    }
    Ok(ValidatedLedgerRow::Refused(row))
}

/// One tap's answer row: it must name an enquiry this node holds, and an
/// answered row must name a held quote that answers that enquiry.
fn validate_answer_entry(
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

fn validate_ledger_entry(
    id: &str,
    payload: Value,
    quotes_by_record_id: &QuotesByRecordId,
    agreements_map: &HashMap<String, AgreementRow>,
    enquiries_by_record_id: &HashMap<String, String>,
    now: u64,
) -> Result<Option<ValidatedLedgerRow>, Response> {
    if id.starts_with("decision:") {
        let row: DecisionRow = serde_json::from_value(payload).map_err(|e| {
            Response::invalid_params(format!("ledger '{id}': invalid decision row: {e}"))
        })?;
        if row.agreement.is_empty() {
            return Err(Response::invalid_params(format!("ledger '{id}': empty agreement")));
        }
        if !quotes_by_record_id.contains_key(&row.agreement) {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': names unheld agreement '{}'",
                row.agreement
            )));
        }
        match row.state {
            DecisionState::Deciding | DecisionState::Agreed | DecisionState::NotFormed => {}
        }
        Ok(Some(ValidatedLedgerRow::Decision(row)))
    } else if id.starts_with("countersign:") {
        let row: CountersignRow = serde_json::from_value(payload).map_err(|e| {
            Response::invalid_params(format!("ledger '{id}': invalid countersign row: {e}"))
        })?;
        let v = syneroym_roym_core::transaction::verify_agreement_receipt(&row.half.envelope, now);
        if !v.verified {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': countersign does not verify: {:?}",
                v.reason
            )));
        }
        if !quotes_by_record_id.contains_key(&row.agreement) {
            return Err(Response::invalid_params(format!(
                "ledger '{id}': names unheld quote '{}'",
                row.agreement
            )));
        }
        Ok(Some(ValidatedLedgerRow::Countersign(row)))
    } else if id.starts_with("decline:") {
        validate_decline_entry(id, payload, quotes_by_record_id, enquiries_by_record_id, now)
            .map(Some)
    } else if let Some(enquiry_record_id) = id.strip_prefix("answer:") {
        validate_answer_entry(
            id,
            enquiry_record_id,
            payload,
            quotes_by_record_id,
            enquiries_by_record_id,
        )
        .map(Some)
    } else if id.starts_with("refused:") {
        validate_refused_entry(id, payload, quotes_by_record_id, agreements_map).map(Some)
    } else if id.starts_with("payreq:") || id.starts_with("ack:") || id.starts_with("fulfil:") {
        validate_fence_entry(id, payload, quotes_by_record_id, now).map(Some)
    } else if let Some(rest) = id.strip_prefix("released:") {
        failure::validate_released_entry(id, rest, payload, quotes_by_record_id).map(Some)
    } else if let Some(rest) = id.strip_prefix("confirmed:") {
        failure::validate_confirmed_entry(id, rest, payload, quotes_by_record_id).map(Some)
    } else if let Some(rest) = id.strip_prefix("failure:") {
        failure::validate_failure_entry(id, rest, payload, quotes_by_record_id, now).map(Some)
    } else {
        Err(Response::invalid_params(format!("ledger '{id}': unknown ledger kind")))
    }
}

pub(crate) fn validate_ledger(
    ledger_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    agreements_map: &HashMap<String, AgreementRow>,
    enquiries_by_record_id: &HashMap<String, String>,
    now: u64,
) -> Result<ValidatedLedger, Response> {
    let mut validated_ledger = Vec::new();
    let mut history = Vec::new();

    for r in &ledger_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        if let Some(row) = validate_ledger_entry(
            id,
            payload,
            quotes_by_record_id,
            agreements_map,
            enquiries_by_record_id,
            now,
        )? {
            if let ValidatedLedgerRow::Decline(decline) = &row {
                history.push((
                    decline.record_id.clone(),
                    "deal-decline".to_string(),
                    decline.envelope.clone(),
                ));
            }
            if let ValidatedLedgerRow::Failure(failure) = &row {
                history.push((
                    failure.record_id.clone(),
                    "commitment-failure".to_string(),
                    failure.envelope.clone(),
                ));
            }
            validated_ledger.push((id.to_string(), row));
        }
    }

    Ok((validated_ledger, history))
}

fn validate_progress(
    progress_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    now: u64,
) -> Result<ValidatedProgress, Response> {
    let mut validated = Vec::new();
    let mut history = Vec::new();
    for r in progress_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: SnapshotRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("progress '{id}': invalid row: {e}")));
            }
        };

        let v = progress::verify_agreement_progress(&row.envelope, now);
        if !v.verified {
            return Err(Response::invalid_params(format!(
                "progress '{id}' does not verify: {:?}",
                v.reason
            )));
        }
        if !quotes_by_record_id.contains_key(&row.agreement) {
            return Err(Response::invalid_params(format!(
                "progress '{id}' references unknown quote '{}'",
                row.agreement
            )));
        }
        if v.payload.as_ref().map(|p| p.agreement.as_str()) != Some(&row.agreement) {
            return Err(Response::invalid_params(format!(
                "progress '{id}' agreement does not match envelope payload"
            )));
        }

        history.push((
            row.record_id.clone(),
            "agreement-progress".to_string(),
            row.envelope.clone(),
        ));
        validated.push((id.to_string(), row));
    }
    Ok((validated, history))
}

pub(crate) fn validate_vertical_sections(
    bundle_sections: &std::collections::BTreeMap<String, Vec<Value>>,
    agreements_map: &HashMap<String, AgreementRow>,
    quotes_by_record_id: &QuotesByRecordId,
    enquiries_by_record_id: &HashMap<String, String>,
    disclosures_by_record_id: &HashMap<String, Vec<String>>,
    now: u64,
) -> Result<ValidatedVerticalSections, Response> {
    let pay_rows = bundle_sections.get(SECTION_PAYMENTS).cloned().unwrap_or_default();
    let ful_rows = bundle_sections.get(SECTION_FULFILMENTS).cloned().unwrap_or_default();
    let led_rows = bundle_sections.get(SECTION_LEDGER).cloned().unwrap_or_default();
    let prog_rows = bundle_sections.get(SECTION_AGREEMENT_PROGRESS).cloned().unwrap_or_default();

    let mut history_entries = Vec::new();
    let (payments, h_pay) = validate_payments(pay_rows, agreements_map, now)?;
    history_entries.extend(h_pay);
    let (fulfilments, h_ful) = validate_fulfilments(ful_rows, agreements_map, now)?;
    history_entries.extend(h_ful);
    let step_rows = bundle_sections.get(SECTION_STEPS).cloned().unwrap_or_default();
    let (steps, h_steps): ValidatedSteps =
        steps::validate_steps(step_rows, agreements_map, disclosures_by_record_id, now)?;
    history_entries.extend(h_steps);
    let (ledger, h_led) = validate_ledger(
        led_rows,
        quotes_by_record_id,
        agreements_map,
        enquiries_by_record_id,
        now,
    )?;
    history_entries.extend(h_led);
    let (progress, h_prog) = validate_progress(prog_rows, quotes_by_record_id, now)?;
    history_entries.extend(h_prog);
    let as_listed_rows = bundle_sections.get(SECTION_AS_LISTED).cloned().unwrap_or_default();
    let as_listed = validate_as_listed(as_listed_rows, enquiries_by_record_id, now)?;

    Ok(ValidatedVerticalSections {
        payments,
        fulfilments,
        steps,
        ledger,
        progress,
        as_listed,
        history_entries,
    })
}

pub(crate) async fn write_vertical_sections<H: AppHost>(
    host: &H,
    validated: ValidatedVerticalSections,
    imported_history: &mut HashMap<String, (String, String)>,
) -> Result<(), Response> {
    for (rid, rtype, env) in validated.history_entries {
        imported_history.insert(rid, (rtype, env));
    }
    for (id, row) in validated.payments {
        put_row(host, PAYMENTS, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.fulfilments {
        put_row(host, FULFILMENTS, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.steps {
        put_row(host, STEPS, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.ledger {
        put_row(host, LEDGER, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.progress {
        put_row(host, AGREEMENT_PROGRESS, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.as_listed {
        put_row(host, AS_LISTED, &id, &row).await.map_err(Response::internal_error)?;
    }
    Ok(())
}
