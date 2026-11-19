use serde_json::{Map, Value};

use super::{
    types::{PresentationError, SettlementView},
    verify::VerifiedRecord,
};
use crate::{
    deal::SettlementRole,
    disclosure::{
        DisclosureIndex, FULFILMENT_SET, HiddenSet, PAYMENT_ACK_SET, PAYMENT_REQUEST_SET,
        reveal_object,
    },
    fulfilment::FulfilmentHidden,
    payment::{PaymentAckHidden, PaymentRequestHidden},
};

struct SettlementTarget<'a> {
    record_id: &'a str,
    issuer: &'a str,
    commitment_id: &'a str,
    role: SettlementRole,
    supersedes: Option<String>,
    observed_at_secs: Option<u64>,
    set: &'a HiddenSet,
}

pub(crate) fn reveal_settlement_views(
    settlements: &[&VerifiedRecord],
    index: &DisclosureIndex,
    all_used: &mut Vec<String>,
) -> Result<Vec<SettlementView>, PresentationError> {
    let mut views = Vec::with_capacity(settlements.len());
    for s in settlements {
        let view = match s {
            VerifiedRecord::PaymentRequest(v, p) => reveal_settlement_record(
                SettlementTarget {
                    record_id: v.record_id.as_deref().unwrap_or_default(),
                    issuer: v.issuer.as_deref().unwrap_or_default(),
                    commitment_id: &p.commitment_id,
                    role: SettlementRole::Receiver,
                    supersedes: None,
                    observed_at_secs: None,
                    set: &PAYMENT_REQUEST_SET,
                },
                &p.sd,
                index,
                all_used,
            )?,
            VerifiedRecord::PaymentAck(v, p) => reveal_settlement_record(
                SettlementTarget {
                    record_id: v.record_id.as_deref().unwrap_or_default(),
                    issuer: v.issuer.as_deref().unwrap_or_default(),
                    commitment_id: &p.commitment_id,
                    role: p.role,
                    supersedes: v.supersedes.clone(),
                    observed_at_secs: Some(p.observed_at_secs),
                    set: &PAYMENT_ACK_SET,
                },
                &p.sd,
                index,
                all_used,
            )?,
            VerifiedRecord::Fulfilment(v, p) => reveal_settlement_record(
                SettlementTarget {
                    record_id: v.record_id.as_deref().unwrap_or_default(),
                    issuer: v.issuer.as_deref().unwrap_or_default(),
                    commitment_id: &p.commitment_id,
                    role: p.role,
                    supersedes: v.supersedes.clone(),
                    observed_at_secs: None,
                    set: &FULFILMENT_SET,
                },
                &p.sd,
                index,
                all_used,
            )?,
            _ => continue,
        };
        views.push(view);
    }
    Ok(views)
}

fn reveal_settlement_record(
    target: SettlementTarget<'_>,
    sd: &[String],
    index: &DisclosureIndex,
    all_used: &mut Vec<String>,
) -> Result<SettlementView, PresentationError> {
    let mut obj = Map::new();
    obj.insert("_sd".to_string(), Value::Array(sd.iter().cloned().map(Value::String).collect()));
    let revealed = reveal_object(&obj, target.set, index).map_err(PresentationError::Sd)?;

    for u in revealed.used {
        all_used.push(u);
    }

    validate_revealed_settlement_fields(&revealed.fields, target.set)?;

    let hidden = revealed.hidden.into_iter().map(|s| s.to_string()).collect();
    Ok(SettlementView {
        record_id: target.record_id.to_string(),
        issuer: target.issuer.to_string(),
        commitment_id: target.commitment_id.to_string(),
        role: target.role,
        supersedes: target.supersedes,
        observed_at_secs: target.observed_at_secs,
        shown: Value::Object(revealed.fields),
        hidden,
    })
}

fn validate_revealed_settlement_fields(
    fields: &Map<String, Value>,
    set: &HiddenSet,
) -> Result<(), PresentationError> {
    let val = Value::Object(fields.clone());
    if set == &PAYMENT_REQUEST_SET {
        let h: PaymentRequestHidden = serde_json::from_value(val)
            .map_err(|e| PresentationError::FieldInvalid("note".to_string(), e.to_string()))?;
        h.validate()
            .map_err(|e| PresentationError::FieldInvalid("note".to_string(), e.to_string()))?;
    } else if set == &PAYMENT_ACK_SET {
        let h: PaymentAckHidden = serde_json::from_value(val).map_err(|e| {
            PresentationError::FieldInvalid("settlement".to_string(), e.to_string())
        })?;
        h.validate().map_err(|e| {
            PresentationError::FieldInvalid("settlement".to_string(), e.to_string())
        })?;
    } else if set == &FULFILMENT_SET {
        let h: FulfilmentHidden = serde_json::from_value(val).map_err(|e| {
            PresentationError::FieldInvalid("settlement_note".to_string(), e.to_string())
        })?;
        h.validate().map_err(|e| {
            PresentationError::FieldInvalid("settlement_note".to_string(), e.to_string())
        })?;
    }
    Ok(())
}
