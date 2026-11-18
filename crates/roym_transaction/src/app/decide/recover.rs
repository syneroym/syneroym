//! Recovery of deciding rows and reservations cleanup.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    deal::ResourceKind,
    method::{
        call_sibling,
        conversation::{ChangesInput, ChangesOutput},
    },
    paging,
    progress::{self as core_progress, AgreementState},
    services, signing,
    transaction::{self, DealRole, ReceiptHalf, SYNC_WINDOW},
};

use super::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CountersignRow, DecisionRow, DecisionState,
    EVENT_PROGRESS, LEDGER, SnapshotRow, countersign_id, decision_id, ensure_snapshot_sent,
    finalize_agreed_deal, get_row, handle_not_formed, put_row, repeat_both, repeat_countersigned,
    reserve_author_holds, snapshot_id,
};
use crate::app::hold::{Reservations, SeatLedger};

pub(crate) async fn release_all_author_holds<H: AppHost, R: Reservations>(
    host: &H,
    row: &AgreementRow,
    reservations: &R,
) {
    for c in &row.terms.commitments {
        if c.giver == DealRole::QuoteAuthor && c.resource_kind == ResourceKind::ServiceInSlot {
            let _ = reservations.release(host, &row.quote_record_id, &c.commitment_id).await;
        }
    }
}

pub(crate) async fn repeat_not_formed<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    mut snap_row: SnapshotRow,
    reservations: &R,
    now: u64,
) -> Result<bool, String> {
    release_all_author_holds(host, row, reservations).await;

    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = snap_row.snapshot.state_reason.clone();
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snap_row.snapshot.commitments.clone();
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let dec_not_formed = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::NotFormed,
        agreement: row.quote_record_id.clone(),
        quote_id: row.quote_id.clone(),
        reason: snap_row.snapshot.state_reason.clone(),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &decision_id(&row.quote_id), &dec_not_formed).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;
    Ok(false)
}

async fn finish_expired_deciding<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    reservations: &R,
    now: u64,
) -> Result<bool, String> {
    let mut snap_row = super::stored_or_new_seq1_snapshot(host, row, "quote-expired", now).await?;

    release_all_author_holds(host, row, reservations).await;

    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = Some("quote-expired".to_string());
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snap_row.snapshot.commitments.clone();
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let dec_not_formed = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::NotFormed,
        agreement: row.quote_record_id.clone(),
        quote_id: row.quote_id.clone(),
        reason: Some("quote-expired".to_string()),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &decision_id(&row.quote_id), &dec_not_formed).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;

    Ok(false)
}

pub(crate) async fn finish_deciding_row_with_reservations<H: AppHost, R: Reservations>(
    host: &H,
    quote_id: &str,
    reservations: R,
    now: u64,
) -> Result<bool, String> {
    let dec_key = decision_id(quote_id);
    let dec = match get_row::<DecisionRow, _>(host, LEDGER, &dec_key).await? {
        Some(d) if d.state == DecisionState::Deciding => d,
        _ => return Ok(false),
    };
    let mut row = match get_row::<AgreementRow, _>(host, AGREEMENTS, &dec.agreement).await? {
        Some(r) => r,
        None => return Ok(false),
    };

    let held = match crate::app::disclosures::load_quote(host, &row.quote_record_id, now).await {
        Ok(h) => h,
        Err(_) => return Ok(false),
    };

    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let mut stored_snap: Option<SnapshotRow> = get_row(host, AGREEMENT_PROGRESS, &snap_key).await?;

    let cs_key = countersign_id(&row.quote_record_id);
    let mut stored_cs: Option<CountersignRow> = get_row(host, LEDGER, &cs_key).await?;

    if stored_cs.is_none() {
        let _ = restore_writer_from_conversation(host, &mut row, now).await;
        stored_cs = get_row(host, LEDGER, &cs_key).await?;
        stored_snap = get_row(host, AGREEMENT_PROGRESS, &snap_key).await?;
    }

    if let Some(snap_row) = stored_snap {
        if snap_row.snapshot.state == AgreementState::NotFormed {
            return repeat_not_formed(host, &mut row, snap_row, &reservations, now).await;
        }
        if let Some(cs) = stored_cs {
            return repeat_both(host, &mut row, snap_row, cs, &held.disclosures, now).await;
        }
    }

    if stored_cs.is_none() && now >= row.terms.quote_expires_at_secs {
        return finish_expired_deciding(host, &mut row, &reservations, now).await;
    }

    if let Some(cs) = stored_cs {
        return repeat_countersigned(host, &mut row, cs, &held.disclosures, now).await;
    }

    let (reserved, failed) = reserve_author_holds(host, &row, &reservations, now).await?;
    if let Some(reason) = failed {
        return handle_not_formed(host, &mut row, &reservations, &reserved, reason, now).await;
    }

    for cid in &reserved {
        if let Err(reason) = reservations.confirm(host, &row.quote_record_id, cid).await {
            return handle_not_formed(host, &mut row, &reservations, &reserved, &reason, now).await;
        }
    }

    finalize_agreed_deal(host, &mut row, &held.signed.terms, &held.disclosures, now).await
}

pub(crate) async fn finish_deciding_row<H: AppHost>(
    host: &H,
    quote_id: &str,
    now: u64,
) -> Result<bool, String> {
    finish_deciding_row_with_reservations(host, quote_id, SeatLedger, now).await
}

pub(crate) async fn recover_deciding_rows<H: AppHost>(host: &H, now: u64) -> Result<(), String> {
    let filter = serde_json::json!({
        "kind": "decision",
        "state": DecisionState::Deciding.as_str(),
    })
    .to_string();
    let deciding_rows: Vec<DecisionRow> = paging::query_all(host, LEDGER, Some(filter)).await?;
    for dec in deciding_rows {
        let _ = finish_deciding_row(host, &dec.quote_id, now).await;
    }
    Ok(())
}

async fn apply_restored_card<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    owner: &str,
    cs_key: &str,
    msg_id: &str,
    body: &str,
    now: u64,
) -> Result<(), String> {
    let Ok(card) = card::parse_card(body) else { return Ok(()) };

    if card.card_type == "agreement-receipt" {
        let v = transaction::verify_agreement_receipt(&card.envelope, now);
        if v.verified
            && let Some(p) = v.payload
            && p.quote_record_id == row.quote_record_id
            && p.role == DealRole::QuoteAuthor
            && owner == p.quote_author_did
        {
            let half = ReceiptHalf {
                envelope: card.envelope.clone(),
                record_id: v.record_id.clone().unwrap_or_default(),
                issuer: v.issuer.clone().unwrap_or_default(),
                issued_at_secs: v.issued_at_secs.unwrap_or(now),
            };
            let cs_row = CountersignRow {
                kind: "countersign".to_string(),
                agreement: row.quote_record_id.clone(),
                half: half.clone(),
                message_id: Some(msg_id.to_string()),
                created_at_secs: half.issued_at_secs,
            };
            put_row(host, LEDGER, cs_key, &cs_row).await?;
            if row.quote_author.is_none() {
                row.quote_author = Some(half);
                row.agreed_at_secs = Some(cs_row.created_at_secs);
                put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;
            }
        }
    } else if card.card_type == "agreement-progress" {
        let v = core_progress::verify_agreement_progress(&card.envelope, now);
        if v.verified
            && let Some(p) = v.payload
            && p.agreement == row.quote_record_id
            && owner == p.quote_author_did
        {
            let snap_key = snapshot_id(&row.quote_record_id, p.seq);
            if get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &snap_key).await?.is_none() {
                let snap_row = SnapshotRow {
                    seq: p.seq,
                    agreement: row.quote_record_id.clone(),
                    conversation: row.conversation.clone(),
                    snapshot: p.clone(),
                    envelope: card.envelope.clone(),
                    record_id: v.record_id.clone().unwrap_or_default(),
                    message_id: Some(msg_id.to_string()),
                    stored_at_secs: now,
                };
                put_row(host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await?;
            }
            if row.seq.unwrap_or(0) < p.seq {
                row.seq = Some(p.seq);
                row.state = Some(p.state);
                row.state_reason = p.state_reason.clone();
                row.cancel_reason = p.cancel_reason;
                row.commitments = p.commitments.clone();
                row.updated_at_secs = now;
                put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;
            }
        }
    }
    Ok(())
}

async fn restore_writer_from_conversation<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    now: u64,
) -> Result<(), String> {
    let owner = signing::owner_did(host).await.unwrap_or_default();
    let cs_key = countersign_id(&row.quote_record_id);
    let mut after_seq = 0u64;

    loop {
        let out = match call_sibling::<_, ChangesInput, ChangesOutput>(
            host,
            services::CONVERSATION,
            "conversation.changes",
            &ChangesInput {
                conversation: row.conversation.clone(),
                after_seq: Some(after_seq),
                limit: Some(SYNC_WINDOW),
            },
        )
        .await
        {
            Ok(o) => o,
            Err(_) => return Ok(()),
        };

        if out.messages.is_empty() {
            break;
        }

        let mut max_seq = after_seq;
        for m in out.messages {
            max_seq = max_seq.max(m.visible_seq);
            if m.content_type == CARD_CONTENT_TYPE
                && let Some(ref body) = m.body
            {
                apply_restored_card(host, row, &owner, &cs_key, &m.id, body, now).await?;
            }
        }

        if max_seq <= after_seq || max_seq >= out.last_seq {
            break;
        }
        after_seq = max_seq;
    }
    Ok(())
}
