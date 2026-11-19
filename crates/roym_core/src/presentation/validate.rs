//! Validation of revealed partial deal terms and commitment fields.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::types::PresentationError;
use crate::{
    deal::{
        CommitmentKind, Details, LocalText, MAX_COMMITMENT_DESCRIPTION_LEN,
        MAX_DECLARED_VALUE_MINOR, MAX_PAYEE_LEN, MAX_SCOPE_LEN, MAX_TERMS_TEXT_LEN,
        SCHEDULING_EXTENSION,
        sd::{PartialTerms, SignedCommitment, SignedTerms},
    },
    transaction::{AgreementReceiptPayload, QuoteLocation, RecordVerdict, TimeWindow},
};

pub(super) fn validate_partial_terms_fields(
    partial: &PartialTerms,
    signed: &SignedTerms,
    receipts: &[(&RecordVerdict<AgreementReceiptPayload>, &AgreementReceiptPayload)],
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let obj = partial
        .value
        .as_object()
        .ok_or_else(|| PresentationError::RecordRefused("terms value not an object".to_string()))?;

    let mut all_shown_texts = Vec::new();
    let total_text_fields_count = 4 + signed.commitments.len(); // scope, cancellation, refund, dispute + descriptions

    validate_terms_level_texts(obj, &mut all_shown_texts)?;
    validate_scope_receipt_ml18(obj, receipts, not_checked)?;

    let comms_val = obj
        .get("commitments")
        .and_then(Value::as_array)
        .ok_or_else(|| PresentationError::RecordRefused("commitments not an array".to_string()))?;

    for (i, c_val) in comms_val.iter().enumerate() {
        let c_orig = &signed.commitments[i];
        let c_map = c_val.as_object().ok_or_else(|| {
            PresentationError::RecordRefused("commitment not an object".to_string())
        })?;
        validate_single_commitment_partial(c_map, c_orig, &mut all_shown_texts, not_checked)?;
    }

    if all_shown_texts.len() > 1 {
        let first = &all_shown_texts[0];
        let first_langs: HashSet<_> = first.translations.iter().map(|tr| &tr.lang).collect();
        for t in &all_shown_texts[1..] {
            let t_langs: HashSet<_> = t.translations.iter().map(|tr| &tr.lang).collect();
            if t.lang != first.lang || t_langs != first_langs {
                return Err(PresentationError::FieldInvalid(
                    "lang".to_string(),
                    "texts have different languages or translation sets".to_string(),
                ));
            }
        }
    }
    if all_shown_texts.len() < total_text_fields_count {
        not_checked.push("deal_terms.all_texts_same_language".to_string());
    }

    Ok(())
}

fn validate_terms_level_texts(
    obj: &Map<String, Value>,
    all_texts: &mut Vec<LocalText>,
) -> Result<(), PresentationError> {
    if let Some(val) = obj.get("scope")
        && !val.is_null()
    {
        let text: LocalText = serde_json::from_value(val.clone())
            .map_err(|e| PresentationError::FieldInvalid("scope".to_string(), e.to_string()))?;
        text.validate("scope", MAX_SCOPE_LEN)
            .map_err(|e| PresentationError::FieldInvalid("scope".to_string(), e.to_string()))?;
        all_texts.push(text);
    }
    if let Some(val) = obj.get("cancellation_terms")
        && !val.is_null()
    {
        let text: LocalText = serde_json::from_value(val.clone()).map_err(|e| {
            PresentationError::FieldInvalid("cancellation_terms".to_string(), e.to_string())
        })?;
        text.validate("cancellation_terms", MAX_TERMS_TEXT_LEN).map_err(|e| {
            PresentationError::FieldInvalid("cancellation_terms".to_string(), e.to_string())
        })?;
        all_texts.push(text);
    }
    if let Some(val) = obj.get("refund_terms")
        && !val.is_null()
    {
        let text: LocalText = serde_json::from_value(val.clone()).map_err(|e| {
            PresentationError::FieldInvalid("refund_terms".to_string(), e.to_string())
        })?;
        text.validate("refund_terms", MAX_TERMS_TEXT_LEN).map_err(|e| {
            PresentationError::FieldInvalid("refund_terms".to_string(), e.to_string())
        })?;
        all_texts.push(text);
    }
    if let Some(val) = obj.get("dispute_path")
        && !val.is_null()
    {
        let text: LocalText = serde_json::from_value(val.clone()).map_err(|e| {
            PresentationError::FieldInvalid("dispute_path".to_string(), e.to_string())
        })?;
        text.validate("dispute_path", MAX_TERMS_TEXT_LEN).map_err(|e| {
            PresentationError::FieldInvalid("dispute_path".to_string(), e.to_string())
        })?;
        all_texts.push(text);
    }
    Ok(())
}

fn validate_scope_receipt_ml18(
    obj: &Map<String, Value>,
    receipts: &[(&RecordVerdict<AgreementReceiptPayload>, &AgreementReceiptPayload)],
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    if receipts.is_empty() {
        return Ok(());
    }
    if let Some(val) = obj.get("scope")
        && !val.is_null()
    {
        let scope: LocalText = serde_json::from_value(val.clone())
            .map_err(|e| PresentationError::FieldInvalid("scope".to_string(), e.to_string()))?;
        for (_, r) in receipts {
            if r.viewed_via_mt && r.viewed_lang == scope.lang {
                return Err(PresentationError::FieldInvalid(
                    "viewed_via_mt".to_string(),
                    "viewed_via_mt is true when viewed_lang equals scope original language"
                        .to_string(),
                ));
            }
        }
    } else {
        not_checked.push("agreement_receipt.viewed_via_mt_scope_lang".to_string());
    }
    Ok(())
}

fn validate_single_commitment_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    all_texts: &mut Vec<LocalText>,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    if let Some(val) = c_map.get("description")
        && !val.is_null()
    {
        let desc: LocalText = serde_json::from_value(val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{}].description", c_orig.commitment_id),
                e.to_string(),
            )
        })?;
        desc.validate("description", MAX_COMMITMENT_DESCRIPTION_LEN).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{}].description", c_orig.commitment_id),
                e.to_string(),
            )
        })?;
        all_texts.push(desc);
    }

    if let Some(val) = c_map.get("window")
        && !val.is_null()
    {
        let w: TimeWindow = serde_json::from_value(val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{}].window", c_orig.commitment_id),
                e.to_string(),
            )
        })?;
        w.validate().map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{}].window", c_orig.commitment_id),
                e.to_string(),
            )
        })?;
    }

    if let Some(val) = c_map.get("location")
        && !val.is_null()
    {
        let loc: QuoteLocation = serde_json::from_value(val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{}].location", c_orig.commitment_id),
                e.to_string(),
            )
        })?;
        loc.validate().map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{}].location", c_orig.commitment_id),
                e.to_string(),
            )
        })?;
    }

    if let Some(val) = c_map.get("eep")
        && !val.is_null()
    {
        return Err(PresentationError::FieldInvalid(
            format!("terms.commitments[{}].eep", c_orig.commitment_id),
            "eep must be null in deal core".to_string(),
        ));
    }

    validate_commitment_values_partial(c_map, c_orig, not_checked)?;
    validate_commitment_details_partial(c_map, c_orig, not_checked)?;
    Ok(())
}

fn validate_commitment_values_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let cid = &c_orig.commitment_id;
    let decl_opt = c_map.get("declared_value_minor").and_then(Value::as_i64);
    if let Some(val) = decl_opt {
        if !(0..=MAX_DECLARED_VALUE_MINOR).contains(&val) {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].declared_value_minor"),
                "value outside range".to_string(),
            ));
        }
        if c_orig.kind == CommitmentKind::Currency && val < 1 {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].declared_value_minor"),
                "currency commitment value must be >= 1".to_string(),
            ));
        }
    }

    if c_orig.kind == CommitmentKind::Currency {
        if let Some(payee_val) = c_map.get("payee") {
            let payee = payee_val.as_str().unwrap_or_default();
            if payee.trim().is_empty() || payee.len() > MAX_PAYEE_LEN {
                return Err(PresentationError::FieldInvalid(
                    format!("terms.commitments[{cid}].payee"),
                    "payee empty or too long".to_string(),
                ));
            }
        } else {
            not_checked.push(format!("terms.commitments[{cid}].payee_not_empty"));
        }

        let tax_opt = c_map.get("tax_minor").and_then(Value::as_i64);
        let fees_opt = c_map.get("fees_minor").and_then(Value::as_i64);
        if let Some(t) = tax_opt
            && t < 0
        {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].tax_minor"),
                "tax cannot be negative".to_string(),
            ));
        }
        if let Some(f) = fees_opt
            && f < 0
        {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].fees_minor"),
                "fees cannot be negative".to_string(),
            ));
        }

        if let (Some(val), Some(tax), Some(fees)) = (decl_opt, tax_opt, fees_opt) {
            if tax.saturating_add(fees) > val {
                return Err(PresentationError::FieldInvalid(
                    format!("terms.commitments[{cid}].tax_minor"),
                    "tax plus fees exceeds declared value".to_string(),
                ));
            }
        } else {
            not_checked.push(format!("terms.commitments[{cid}].tax_fees_within_value"));
        }
    } else {
        if c_map.get("payee").is_some_and(|v| !v.is_null())
            || c_map.get("payment_methods").is_some_and(|v| !v.is_null())
            || c_map.get("tax_minor").is_some_and(|v| !v.is_null())
            || c_map.get("fees_minor").is_some_and(|v| !v.is_null())
        {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].currency_fields"),
                "currency fields must be null on non-currency commitment".to_string(),
            ));
        }
        if c_map.get("payee").is_none()
            || c_map.get("payment_methods").is_none()
            || c_map.get("tax_minor").is_none()
            || c_map.get("fees_minor").is_none()
        {
            not_checked.push(format!("terms.commitments[{cid}].currency_fields_null"));
        }
    }

    Ok(())
}

fn validate_commitment_details_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let cid = &c_orig.commitment_id;
    if let Some(ref ext) = c_orig.extension {
        if let Some(det_val) = c_map.get("details")
            && !det_val.is_null()
        {
            let details: Details = serde_json::from_value(det_val.clone()).map_err(|e| {
                PresentationError::FieldInvalid(
                    format!("terms.commitments[{cid}].details"),
                    e.to_string(),
                )
            })?;
            if details.extension != ext.id {
                return Err(PresentationError::FieldInvalid(
                    format!("terms.commitments[{cid}].details"),
                    "details extension mismatch".to_string(),
                ));
            }
            if ext.id == SCHEDULING_EXTENSION {
                if let Some(win_val) = c_map.get("window")
                    && !win_val.is_null()
                {
                    let _w: TimeWindow = serde_json::from_value(win_val.clone()).map_err(|e| {
                        PresentationError::FieldInvalid(
                            format!("terms.commitments[{cid}].window"),
                            e.to_string(),
                        )
                    })?;
                } else {
                    not_checked.push(format!("terms.commitments[{cid}].details_matches_extension"));
                }
            }
        } else {
            not_checked.push(format!("terms.commitments[{cid}].details_matches_extension"));
        }
    } else if let Some(det_val) = c_map.get("details")
        && !det_val.is_null()
    {
        return Err(PresentationError::FieldInvalid(
            format!("terms.commitments[{cid}].details"),
            "details present without extension".to_string(),
        ));
    }
    Ok(())
}
