//! Presentation methods for showing chosen hidden fields to third parties.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    clock,
    deal::sd::SignedTerms,
    disclosure::{
        COMMITMENT_SET, FULFILMENT_SET, FieldPath, PAYMENT_ACK_SET, PAYMENT_REQUEST_SET, TERMS_SET,
        decode_disclosure, disclosure_digest,
    },
    envelope::Response,
    method::Reply,
    presentation::{
        MAX_PRESENTATION_RECORDS, Presentation, PresentationView,
        verify_presentation as verify_core_presentation,
    },
    record::{
        Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_FULFILMENT_RECEIPT,
        RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST,
    },
    transaction::QuotePayload,
};

use super::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, FULFILMENTS, FulfilmentsRow, PAYMENTS,
    PaymentsRow, QUOTE_HISTORY,
    decide::{SnapshotRow, snapshot_id},
    disclosures, get_bytes, get_row,
    payment_ops::payment_id,
};
use crate::app::fulfilment_ops::fulfilment_id;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentInput {
    pub record_id: String,
    #[serde(default)]
    pub include: PresentInclude,
    #[serde(default)]
    pub show: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentInclude {
    #[serde(default = "default_true")]
    pub quote: bool,
    #[serde(default = "default_true")]
    pub receipt: bool,
    #[serde(default)]
    pub progress: bool,
    #[serde(default)]
    pub settlement: bool,
}

fn default_true() -> bool {
    true
}

impl Default for PresentInclude {
    fn default() -> Self {
        Self { quote: true, receipt: true, progress: false, settlement: false }
    }
}

pub(crate) async fn present<H: AppHost>(host: &H, input: PresentInput) -> Reply<Presentation> {
    let (quote_env, signed_terms) = load_quote_and_terms(host, &input.record_id).await?;
    let agr_opt: Option<AgreementRow> = match get_row(host, AGREEMENTS, &input.record_id).await {
        Ok(a) => a,
        Err(e) => return Err(Response::internal_error(e)),
    };

    let records = collect_presentation_records(
        host,
        &input.include,
        &input.record_id,
        quote_env.as_str(),
        agr_opt.as_ref(),
        &signed_terms,
    )
    .await?;

    if records.is_empty() || records.len() > MAX_PRESENTATION_RECORDS {
        return Err(Response::invalid_params("record-count"));
    }

    let disclosures =
        locate_disclosures_for_show(host, &input.show, &input.record_id, &records, &signed_terms)
            .await?;

    Ok(Presentation { presentation_version: 1, records, disclosures })
}

pub(crate) async fn verify_presentation<H: AppHost>(
    _host: &H,
    input: Presentation,
) -> Reply<PresentationView> {
    let now = clock::now_secs();
    match verify_core_presentation(&input, now) {
        Ok(view) => Ok(view),
        Err(e) => {
            Err(Response::invalid_params(e.code())
                .with_data(serde_json::json!({ "code": e.code() })))
        }
    }
}

async fn load_quote_and_terms<H: AppHost>(
    host: &H,
    quote_record_id: &str,
) -> Result<(String, SignedTerms), Response> {
    let bytes = match get_bytes(host, QUOTE_HISTORY, quote_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(Response::invalid_params("deal not found")),
        Err(e) => return Err(Response::internal_error(e)),
    };
    let env_str = String::from_utf8(bytes).map_err(|e| Response::internal_error(e.to_string()))?;
    let env = Envelope::from_json(&env_str).map_err(|e| Response::invalid_params(e.to_string()))?;
    let payload: QuotePayload =
        serde_json::from_value(env.payload).map_err(|e| Response::internal_error(e.to_string()))?;
    Ok((env_str, payload.terms))
}

async fn collect_presentation_records<H: AppHost>(
    host: &H,
    include: &PresentInclude,
    quote_record_id: &str,
    quote_env: &str,
    agr_opt: Option<&AgreementRow>,
    signed_terms: &SignedTerms,
) -> Result<Vec<String>, Response> {
    let mut records = Vec::new();
    if include.quote {
        records.push(quote_env.to_string());
    }
    if include.receipt
        && let Some(agr) = agr_opt
    {
        if let Some(ref h) = agr.quote_author {
            records.push(h.envelope.clone());
        }
        if let Some(ref h) = agr.counterparty {
            records.push(h.envelope.clone());
        }
    }
    if include.progress
        && let Some(agr) = agr_opt
        && let Some(seq) = agr.seq
    {
        let snap_key = snapshot_id(&agr.quote_record_id, seq);
        match get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &snap_key).await {
            Ok(Some(sr)) => records.push(sr.envelope),
            Ok(None) => {}
            Err(e) => return Err(Response::internal_error(e)),
        }
    }
    if include.settlement && agr_opt.is_some() {
        collect_settlement_records(host, quote_record_id, signed_terms, &mut records).await?;
    }
    Ok(records)
}

async fn collect_settlement_records<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    signed_terms: &SignedTerms,
    records: &mut Vec<String>,
) -> Result<(), Response> {
    for c in &signed_terms.commitments {
        let p_id = payment_id(quote_record_id, &c.commitment_id);
        match get_row::<PaymentsRow, _>(host, PAYMENTS, &p_id).await {
            Ok(Some(py)) => {
                if let Some(ref req) = py.request {
                    records.push(req.envelope.clone());
                }
                for h in &py.counterparty {
                    records.push(h.envelope.clone());
                }
                for h in &py.quote_author {
                    records.push(h.envelope.clone());
                }
            }
            Ok(None) => {}
            Err(e) => return Err(Response::internal_error(e)),
        }
        let f_id = fulfilment_id(quote_record_id, &c.commitment_id);
        match get_row::<FulfilmentsRow, _>(host, FULFILMENTS, &f_id).await {
            Ok(Some(fy)) => {
                if let Some(ref h) = fy.counterparty {
                    records.push(h.envelope.clone());
                }
                if let Some(ref h) = fy.quote_author {
                    records.push(h.envelope.clone());
                }
            }
            Ok(None) => {}
            Err(e) => return Err(Response::internal_error(e)),
        }
    }
    Ok(())
}

async fn locate_disclosures_for_show<H: AppHost>(
    host: &H,
    show_paths: &[String],
    quote_record_id: &str,
    records: &[String],
    signed_terms: &SignedTerms,
) -> Result<Vec<String>, Response> {
    let mut selected = Vec::new();
    let mut included_ids = HashSet::new();
    let mut has_terms_record = false;

    for raw in records {
        if let Ok(env) = Envelope::from_json(raw)
            && let Ok(rid) = env.record_id()
        {
            if env.record_type == "quote" || env.record_type == RECORD_AGREEMENT_RECEIPT {
                has_terms_record = true;
            }
            included_ids.insert(rid);
        }
    }

    let stored_quote_discs = disclosures::load(host, quote_record_id)
        .await
        .map_err(Response::internal_error)?
        .unwrap_or_default();

    for path_str in show_paths {
        let fp = FieldPath::parse(path_str).map_err(|_| {
            Response::invalid_params(path_str.clone())
                .with_data(serde_json::json!({ "field": path_str }))
        })?;

        let disc = match fp {
            FieldPath::Terms(ref field) => {
                if !has_terms_record || !TERMS_SET.hidden.contains(&field.as_str()) {
                    return Err(Response::invalid_params(path_str.clone())
                        .with_data(serde_json::json!({ "field": path_str })));
                }
                locate_matching_disclosure(&stored_quote_discs, field, &signed_terms.sd)
                    .ok_or_else(|| {
                        Response::invalid_params(path_str.clone())
                            .with_data(serde_json::json!({ "field": path_str }))
                    })?
            }
            FieldPath::Commitment(ref cid, ref field) => {
                if !has_terms_record || !COMMITMENT_SET.hidden.contains(&field.as_str()) {
                    return Err(Response::invalid_params(path_str.clone())
                        .with_data(serde_json::json!({ "field": path_str })));
                }
                let c = signed_terms
                    .commitments
                    .iter()
                    .find(|comm| &comm.commitment_id == cid)
                    .ok_or_else(|| {
                        Response::invalid_params(path_str.clone())
                            .with_data(serde_json::json!({ "field": path_str }))
                    })?;
                locate_matching_disclosure(&stored_quote_discs, field, &c.sd).ok_or_else(|| {
                    Response::invalid_params(path_str.clone())
                        .with_data(serde_json::json!({ "field": path_str }))
                })?
            }
            FieldPath::Record(ref rec_id, ref field) => {
                if !included_ids.contains(rec_id.as_str()) {
                    return Err(Response::invalid_params(path_str.clone())
                        .with_data(serde_json::json!({ "field": path_str })));
                }
                locate_settlement_disclosure(host, rec_id, field, records).await.ok_or_else(
                    || {
                        Response::invalid_params(path_str.clone())
                            .with_data(serde_json::json!({ "field": path_str }))
                    },
                )?
            }
        };

        if !selected.contains(&disc) {
            selected.push(disc);
        }
    }

    Ok(selected)
}

fn locate_matching_disclosure(
    stored_discs: &[String],
    field_name: &str,
    sd_list: &[String],
) -> Option<String> {
    for d in stored_discs {
        if let Ok(dec) = decode_disclosure(d)
            && dec.name == field_name
        {
            let digest = disclosure_digest(d);
            if sd_list.iter().any(|s| s == &digest) {
                return Some(d.clone());
            }
        }
    }
    None
}

async fn locate_settlement_disclosure<H: AppHost>(
    host: &H,
    rec_id: &str,
    field_name: &str,
    records: &[String],
) -> Option<String> {
    let env_raw = records.iter().find(|r| {
        Envelope::from_json(r).ok().and_then(|e| e.record_id().ok()).as_deref() == Some(rec_id)
    })?;
    let env = Envelope::from_json(env_raw).ok()?;
    let sd_vals: Vec<String> = env
        .payload
        .get("_sd")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();

    let is_valid_field = match env.record_type.as_str() {
        RECORD_PAYMENT_REQUEST => PAYMENT_REQUEST_SET.hidden.contains(&field_name),
        RECORD_PAYMENT_ACKNOWLEDGEMENT => PAYMENT_ACK_SET.hidden.contains(&field_name),
        RECORD_FULFILMENT_RECEIPT => FULFILMENT_SET.hidden.contains(&field_name),
        _ => false,
    };
    if !is_valid_field {
        return None;
    }

    let stored = disclosures::load(host, rec_id).await.ok()??;
    locate_matching_disclosure(&stored, field_name, &sd_vals)
}
