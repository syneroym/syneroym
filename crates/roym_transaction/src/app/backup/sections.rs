//! Backup section verification and import for payments, fulfilments, ledger,
//! and progress.

use std::collections::HashMap;

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{SECTION_FULFILMENTS, SECTION_LEDGER, SECTION_PAYMENTS, SECTION_PROGRESS},
    booking,
    envelope::Response,
    fulfilment, payment,
    transaction::{AgreedTerms, ReceiptHalf, Role},
};

use super::{
    super::{
        AgreementRow, BOOKINGS, BookingRow, FULFILMENTS, FulfilmentsRow, LEDGER, PAYMENTS,
        PROGRESS, PaymentsRow, ProgressRow,
        ledger::{LedgerKind, LedgerRow, StepRow},
        put_row,
    },
    QuotesByRecordId,
};

type HistoryEntry = (String, String, String);
type ValidatedPayments = (Vec<(String, PaymentsRow)>, Vec<HistoryEntry>);
type ValidatedFulfilments = (Vec<(String, FulfilmentsRow)>, Vec<HistoryEntry>);
type ValidatedLedgerAndBookings =
    (Vec<(String, LedgerRow)>, Vec<(String, BookingRow)>, Vec<HistoryEntry>);
type ValidatedProgress = (Vec<(String, ProgressRow)>, Vec<HistoryEntry>);

pub(crate) struct ValidatedVerticalSections {
    pub(crate) payments: Vec<(String, PaymentsRow)>,
    pub(crate) fulfilments: Vec<(String, FulfilmentsRow)>,
    pub(crate) ledger: Vec<(String, LedgerRow)>,
    pub(crate) bookings: Vec<(String, BookingRow)>,
    pub(crate) progress: Vec<(String, ProgressRow)>,
    pub(crate) history_entries: Vec<HistoryEntry>,
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
            if p.amount_minor != agr.terms.amount_minor || p.currency != agr.terms.currency {
                return Err(Response::invalid_params(format!(
                    "payments '{id}' request amount/currency mismatch"
                )));
            }
            history.push((
                req_half.record_id.clone(),
                "payment-request".to_string(),
                req_half.envelope.clone(),
            ));
        }

        verify_payment_half_chain(
            &row.consumer,
            Role::Consumer,
            &agr.terms,
            id,
            &mut history,
            now,
        )?;
        verify_payment_half_chain(
            &row.provider,
            Role::Provider,
            &agr.terms,
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
    expected_role: Role,
    terms: &AgreedTerms,
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
        if p.role != expected_role {
            return Err(Response::invalid_params(format!(
                "payments '{id}' role mismatch in acknowledgement list"
            )));
        }
        if !payment::matches_terms(&p.currency, p.amount_minor, p.method.as_deref(), terms) {
            return Err(Response::invalid_params(format!(
                "payments '{id}' acknowledgement does not match terms"
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

        if let Some(ref c) = row.consumer {
            let v = fulfilment::verify_fulfilment_receipt(&c.envelope, now);
            if !v.verified {
                return Err(Response::invalid_params(format!(
                    "fulfilments '{id}' consumer receipt does not verify"
                )));
            }
            let p = match v.payload.as_ref() {
                Some(p) => p,
                None => {
                    return Err(Response::invalid_params(format!(
                        "fulfilments '{id}' consumer receipt missing payload"
                    )));
                }
            };
            if p.terms != agr.terms || p.role != Role::Consumer {
                return Err(Response::invalid_params(format!(
                    "fulfilments '{id}' consumer receipt mismatch"
                )));
            }
            history.push((
                c.record_id.clone(),
                "fulfilment-receipt".to_string(),
                c.envelope.clone(),
            ));
        }

        if let Some(ref p_half) = row.provider {
            let v = fulfilment::verify_fulfilment_receipt(&p_half.envelope, now);
            if !v.verified {
                return Err(Response::invalid_params(format!(
                    "fulfilments '{id}' provider receipt does not verify"
                )));
            }
            let p = match v.payload.as_ref() {
                Some(p) => p,
                None => {
                    return Err(Response::invalid_params(format!(
                        "fulfilments '{id}' provider receipt missing payload"
                    )));
                }
            };
            if p.terms != agr.terms || p.role != Role::Provider {
                return Err(Response::invalid_params(format!(
                    "fulfilments '{id}' provider receipt mismatch"
                )));
            }
            history.push((
                p_half.record_id.clone(),
                "fulfilment-receipt".to_string(),
                p_half.envelope.clone(),
            ));
        }

        validated.push((id.to_string(), row));
    }
    Ok((validated, history))
}

fn validate_ledger_and_bookings(
    ledger_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    now: u64,
) -> Result<ValidatedLedgerAndBookings, Response> {
    let mut validated_ledger = Vec::new();
    let mut history = Vec::new();
    let mut highest_steps: HashMap<String, (StepRow, Option<String>, Option<u32>, u64)> =
        HashMap::new();
    let mut decisions: HashMap<String, (Option<String>, Option<u32>, u64)> = HashMap::new();

    for r in ledger_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: LedgerRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("ledger '{id}': invalid row: {e}")));
            }
        };

        if row.kind == LedgerKind::Decision {
            decisions.insert(
                row.agreement.clone(),
                (row.slot_id.clone(), row.seat, row.created_at_secs),
            );
        } else if row.kind == LedgerKind::Step
            && let Some(ref step) = row.step
        {
            let v = booking::verify_booking_progress(&step.envelope, now);
            if !v.verified {
                return Err(Response::invalid_params(format!(
                    "ledger step '{id}' does not verify: {:?}",
                    v.reason
                )));
            }
            if !quotes_by_record_id.contains_key(&row.agreement) {
                return Err(Response::invalid_params(format!(
                    "ledger step '{id}' references unknown quote '{}'",
                    row.agreement
                )));
            }
            if v.payload.as_ref().map(|p| p.agreement.as_str()) != Some(&row.agreement) {
                return Err(Response::invalid_params(format!(
                    "ledger step '{id}' agreement does not match envelope payload"
                )));
            }

            history.push((
                step.record_id.clone(),
                "booking-progress".to_string(),
                step.envelope.clone(),
            ));

            let entry = highest_steps.entry(row.agreement.clone()).or_insert_with(|| {
                (step.clone(), row.slot_id.clone(), row.seat, row.created_at_secs)
            });
            if step.seq > entry.0.seq {
                *entry = (step.clone(), row.slot_id.clone(), row.seat, row.created_at_secs);
            }
        }

        validated_ledger.push((id.to_string(), row));
    }

    let mut validated_bookings = Vec::new();
    for (agr_id, (step, slot_id, seat, created_at)) in highest_steps {
        let (dec_slot, dec_seat, dec_created) =
            decisions.get(&agr_id).cloned().unwrap_or((slot_id, seat, created_at));
        let booking_row = BookingRow {
            agreement: agr_id.clone(),
            conversation: step.snapshot.conversation.clone(),
            state: step.snapshot.state,
            slot_id: dec_slot,
            seat: dec_seat,
            snapshot: step.snapshot,
            progress_record_id: step.record_id,
            updated_at_secs: dec_created,
        };
        validated_bookings.push((agr_id, booking_row));
    }

    Ok((validated_ledger, validated_bookings, history))
}

fn validate_progress(
    progress_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    owner: &str,
    now: u64,
) -> Result<ValidatedProgress, Response> {
    let mut validated = Vec::new();
    let mut history = Vec::new();
    for r in progress_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: ProgressRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("progress '{id}': invalid row: {e}")));
            }
        };

        let v = booking::verify_booking_progress(&row.envelope, now);
        if !v.verified {
            return Err(Response::invalid_params(format!(
                "progress '{id}' does not verify: {:?}",
                v.reason
            )));
        }
        if owner != row.snapshot.consumer_did {
            return Err(Response::invalid_params(format!(
                "progress '{id}' owner did not match consumer did"
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

        history.push((row.record_id.clone(), "booking-progress".to_string(), row.envelope.clone()));
        validated.push((id.to_string(), row));
    }
    Ok((validated, history))
}

pub(crate) fn validate_vertical_sections(
    bundle_sections: &std::collections::BTreeMap<String, Vec<Value>>,
    agreements_map: &HashMap<String, AgreementRow>,
    quotes_by_record_id: &QuotesByRecordId,
    owner: &str,
    now: u64,
) -> Result<ValidatedVerticalSections, Response> {
    let pay_rows = bundle_sections.get(SECTION_PAYMENTS).cloned().unwrap_or_default();
    let ful_rows = bundle_sections.get(SECTION_FULFILMENTS).cloned().unwrap_or_default();
    let led_rows = bundle_sections.get(SECTION_LEDGER).cloned().unwrap_or_default();
    let prog_rows = bundle_sections.get(SECTION_PROGRESS).cloned().unwrap_or_default();

    let mut history_entries = Vec::new();
    let (payments, h_pay) = validate_payments(pay_rows, agreements_map, now)?;
    history_entries.extend(h_pay);
    let (fulfilments, h_ful) = validate_fulfilments(ful_rows, agreements_map, now)?;
    history_entries.extend(h_ful);
    let (ledger, bookings, h_led) =
        validate_ledger_and_bookings(led_rows, quotes_by_record_id, now)?;
    history_entries.extend(h_led);
    let (progress, h_prog) = validate_progress(prog_rows, quotes_by_record_id, owner, now)?;
    history_entries.extend(h_prog);

    Ok(ValidatedVerticalSections {
        payments,
        fulfilments,
        ledger,
        bookings,
        progress,
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
    for (id, row) in validated.ledger {
        put_row(host, LEDGER, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.bookings {
        put_row(host, BOOKINGS, &id, &row).await.map_err(Response::internal_error)?;
    }
    for (id, row) in validated.progress {
        put_row(host, PROGRESS, &id, &row).await.map_err(Response::internal_error)?;
    }
    Ok(())
}
