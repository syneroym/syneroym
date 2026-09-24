//! Inbound filing for booking progress, payment requests/acknowledgements, and
//! fulfilment receipts.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    booking::{self, BookingEvent, Track},
    fulfilment, payment,
    record::Envelope,
    transaction::{self, PairState, ReceiptHalf, Role, pair_state},
    verdict::RecordVerdict,
};

use super::{
    super::{
        AGREEMENTS, AgreementRow, CARDS, CardRow, FULFILMENTS, FulfilmentsRow, PAYMENTS, PROGRESS,
        PaymentsRow, ProgressRow, QUOTE_HISTORY, booking_ops, get_bytes, get_row, put_row,
    },
    FileCardResult, refuse_card,
};

pub(crate) async fn defer_or_refuse<H: AppHost>(
    host: &H,
    msg_id: &str,
    row: CardRow,
    envelope_json: &str,
    now: u64,
    reason: &str,
) -> Result<FileCardResult, String> {
    let issued_at = Envelope::from_json(envelope_json).map(|e| e.issued_at_secs).unwrap_or(0);
    if issued_at + booking::MAX_DEFER_SECS > now {
        Ok(FileCardResult {
            filed: false,
            refused: false,
            unknown: false,
            countersigned: false,
            deferred: true,
        })
    } else {
        refuse_card(host, msg_id, row, reason).await
    }
}

pub(crate) async fn file_progress_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = booking::verify_booking_progress(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "booking progress does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    if p.conversation != conversation {
        return refuse_card(host, msg_id, row, "card names another conversation").await;
    }
    if owner != p.consumer_did {
        return refuse_card(
            host,
            msg_id,
            row,
            "progress for an agreement this node is not the customer of",
        )
        .await;
    }
    let quote_bytes = match get_bytes(host, QUOTE_HISTORY, &p.agreement).await? {
        Some(b) => b,
        None => {
            return defer_or_refuse(
                host,
                msg_id,
                row,
                envelope,
                now,
                "names a quote this node does not hold",
            )
            .await;
        }
    };
    let quote_env = String::from_utf8_lossy(&quote_bytes);
    let qv = transaction::verify_quote(&quote_env, now);
    if !qv.verified {
        return refuse_card(host, msg_id, row, "stored quote does not verify").await;
    }
    if v.issuer != qv.signer_did {
        return refuse_card(host, msg_id, row, "not signed by the service that signed the quote")
            .await;
    }
    let qp = match qv.payload.as_ref() {
        Some(q) => q,
        None => return refuse_card(host, msg_id, row, "stored quote has no payload").await,
    };
    if p.provider_did != qv.issuer.unwrap_or_default() || p.consumer_did != qp.consumer_did {
        return refuse_card(host, msg_id, row, "names the wrong parties").await;
    }

    let stored: Option<ProgressRow> = get_row(host, PROGRESS, &p.agreement).await?;
    let should_store = stored.map(|s| p.seq > s.seq).unwrap_or(true);
    if should_store {
        let prog_row = ProgressRow {
            agreement: p.agreement.clone(),
            conversation: conversation.to_string(),
            seq: p.seq,
            snapshot: p.clone(),
            envelope: envelope.to_string(),
            record_id: v.record_id.clone().unwrap_or_default(),
            writer: v.issuer.unwrap_or_default(),
            received_at_secs: now,
        };
        put_row(host, PROGRESS, &p.agreement, &prog_row).await?;
    }

    row.known = true;
    row.verified = true;
    row.record_id = v.record_id;
    row.data = serde_json::to_value(p).ok();
    put_row(host, CARDS, msg_id, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

pub(crate) async fn file_payment_request_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    _owner: &str,
) -> Result<FileCardResult, String> {
    let v = payment::verify_payment_request(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "payment request does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let agr = match agr {
        Some(a) => a,
        None => {
            return defer_or_refuse(
                host,
                msg_id,
                row,
                envelope,
                now,
                "names an agreement this node does not hold",
            )
            .await;
        }
    };
    let pair = pair_state(agr.consumer.as_ref(), agr.provider.as_ref());
    if pair != PairState::Complete {
        return defer_or_refuse(
            host,
            msg_id,
            row,
            envelope,
            now,
            "agreement not complete on this node",
        )
        .await;
    }
    if p.provider_did != agr.provider_did || p.consumer_did != agr.consumer_did {
        return refuse_card(host, msg_id, row, "names the wrong parties").await;
    }
    if !payment::matches_terms(&p.currency, p.amount_minor, None, &agr.terms) {
        return refuse_card(host, msg_id, row, "amount-mismatch").await;
    }

    let mut payments: PaymentsRow =
        get_row(host, PAYMENTS, &p.agreement).await?.unwrap_or_else(|| PaymentsRow {
            agreement: p.agreement.clone(),
            conversation: conversation.to_string(),
            request: None,
            consumer: Vec::new(),
            provider: Vec::new(),
            updated_at_secs: now,
        });

    let rec_id = v.record_id.clone().unwrap_or_default();
    if let Some(existing) = &payments.request {
        if existing.record_id != rec_id {
            row.known = true;
            row.verified = true;
            row.record_id = Some(rec_id);
            row.reason = Some("a second payment request for one agreement".to_string());
            row.data = serde_json::to_value(p).ok();
            put_row(host, CARDS, msg_id, &row).await?;
            return Ok(FileCardResult {
                filed: true,
                refused: false,
                unknown: false,
                countersigned: false,
                deferred: false,
            });
        }
    } else {
        payments.request = Some(ReceiptHalf {
            record_id: rec_id.clone(),
            envelope: envelope.to_string(),
            issuer: v.issuer.clone().unwrap_or_default(),
            issued_at_secs: v.issued_at_secs.unwrap_or(now),
        });
        payments.updated_at_secs = now;
        put_row(host, PAYMENTS, &p.agreement, &payments).await?;
    }

    row.known = true;
    row.verified = true;
    row.record_id = Some(rec_id);
    row.data = serde_json::to_value(p).ok();
    put_row(host, CARDS, msg_id, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

pub(crate) async fn file_payment_ack_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = payment::verify_payment_acknowledgement(envelope, now);
    if !v.verified {
        let reason =
            v.reason.unwrap_or_else(|| "payment acknowledgement does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let agr = match validate_ack_agreement(agr, p) {
        Ok(a) => a,
        Err((reason, true)) => {
            return defer_or_refuse(host, msg_id, row, envelope, now, reason).await;
        }
        Err((reason, false)) => return refuse_card(host, msg_id, row, reason).await,
    };

    let mut payments: PaymentsRow =
        get_row(host, PAYMENTS, &p.agreement).await?.unwrap_or_else(|| PaymentsRow {
            agreement: p.agreement.clone(),
            conversation: conversation.to_string(),
            request: None,
            consumer: Vec::new(),
            provider: Vec::new(),
            updated_at_secs: now,
        });

    let env = match Envelope::from_json(envelope) {
        Ok(e) => e,
        Err(e) => return refuse_card(host, msg_id, row, e.to_string()).await,
    };

    let versions = match p.role {
        Role::Consumer => &mut payments.consumer,
        Role::Provider => &mut payments.provider,
    };

    let first = match append_ack_half(versions, env.supersedes.as_deref(), envelope, &v, p, now) {
        AckAppendResult::First => true,
        AckAppendResult::Subsequent => false,
        AckAppendResult::Refuse(reason) => return refuse_card(host, msg_id, row, reason).await,
        AckAppendResult::Defer(reason) => {
            return defer_or_refuse(host, msg_id, row, envelope, now, reason).await;
        }
    };

    payments.updated_at_secs = now;
    put_row(host, PAYMENTS, &p.agreement, &payments).await?;

    row.known = true;
    row.verified = true;
    row.record_id = v.record_id.clone();
    row.data = serde_json::to_value(p).ok();
    put_row(host, CARDS, msg_id, &row).await?;

    if first && owner == agr.provider_did && p.role == Role::Consumer {
        let _ = booking_ops::transition(
            host,
            &p.agreement,
            BookingEvent::Half { track: Track::Payment, role: Role::Consumer },
            now,
        )
        .await;
    }

    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

pub(crate) async fn file_fulfilment_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = fulfilment::verify_fulfilment_receipt(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "fulfilment receipt does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let agr = match agr {
        Some(a) => a,
        None => {
            return defer_or_refuse(
                host,
                msg_id,
                row,
                envelope,
                now,
                "names an agreement this node does not hold",
            )
            .await;
        }
    };
    let pair = pair_state(agr.consumer.as_ref(), agr.provider.as_ref());
    if pair != PairState::Complete {
        return defer_or_refuse(
            host,
            msg_id,
            row,
            envelope,
            now,
            "agreement not complete on this node",
        )
        .await;
    }
    if p.provider_did != agr.provider_did || p.consumer_did != agr.consumer_did {
        return refuse_card(host, msg_id, row, "names the wrong parties").await;
    }
    if p.terms != agr.terms {
        return refuse_card(host, msg_id, row, "terms differ from the agreement").await;
    }

    let mut fulfilments: FulfilmentsRow =
        get_row(host, FULFILMENTS, &p.agreement).await?.unwrap_or_else(|| FulfilmentsRow {
            agreement: p.agreement.clone(),
            conversation: conversation.to_string(),
            consumer: None,
            provider: None,
            updated_at_secs: now,
        });

    let rec_id = v.record_id.clone().unwrap_or_default();
    let existing = match p.role {
        Role::Consumer => &mut fulfilments.consumer,
        Role::Provider => &mut fulfilments.provider,
    };

    let first = if let Some(ex) = existing {
        if ex.record_id == rec_id {
            false
        } else {
            return refuse_card(host, msg_id, row, "a second receipt from this party").await;
        }
    } else {
        *existing = Some(ReceiptHalf {
            record_id: rec_id.clone(),
            envelope: envelope.to_string(),
            issuer: v.issuer.clone().unwrap_or_default(),
            issued_at_secs: v.issued_at_secs.unwrap_or(now),
        });
        fulfilments.updated_at_secs = now;
        put_row(host, FULFILMENTS, &p.agreement, &fulfilments).await?;
        true
    };

    row.known = true;
    row.verified = true;
    row.record_id = Some(rec_id);
    row.data = serde_json::to_value(p).ok();
    put_row(host, CARDS, msg_id, &row).await?;

    if first && owner == agr.provider_did && p.role == Role::Consumer {
        let _ = booking_ops::transition(
            host,
            &p.agreement,
            BookingEvent::Half { track: Track::Fulfilment, role: Role::Consumer },
            now,
        )
        .await;
    }

    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

fn validate_ack_agreement(
    agr: Option<AgreementRow>,
    p: &payment::PaymentAcknowledgementPayload,
) -> Result<AgreementRow, (&'static str, bool)> {
    let agr = match agr {
        Some(a) => a,
        None => return Err(("names an agreement this node does not hold", true)),
    };
    let pair = pair_state(agr.consumer.as_ref(), agr.provider.as_ref());
    if pair != PairState::Complete {
        return Err(("agreement not complete on this node", true));
    }
    if p.provider_did != agr.provider_did || p.consumer_did != agr.consumer_did {
        return Err(("names the wrong parties", false));
    }
    if !payment::matches_terms(&p.currency, p.amount_minor, p.method.as_deref(), &agr.terms) {
        return Err(("amount-mismatch", false));
    }
    Ok(agr)
}

enum AckAppendResult {
    First,
    Subsequent,
    Refuse(&'static str),
    Defer(&'static str),
}

fn append_ack_half(
    versions: &mut Vec<ReceiptHalf>,
    supersedes: Option<&str>,
    envelope: &str,
    v: &RecordVerdict<payment::PaymentAcknowledgementPayload>,
    p: &payment::PaymentAcknowledgementPayload,
    now: u64,
) -> AckAppendResult {
    let rec_id = v.record_id.as_deref().unwrap_or_default();
    let issuer = v.issuer.as_deref().unwrap_or_default();
    let issued_at = v.issued_at_secs.unwrap_or(now);
    match supersedes {
        None if versions.is_empty() => {
            versions.push(ReceiptHalf {
                record_id: rec_id.to_string(),
                envelope: envelope.to_string(),
                issuer: issuer.to_string(),
                issued_at_secs: issued_at,
            });
            AckAppendResult::First
        }
        None => {
            if versions.first().map(|x| x.record_id.as_str()) == Some(rec_id) {
                AckAppendResult::Subsequent
            } else {
                AckAppendResult::Refuse("a second first version for this party")
            }
        }
        Some(prev) => {
            if versions.last().map(|x| x.record_id.as_str()) == Some(prev) {
                if let Some(last_half) = versions.last() {
                    let prev_v = payment::verify_payment_acknowledgement(&last_half.envelope, now);
                    if let Some(prev_p) = prev_v.payload
                        && !payment::is_valid_correction(&prev_p, p)
                    {
                        return AckAppendResult::Refuse("invalid-correction");
                    }
                }
                versions.push(ReceiptHalf {
                    record_id: rec_id.to_string(),
                    envelope: envelope.to_string(),
                    issuer: issuer.to_string(),
                    issued_at_secs: issued_at,
                });
                AckAppendResult::Subsequent
            } else if versions.iter().any(|x| x.record_id == rec_id) {
                AckAppendResult::Subsequent
            } else {
                AckAppendResult::Defer("corrects a version this node does not hold")
            }
        }
    }
}
