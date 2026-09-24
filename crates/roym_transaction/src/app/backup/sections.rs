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
    transaction::{AgreedTerms, Role},
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

pub(crate) async fn import_payments<H: AppHost>(
    host: &H,
    payment_rows: Vec<Value>,
    agreements: &HashMap<String, AgreementRow>,
    imported_history: &mut HashMap<String, (String, String)>,
    now: u64,
) -> Result<(), Response> {
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
            imported_history.insert(
                req_half.record_id.clone(),
                ("payment-request".to_string(), req_half.envelope.clone()),
            );
        }

        verify_payment_half_chain(
            &row.consumer,
            Role::Consumer,
            &agr.terms,
            id,
            imported_history,
            now,
        )?;
        verify_payment_half_chain(
            &row.provider,
            Role::Provider,
            &agr.terms,
            id,
            imported_history,
            now,
        )?;

        if let Err(e) = put_row(host, PAYMENTS, id, &row).await {
            return Err(Response::internal_error(e));
        }
    }
    Ok(())
}

fn verify_payment_half_chain(
    halves: &[syneroym_roym_core::transaction::ReceiptHalf],
    expected_role: Role,
    terms: &AgreedTerms,
    id: &str,
    imported_history: &mut HashMap<String, (String, String)>,
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
        imported_history.insert(
            half.record_id.clone(),
            ("payment-acknowledgement".to_string(), half.envelope.clone()),
        );
    }
    Ok(())
}

pub(crate) async fn import_fulfilments<H: AppHost>(
    host: &H,
    fulfilment_rows: Vec<Value>,
    agreements: &HashMap<String, AgreementRow>,
    imported_history: &mut HashMap<String, (String, String)>,
    now: u64,
) -> Result<(), Response> {
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
            imported_history.insert(
                c.record_id.clone(),
                ("fulfilment-receipt".to_string(), c.envelope.clone()),
            );
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
            imported_history.insert(
                p_half.record_id.clone(),
                ("fulfilment-receipt".to_string(), p_half.envelope.clone()),
            );
        }

        if let Err(e) = put_row(host, FULFILMENTS, id, &row).await {
            return Err(Response::internal_error(e));
        }
    }
    Ok(())
}

pub(crate) async fn import_ledger_and_bookings<H: AppHost>(
    host: &H,
    ledger_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    imported_history: &mut HashMap<String, (String, String)>,
    now: u64,
) -> Result<(), Response> {
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
            let quote = match quotes_by_record_id.get(&row.agreement) {
                Some(q) => q,
                None => {
                    return Err(Response::invalid_params(format!(
                        "ledger step '{id}' references unknown quote '{}'",
                        row.agreement
                    )));
                }
            };
            if v.signer_did.as_deref() != quote.signer_did.as_deref() {
                return Err(Response::invalid_params(format!(
                    "ledger step '{id}' not signed by the service that signed the quote"
                )));
            }

            imported_history.insert(
                step.record_id.clone(),
                ("booking-progress".to_string(), step.envelope.clone()),
            );

            let entry = highest_steps.entry(row.agreement.clone()).or_insert_with(|| {
                (step.clone(), row.slot_id.clone(), row.seat, row.created_at_secs)
            });
            if step.seq > entry.0.seq {
                *entry = (step.clone(), row.slot_id.clone(), row.seat, row.created_at_secs);
            }
        }

        if let Err(e) = put_row(host, LEDGER, id, &row).await {
            return Err(Response::internal_error(e));
        }
    }

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
        if let Err(e) = put_row(host, BOOKINGS, &agr_id, &booking_row).await {
            return Err(Response::internal_error(e));
        }
    }

    Ok(())
}

pub(crate) async fn import_progress<H: AppHost>(
    host: &H,
    progress_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    imported_history: &mut HashMap<String, (String, String)>,
    owner: &str,
    now: u64,
) -> Result<(), Response> {
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
        let quote = match quotes_by_record_id.get(&row.agreement) {
            Some(q) => q,
            None => {
                return Err(Response::invalid_params(format!(
                    "progress '{id}' references unknown quote '{}'",
                    row.agreement
                )));
            }
        };
        if v.signer_did.as_deref() != quote.signer_did.as_deref() {
            return Err(Response::invalid_params(format!(
                "progress '{id}' not signed by the service that signed the quote"
            )));
        }

        imported_history
            .insert(row.record_id.clone(), ("booking-progress".to_string(), row.envelope.clone()));

        if let Err(e) = put_row(host, PROGRESS, id, &row).await {
            return Err(Response::internal_error(e));
        }
    }
    Ok(())
}

pub(crate) async fn import_vertical_sections<H: AppHost>(
    host: &H,
    bundle_sections: &std::collections::BTreeMap<String, Vec<Value>>,
    agreements_map: &HashMap<String, AgreementRow>,
    quotes_by_record_id: &QuotesByRecordId,
    imported_history: &mut HashMap<String, (String, String)>,
    owner: &str,
    now: u64,
) -> Result<(), Response> {
    let pay_rows = bundle_sections.get(SECTION_PAYMENTS).cloned().unwrap_or_default();
    let ful_rows = bundle_sections.get(SECTION_FULFILMENTS).cloned().unwrap_or_default();
    let led_rows = bundle_sections.get(SECTION_LEDGER).cloned().unwrap_or_default();
    let prog_rows = bundle_sections.get(SECTION_PROGRESS).cloned().unwrap_or_default();

    import_payments(host, pay_rows, agreements_map, imported_history, now).await?;
    import_fulfilments(host, ful_rows, agreements_map, imported_history, now).await?;
    import_ledger_and_bookings(host, led_rows, quotes_by_record_id, imported_history, now).await?;
    import_progress(host, prog_rows, quotes_by_record_id, imported_history, owner, now).await?;
    Ok(())
}
