//! Backup verification and import for the `disclosures` section.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    deal::{DealTerms, sd},
    disclosure::{
        FULFILMENT_SET, PAYMENT_ACK_SET, PAYMENT_REQUEST_SET, decode_base64url, decode_disclosure,
        reveal_hidden_strict,
    },
    envelope::Response,
    fulfilment::{self, FulfilmentHidden},
    payment::{self, PaymentAckHidden, PaymentRequestHidden},
    transaction,
};

use super::super::{
    CardRow, DISCLOSURES,
    disclosures::{DisclosureRow, plain_view},
};

pub(crate) struct ValidatedDisclosures {
    pub(crate) by_record_id: HashMap<String, Vec<String>>,
    pub(crate) revealed_quotes: HashMap<String, DealTerms>,
    pub(crate) rows: Vec<DisclosureRow>,
}

/// Verifies the `disclosures` section against the records present in the
/// bundle.
///
/// Refuses if any disclosure row names a record not in the bundle, if any
/// disclosure matches no digest of that record, or if a quote's disclosure set
/// is not complete and strict.
pub(crate) fn validate_disclosures_section(
    bundle_sections: &BTreeMap<String, Vec<Value>>,
    now: u64,
) -> Result<ValidatedDisclosures, Response> {
    let empty = Vec::new();
    let disc_rows = bundle_sections.get("disclosures").unwrap_or(&empty);
    let record_envelopes = collect_bundle_record_envelopes(bundle_sections);

    let mut quote_record_ids = HashSet::new();
    for r in bundle_sections.get("quotes").unwrap_or(&empty) {
        if let Some(payload) = r.get("payload")
            && let Some(rid) = payload.get("record_id").and_then(Value::as_str)
            && !rid.is_empty()
        {
            quote_record_ids.insert(rid.to_string());
        }
    }
    for r in bundle_sections.get("quote_history").unwrap_or(&empty) {
        if let Some(id) = r.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            quote_record_ids.insert(id.to_string());
        }
    }

    let mut by_record_id = HashMap::with_capacity(disc_rows.len());
    let mut rows = Vec::with_capacity(disc_rows.len());
    let mut revealed_quotes = HashMap::new();

    for r in disc_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: DisclosureRow = serde_json::from_value(payload).map_err(|e| {
            Response::invalid_params(format!("disclosures '{id}': invalid row: {e}"))
        })?;

        if id != row.record_id {
            return Err(Response::invalid_params(format!(
                "disclosures '{id}' id does not match payload record_id '{}'",
                row.record_id
            )));
        }

        validate_disclosure_strings(&row.disclosures, id)?;

        let env_str = record_envelopes.get(&row.record_id).ok_or_else(|| {
            Response::invalid_params(format!(
                "disclosures section names record '{}' not present in bundle",
                row.record_id
            ))
        })?;

        if let Some(terms) =
            verify_record_disclosures(&row.record_id, env_str, &row.disclosures, now)?
        {
            revealed_quotes.insert(row.record_id.clone(), terms);
        }

        by_record_id.insert(row.record_id.clone(), row.disclosures.clone());
        rows.push(row);
    }

    for rid in &quote_record_ids {
        if !by_record_id.contains_key(rid) {
            return Err(Response::invalid_params(format!("quote '{rid}': disclosures missing")));
        }
    }

    Ok(ValidatedDisclosures { by_record_id, revealed_quotes, rows })
}

fn validate_disclosure_strings(disclosures: &[String], id: &str) -> Result<(), Response> {
    for d in disclosures {
        let decoded = decode_disclosure(d).map_err(|e| {
            Response::invalid_params(format!(
                "disclosures '{id}': invalid disclosure encoding: {e}"
            ))
        })?;
        let salt_bytes = decode_base64url(&decoded.salt).map_err(|_| {
            Response::invalid_params(format!("disclosures '{id}': invalid salt base64url"))
        })?;
        if salt_bytes.len() != 16 {
            return Err(Response::invalid_params(format!(
                "disclosures '{id}': salt must be 16 bytes, found {}",
                salt_bytes.len()
            )));
        }
    }
    Ok(())
}

fn verify_record_disclosures(
    record_id: &str,
    env_str: &str,
    disclosures: &[String],
    now: u64,
) -> Result<Option<DealTerms>, Response> {
    let q_verdict = transaction::verify_quote(env_str, now);
    if q_verdict.verified {
        let signed = q_verdict.payload.ok_or_else(|| {
            Response::invalid_params(format!("quote '{record_id}' missing payload"))
        })?;
        let terms = sd::reveal_terms_strict(&signed.terms, disclosures).map_err(|e| {
            Response::invalid_params(format!(
                "quote '{record_id}' disclosures do not strictly reveal: {e}"
            ))
        })?;
        return Ok(Some(terms));
    }

    let p_req_verdict = payment::verify_payment_request(env_str, now);
    if p_req_verdict.verified {
        let p = p_req_verdict.payload.ok_or_else(|| {
            Response::invalid_params(format!("payment request '{record_id}' missing payload"))
        })?;
        reveal_hidden_strict::<PaymentRequestHidden>(&p.sd, &PAYMENT_REQUEST_SET, disclosures)
            .map_err(|e| {
                Response::invalid_params(format!(
                    "payment request '{record_id}' disclosures do not strictly reveal: {e}"
                ))
            })?;
        return Ok(None);
    }

    let p_ack_verdict = payment::verify_payment_acknowledgement(env_str, now);
    if p_ack_verdict.verified {
        let p = p_ack_verdict.payload.ok_or_else(|| {
            Response::invalid_params(format!("payment ack '{record_id}' missing payload"))
        })?;
        reveal_hidden_strict::<PaymentAckHidden>(&p.sd, &PAYMENT_ACK_SET, disclosures).map_err(
            |e| {
                Response::invalid_params(format!(
                    "payment ack '{record_id}' disclosures do not strictly reveal: {e}"
                ))
            },
        )?;
        return Ok(None);
    }

    let f_verdict = fulfilment::verify_fulfilment_receipt(env_str, now);
    if f_verdict.verified {
        let p = f_verdict.payload.ok_or_else(|| {
            Response::invalid_params(format!("fulfilment receipt '{record_id}' missing payload"))
        })?;
        reveal_hidden_strict::<FulfilmentHidden>(&p.sd, &FULFILMENT_SET, disclosures).map_err(
            |e| {
                Response::invalid_params(format!(
                    "fulfilment '{record_id}' disclosures do not strictly reveal: {e}"
                ))
            },
        )?;
        return Ok(None);
    }

    Err(Response::invalid_params(format!(
        "disclosures '{record_id}': record envelope does not verify as a known deal record"
    )))
}

fn collect_bundle_record_envelopes(
    sections: &BTreeMap<String, Vec<Value>>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let empty = Vec::new();

    for r in sections.get("quotes").unwrap_or(&empty) {
        if let Some(payload) = r.get("payload")
            && let Some(env) = payload.get("envelope").and_then(Value::as_str)
            && let Some(rid) = payload.get("record_id").and_then(Value::as_str)
        {
            map.insert(rid.to_string(), env.to_string());
        }
    }

    for r in sections.get("quote_history").unwrap_or(&empty) {
        if let Some(id) = r.get("id").and_then(Value::as_str) {
            let env = r
                .get("payload")
                .and_then(|p| p.get("envelope").and_then(Value::as_str).or_else(|| p.as_str()));
            if let Some(env) = env {
                map.insert(id.to_string(), env.to_string());
            }
        }
    }

    for r in sections.get("payments").unwrap_or(&empty) {
        if let Some(payload) = r.get("payload") {
            if let Some(req) = payload.get("request")
                && let Some(rid) = req.get("record_id").and_then(Value::as_str)
                && let Some(env) = req.get("envelope").and_then(Value::as_str)
            {
                map.insert(rid.to_string(), env.to_string());
            }
            for list_key in ["counterparty", "quote_author"] {
                if let Some(arr) = payload.get(list_key).and_then(Value::as_array) {
                    for half in arr {
                        if let Some(rid) = half.get("record_id").and_then(Value::as_str)
                            && let Some(env) = half.get("envelope").and_then(Value::as_str)
                        {
                            map.insert(rid.to_string(), env.to_string());
                        }
                    }
                }
            }
        }
    }

    for r in sections.get("fulfilments").unwrap_or(&empty) {
        if let Some(payload) = r.get("payload") {
            for key in ["counterparty", "quote_author"] {
                if let Some(half) = payload.get(key)
                    && let Some(rid) = half.get("record_id").and_then(Value::as_str)
                    && let Some(env) = half.get("envelope").and_then(Value::as_str)
                {
                    map.insert(rid.to_string(), env.to_string());
                }
            }
        }
    }

    map
}

/// Rebuilds a deal card row's `data` using disclosures.
///
/// If disclosures are missing for a record that requires them, marks the card
/// as not verified with the reason "disclosures missing".
pub(crate) fn rebuild_card_data(
    row: &mut CardRow,
    kind: &str,
    env_str: &str,
    disclosures_map: &HashMap<String, Vec<String>>,
    now: u64,
) {
    match kind {
        "quote" => {
            let v = transaction::verify_quote(env_str, now);
            if let Some(ref p) = v.payload
                && let Some(ref rid) = row.record_id
            {
                if let Some(disc) = disclosures_map.get(rid) {
                    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
                    row.data = Some(plain_view(&p_val, disc));
                } else {
                    row.verified = false;
                    row.reason = Some("disclosures missing".to_string());
                }
            }
        }
        "agreement-receipt" => {
            let v = transaction::verify_agreement_receipt(env_str, now);
            if let Some(ref p) = v.payload {
                if let Some(disc) = disclosures_map.get(&p.quote_record_id) {
                    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
                    row.data = Some(plain_view(&p_val, disc));
                } else {
                    row.verified = false;
                    row.reason = Some("disclosures missing".to_string());
                }
            }
        }
        "payment-request" => {
            let v = payment::verify_payment_request(env_str, now);
            if let Some(ref p) = v.payload
                && let Some(ref rid) = row.record_id
            {
                if let Some(disc) = disclosures_map.get(rid) {
                    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
                    row.data = Some(plain_view(&p_val, disc));
                } else {
                    row.verified = false;
                    row.reason = Some("disclosures missing".to_string());
                }
            }
        }
        "payment-acknowledgement" => {
            let v = payment::verify_payment_acknowledgement(env_str, now);
            if let Some(ref p) = v.payload
                && let Some(ref rid) = row.record_id
            {
                if let Some(disc) = disclosures_map.get(rid) {
                    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
                    row.data = Some(plain_view(&p_val, disc));
                } else {
                    row.verified = false;
                    row.reason = Some("disclosures missing".to_string());
                }
            }
        }
        "fulfilment-receipt" => {
            let v = fulfilment::verify_fulfilment_receipt(env_str, now);
            if let Some(ref p) = v.payload
                && let Some(ref rid) = row.record_id
            {
                if let Some(disc) = disclosures_map.get(rid) {
                    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
                    row.data = Some(plain_view(&p_val, disc));
                } else {
                    row.verified = false;
                    row.reason = Some("disclosures missing".to_string());
                }
            }
        }
        _ => {}
    }
}

/// Writes all verified disclosure rows to the `disclosures` collection.
pub(crate) async fn write_imported_disclosures<H: AppHost>(
    host: &H,
    disclosures: &[DisclosureRow],
) -> Result<(), Response> {
    for row in disclosures {
        let bytes = serde_json::to_vec(row).map_err(|e| Response::internal_error(e.to_string()))?;
        let write = RecordWriteValue { id: row.record_id.clone(), payload: bytes };
        if let Err(e) = AppDataLayer::create(host, DISCLOSURES.to_string(), vec![write]).await {
            return Err(Response::internal_error(e.to_string()));
        }
    }
    Ok(())
}
