//! Transaction thread and card pagination.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    clock, envelope::Response, method::Reply, paging, progress::AgreementState,
};

use super::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, ENQUIRIES, QUOTES, RecordPointerRow,
    default_list_limit_u64, ensure_collections, is_deal_level_not_formed_reason,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadInput {
    pub conversation: String,
    #[serde(default = "default_list_limit_u64")]
    pub limit: u64,
    #[serde(default)]
    pub cursor: u64,
}

#[derive(Debug, Serialize)]
pub struct ThreadOutput {
    pub cards: Vec<CardRow>,
}

pub(crate) async fn thread<H: AppHost>(host: &H, input: ThreadInput) -> Reply<ThreadOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let now = clock::now_secs();
    let filter = serde_json::json!({ "conversation": input.conversation }).to_string();

    let quote_pointers: Vec<RecordPointerRow> =
        match paging::query_all(host, QUOTES, Some(filter.clone())).await {
            Ok(v) => v,
            Err(e) => return Err(Response::internal_error(e)),
        };
    let quote_map: HashMap<String, RecordPointerRow> =
        quote_pointers.into_iter().map(|p| (p.id.clone(), p)).collect();

    let enquiry_pointers: Vec<RecordPointerRow> =
        match paging::query_all(host, ENQUIRIES, Some(filter.clone())).await {
            Ok(v) => v,
            Err(e) => return Err(Response::internal_error(e)),
        };
    let enquiry_map: HashMap<String, RecordPointerRow> =
        enquiry_pointers.into_iter().map(|p| (p.id.clone(), p)).collect();

    let agreement_rows: Vec<AgreementRow> =
        match paging::query_all(host, AGREEMENTS, Some(filter.clone())).await {
            Ok(v) => v,
            Err(e) => return Err(Response::internal_error(e)),
        };
    let agreement_map: HashMap<String, AgreementRow> =
        agreement_rows.into_iter().map(|a| (a.quote_record_id.clone(), a)).collect();
    let mut agreements_by_quote_id: HashMap<String, Vec<&AgreementRow>> = HashMap::new();
    for agr in agreement_map.values() {
        agreements_by_quote_id.entry(agr.quote_id.clone()).or_default().push(agr);
    }

    let mut rows: Vec<CardRow> = match paging::query_all(host, CARDS, Some(filter.clone())).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    for row in &mut rows {
        if row.card_type == "quote" && row.verified {
            let qid = row.data.as_ref().and_then(|q| q.get("quote_id")).and_then(Value::as_str);
            let pointer = qid.and_then(|id| quote_map.get(id));
            if let Some(ptr) = pointer {
                if ptr.declined_at_secs.is_some() {
                    row.declined = Some(true);
                }
                if row.version_count.is_none() {
                    row.version_count = Some(ptr.version_count);
                }
            }
            if let Some(exp) = row
                .data
                .as_ref()
                .and_then(|q| q.get("terms"))
                .and_then(|t| t.get("quote_expires_at_secs"))
                .and_then(Value::as_u64)
            {
                row.expired = now >= exp;
            }
            let d_state = compute_quote_deal_state(
                row.record_id.as_deref(),
                qid,
                pointer,
                row.expired,
                &agreement_map,
                &agreements_by_quote_id,
            );
            row.deal_state = Some(d_state);
        } else if row.card_type == "enquiry"
            && row.verified
            && let Some(eid) =
                row.data.as_ref().and_then(|r| r.get("enquiry_id")).and_then(Value::as_str)
            && let Some(pointer) = enquiry_map.get(eid)
        {
            if pointer.declined_at_secs.is_some() {
                row.declined = Some(true);
            }
            if row.version_count.is_none() {
                row.version_count = Some(pointer.version_count);
            }
        } else if (row.card_type == "payment-request" || row.card_type == "payment-acknowledgement")
            && row.verified
            && let Some(agr_id) =
                row.data.as_ref().and_then(|p| p.get("agreement")).and_then(Value::as_str)
            && let Some(cid) =
                row.data.as_ref().and_then(|p| p.get("commitment_id")).and_then(Value::as_str)
            && let Some(agr) = agreement_map.get(agr_id)
            && let Some(c) = agr.terms.commitments.iter().find(|c| c.commitment_id == cid)
        {
            row.agreement_payee = c.payee.clone();
            row.agreement_payment_methods = c.payment_methods.clone().unwrap_or_default();
        }
    }

    // Sort by (sender_timestamp_ms, message_id). The author component is
    // absent because a card row's author is its issuer (a person DID), not the
    // conversation author, so mixing them would order transcripts differently.
    rows.sort_by(|a, b| {
        a.sender_timestamp_ms
            .cmp(&b.sender_timestamp_ms)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });

    let cursor = usize::try_from(input.cursor).unwrap_or(usize::MAX);
    let limit = usize::try_from(input.limit).unwrap_or(usize::MAX);
    let page: Vec<CardRow> = rows.into_iter().skip(cursor).take(limit).collect();
    Ok(ThreadOutput { cards: page })
}

fn compute_quote_deal_state(
    record_id: Option<&str>,
    quote_id: Option<&str>,
    pointer: Option<&RecordPointerRow>,
    is_expired: bool,
    agreement_map: &HashMap<String, AgreementRow>,
    agreements_by_quote_id: &HashMap<String, Vec<&AgreementRow>>,
) -> String {
    if pointer.and_then(|p| p.declined_at_secs).is_some() {
        return "declined".to_string();
    }

    let this_agr = record_id.and_then(|rid| agreement_map.get(rid));
    let quote_agrs = quote_id
        .and_then(|qid| agreements_by_quote_id.get(qid))
        .map(|v| v.as_slice())
        .unwrap_or_default();

    for agr in quote_agrs {
        match agr.state {
            Some(AgreementState::Closed) => return "closed".to_string(),
            Some(AgreementState::Cancelled) => return "cancelled".to_string(),
            _ => {}
        }
    }

    if let Some(agr) = this_agr
        && (agr.state == Some(AgreementState::Agreed)
            || (agr.counterparty.is_some() && agr.quote_author.is_some()))
    {
        return "agreed".to_string();
    }
    for agr in quote_agrs {
        if agr.state == Some(AgreementState::Agreed)
            || (agr.counterparty.is_some() && agr.quote_author.is_some())
        {
            return "agreed".to_string();
        }
    }

    for agr in quote_agrs {
        if agr.state == Some(AgreementState::NotFormed)
            && is_deal_level_not_formed_reason(agr.state_reason.as_deref())
        {
            return "not-formed".to_string();
        }
    }

    if let Some(agr) = this_agr
        && agr.state == Some(AgreementState::NotFormed)
    {
        return "not-formed".to_string();
    }

    if let Some(agr) = this_agr
        && agr.counterparty.is_some()
    {
        if is_expired {
            return "expired".to_string();
        }
        return "accepted".to_string();
    }

    if is_expired {
        return "expired".to_string();
    }

    "quoted".to_string()
}

#[cfg(test)]
mod tests {
    use syneroym_roym_core::{
        deal::{DealTerms, LocalText},
        lang::LangTag,
        transaction::ReceiptHalf,
    };

    use super::*;
    use crate::app::{put_row, tests::fixture};

    #[tokio::test]
    async fn agreed_deal_with_late_quote_already_agreed_refusal_stays_agreed() {
        let f = fixture();
        let now = 1000;
        let en = LangTag::parse_canonical("en").expect("lang");
        let terms = DealTerms {
            currency: "EUR".to_string(),
            commitments: vec![],
            home_exchange: None,
            listing_id: None,
            scope: LocalText::plain(en.clone(), "Scope"),
            cancellation_terms: LocalText::plain(en.clone(), "Terms"),
            refund_terms: LocalText::plain(en.clone(), "Refund"),
            dispute_path: LocalText::plain(en, "Dispute"),
            quote_expires_at_secs: now + 3600,
        };
        let half = ReceiptHalf {
            record_id: "rec_half".to_string(),
            envelope: "{}".to_string(),
            issuer: "did:key:alice".to_string(),
            issued_at_secs: now,
        };

        let agr_v1 = AgreementRow {
            quote_record_id: "rec_v1".to_string(),
            quote_id: "q1".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            quote_author_did: f.master_did.clone(),
            terms: terms.clone(),
            agreed_at_secs: Some(now),
            counterparty: Some(half.clone()),
            quote_author: Some(half),
            state: Some(AgreementState::Agreed),
            state_reason: None,
            cancel_reason: None,
            seq: Some(1),
            commitments: vec![],
            updated_at_secs: now,
        };

        let agr_v2 = AgreementRow {
            quote_record_id: "rec_v2".to_string(),
            quote_id: "q1".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            quote_author_did: f.master_did.clone(),
            terms: terms.clone(),
            agreed_at_secs: None,
            counterparty: None,
            quote_author: None,
            state: Some(AgreementState::NotFormed),
            state_reason: Some("quote-already-agreed".to_string()),
            cancel_reason: None,
            seq: Some(1),
            commitments: vec![],
            updated_at_secs: now,
        };

        put_row(&f.host, AGREEMENTS, &agr_v1.quote_record_id, &agr_v1).await.unwrap();
        put_row(&f.host, AGREEMENTS, &agr_v2.quote_record_id, &agr_v2).await.unwrap();

        // is_deal_ended must be false because the deal is agreed
        assert!(!crate::app::is_deal_ended(&f.host, "q1", None, now).await.unwrap());

        let mut agreement_map = HashMap::new();
        agreement_map.insert("rec_v1".to_string(), agr_v1.clone());
        agreement_map.insert("rec_v2".to_string(), agr_v2.clone());

        let mut agreements_by_quote_id = HashMap::new();
        agreements_by_quote_id.insert("q1".to_string(), vec![&agr_v1, &agr_v2]);

        let state_v1 = compute_quote_deal_state(
            Some("rec_v1"),
            Some("q1"),
            None,
            false,
            &agreement_map,
            &agreements_by_quote_id,
        );
        assert_eq!(state_v1, "agreed");

        let state_v2 = compute_quote_deal_state(
            Some("rec_v2"),
            Some("q1"),
            None,
            false,
            &agreement_map,
            &agreements_by_quote_id,
        );
        assert_eq!(state_v2, "agreed");
    }
}
