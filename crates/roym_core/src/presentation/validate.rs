//! Validation of revealed partial deal terms and commitment fields.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::types::PresentationError;
use crate::{
    deal::{
        CommitmentKind, Details, LocalText, MAX_COMMITMENT_DESCRIPTION_LEN,
        MAX_DECLARED_VALUE_MINOR, MAX_PAYEE_LEN, MAX_PAYMENT_METHODS, MAX_SCOPE_LEN,
        MAX_TERMS_TEXT_LEN,
        sd::{PartialTerms, SignedCommitment, SignedTerms},
    },
    extension::{self, PartialCheck},
    listing::{self, MAX_PAYMENT_METHOD_LEN},
    transaction::{AgreementReceiptPayload, QuoteLocation, RecordVerdict, TimeWindow},
};

pub(crate) fn validate_partial_terms_fields(
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

const TERMS_TEXT_FIELDS: &[(&str, usize)] = &[
    ("scope", MAX_SCOPE_LEN),
    ("cancellation_terms", MAX_TERMS_TEXT_LEN),
    ("refund_terms", MAX_TERMS_TEXT_LEN),
    ("dispute_path", MAX_TERMS_TEXT_LEN),
];

fn validate_terms_level_texts(
    obj: &Map<String, Value>,
    all_texts: &mut Vec<LocalText>,
) -> Result<(), PresentationError> {
    if let Some(val) = obj.get("listing_id")
        && !val.is_null()
        && val.as_str().is_none()
    {
        return Err(PresentationError::FieldInvalid(
            "listing_id".to_string(),
            "listing_id must be a string".to_string(),
        ));
    }
    for &(field, limit) in TERMS_TEXT_FIELDS {
        if let Some(val) = obj.get(field) {
            if val.is_null() {
                return Err(PresentationError::FieldInvalid(
                    field.to_string(),
                    format!("{field} cannot be null"),
                ));
            }
            let text: LocalText = serde_json::from_value(val.clone())
                .map_err(|e| PresentationError::FieldInvalid(field.to_string(), e.to_string()))?;
            text.validate(field, limit)
                .map_err(|e| PresentationError::FieldInvalid(field.to_string(), e.to_string()))?;
            all_texts.push(text);
        }
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
    if let Some(val) = c_map.get("description") {
        if val.is_null() {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{}].description", c_orig.commitment_id),
                "description cannot be null".to_string(),
            ));
        }
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

fn validate_declared_value_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
) -> Result<Option<i64>, PresentationError> {
    let cid = &c_orig.commitment_id;
    if let Some(val_raw) = c_map.get("declared_value_minor") {
        if val_raw.is_null() {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].declared_value_minor"),
                "declared_value_minor cannot be null".to_string(),
            ));
        }
        let val: i64 = serde_json::from_value(val_raw.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].declared_value_minor"),
                e.to_string(),
            )
        })?;
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
        Ok(Some(val))
    } else {
        Ok(None)
    }
}

fn validate_currency_commitment_fields_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    decl_opt: Option<i64>,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let cid = &c_orig.commitment_id;
    if let Some(payee_val) = c_map.get("payee") {
        if payee_val.is_null() {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].payee"),
                "payee cannot be null".to_string(),
            ));
        }
        let payee: String = serde_json::from_value(payee_val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].payee"),
                e.to_string(),
            )
        })?;
        if payee.trim().is_empty() || payee.len() > MAX_PAYEE_LEN {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].payee"),
                "payee empty or too long".to_string(),
            ));
        }
    } else {
        not_checked.push(format!("terms.commitments[{cid}].payee_not_empty"));
    }

    if let Some(methods_val) = c_map.get("payment_methods")
        && !methods_val.is_null()
    {
        let methods: Vec<String> = serde_json::from_value(methods_val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].payment_methods"),
                e.to_string(),
            )
        })?;
        if methods.len() > MAX_PAYMENT_METHODS {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].payment_methods"),
                "too many payment methods".to_string(),
            ));
        }
        for m in &methods {
            if !listing::valid_token(m, MAX_PAYMENT_METHOD_LEN) {
                return Err(PresentationError::FieldInvalid(
                    format!("terms.commitments[{cid}].payment_methods"),
                    "invalid payment method token".to_string(),
                ));
            }
        }
    }

    let tax_opt = if let Some(t_val) = c_map.get("tax_minor")
        && !t_val.is_null()
    {
        let t: i64 = serde_json::from_value(t_val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].tax_minor"),
                e.to_string(),
            )
        })?;
        if t < 0 {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].tax_minor"),
                "tax cannot be negative".to_string(),
            ));
        }
        Some(t)
    } else {
        None
    };

    let fees_opt = if let Some(f_val) = c_map.get("fees_minor")
        && !f_val.is_null()
    {
        let f: i64 = serde_json::from_value(f_val.clone()).map_err(|e| {
            PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].fees_minor"),
                e.to_string(),
            )
        })?;
        if f < 0 {
            return Err(PresentationError::FieldInvalid(
                format!("terms.commitments[{cid}].fees_minor"),
                "fees cannot be negative".to_string(),
            ));
        }
        Some(f)
    } else {
        None
    };

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

    Ok(())
}

fn validate_non_currency_commitment_fields_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let cid = &c_orig.commitment_id;
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
    Ok(())
}

fn validate_commitment_values_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let decl_opt = validate_declared_value_partial(c_map, c_orig)?;
    if c_orig.kind == CommitmentKind::Currency {
        validate_currency_commitment_fields_partial(c_map, c_orig, decl_opt, not_checked)
    } else {
        validate_non_currency_commitment_fields_partial(c_map, c_orig, not_checked)
    }
}

fn validate_commitment_details_partial(
    c_map: &Map<String, Value>,
    c_orig: &SignedCommitment,
    not_checked: &mut Vec<String>,
) -> Result<(), PresentationError> {
    let cid = &c_orig.commitment_id;
    let details_path = format!("terms.commitments[{cid}].details");
    if let Some(ref ext) = c_orig.extension {
        let details = match c_map.get("details") {
            Some(det_val) if det_val.is_null() => {
                return Err(PresentationError::FieldInvalid(
                    details_path,
                    "details cannot be null when extension is set".to_string(),
                ));
            }
            Some(det_val) => {
                Some(serde_json::from_value::<Details>(det_val.clone()).map_err(|e| {
                    PresentationError::FieldInvalid(details_path.clone(), e.to_string())
                })?)
            }
            None => None,
        };
        let window = match c_map.get("window") {
            Some(win_val) if win_val.is_null() => Some(None),
            Some(win_val) => {
                Some(Some(serde_json::from_value::<TimeWindow>(win_val.clone()).map_err(|e| {
                    PresentationError::FieldInvalid(
                        format!("terms.commitments[{cid}].window"),
                        e.to_string(),
                    )
                })?))
            }
            None => None,
        };
        let verdict = extension::check_partial_details(
            extension::registry(),
            ext,
            details.as_ref(),
            window.as_ref().map(Option::as_ref),
        )
        .map_err(|e| PresentationError::FieldInvalid(details_path, e.to_string()))?;
        if verdict != PartialCheck::Checked {
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
