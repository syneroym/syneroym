//! Card filing helpers for enquiry and quote messages.

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{deal::sd::reveal_terms_strict, record::RECORD_QUOTE, transaction};

use super::{FileCardResult, refuse_card};
use crate::app::{
    CardRow, ENQUIRY_HISTORY, QUOTE_HISTORY, disclosures, enquiry_ops::store_received_enquiry,
    get_bytes, put_card, quote_ops::store_received_quote,
};

pub(super) async fn file_enquiry_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    if !disclosures.is_empty() {
        return refuse_card(host, row, "disclosures not permitted on enquiry card").await;
    }
    let v = transaction::verify_enquiry(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "enquiry does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, row, "card names another conversation").await;
    }
    let already_held = match &v.record_id {
        Some(rid) => get_bytes(host, ENQUIRY_HISTORY, rid).await?.is_some(),
        None => false,
    };
    let version_count = match store_received_enquiry(host, envelope, &v, payload, now, owner).await
    {
        Ok(vc) => vc,
        Err(e) => return refuse_card(host, row, e).await,
    };
    row.verified = true;
    row.repeat = already_held;
    row.version_count = Some(version_count);
    row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

pub(super) async fn file_quote_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_quote(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "quote does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, row, "card names another conversation").await;
    }
    let rid = v.record_id.as_deref().unwrap_or_default();
    let already_held = match &v.record_id {
        Some(rid) => get_bytes(host, QUOTE_HISTORY, rid).await?.is_some(),
        None => false,
    };

    let effective_disclosures = if already_held && disclosures.is_empty() {
        match disclosures::load(host, rid).await? {
            Some(dr) => dr,
            None => return refuse_card(host, row, "disclosures missing for held quote").await,
        }
    } else {
        disclosures.to_vec()
    };

    let plain_terms = match reveal_terms_strict(&payload.terms, &effective_disclosures) {
        Ok(t) => t,
        Err(e) => {
            return refuse_card(host, row, format!("quote disclosures invalid: {}", e.code()))
                .await;
        }
    };
    if let Err(e) = plain_terms.validate() {
        return refuse_card(host, row, e.to_string()).await;
    }

    let enquiry_rec_id = match &payload.answers {
        transaction::Answers::Enquiry { record_id } => record_id,
        transaction::Answers::Want { .. } => {
            return refuse_card(host, row, "answers a want, which is not supported").await;
        }
    };
    let req_bytes = match get_bytes(host, ENQUIRY_HISTORY, enquiry_rec_id).await? {
        Some(b) => b,
        None => {
            return refuse_card(host, row, "answers an enquiry this node does not hold").await;
        }
    };
    let req_str = match String::from_utf8(req_bytes) {
        Ok(s) => s,
        Err(_) => {
            return refuse_card(host, row, "stored enquiry envelope is invalid utf8").await;
        }
    };
    let req_v = transaction::verify_enquiry(&req_str, now);
    if !req_v.verified || req_v.issuer.as_deref() != Some(payload.counterparty_did.as_str()) {
        return refuse_card(host, row, "quote counterparty_did does not match enquiry issuer")
            .await;
    }

    disclosures::store(host, rid, RECORD_QUOTE, &effective_disclosures, now).await?;

    let version_count = match store_received_quote(host, envelope, &v, payload, now, owner).await {
        Ok(vc) => vc,
        Err(e) => return refuse_card(host, row, e).await,
    };
    row.verified = true;
    row.repeat = already_held;
    row.expired = v.expired;
    row.version_count = Some(version_count);
    row.data = Some(disclosures::plain_view(
        &serde_json::to_value(payload).unwrap_or(Value::Null),
        &effective_disclosures,
    ));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}
