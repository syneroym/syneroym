//! Inbound filing for payment requests/acknowledgements and fulfilment
//! receipts.

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::{CommitmentKind, SettlementRole},
    disclosure::{FULFILMENT_SET, PAYMENT_ACK_SET, PAYMENT_REQUEST_SET, reveal_hidden_strict},
    fulfilment::{self, FulfilmentHidden},
    payment::{self, PaymentAckHidden, PaymentRequestHidden},
    record::{
        Envelope, RECORD_FULFILMENT_RECEIPT, RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST,
    },
    transaction::{DealRole, PairState, ReceiptHalf, pair_state},
    verdict::RecordVerdict,
};

use super::{
    super::{
        AGREEMENTS, AgreementRow, CardRow, FULFILMENTS, FulfilmentsRow, PAYMENTS, PaymentsRow,
        disclosures, fulfilment_ops::fulfilment_id, get_row, payment_ops::payment_id, progress_ops,
        put_card, put_row,
    },
    FileCardResult,
    receipts::defer_or_refuse,
    refuse_card,
};

fn validate_payment_request_agr(
    agr: Option<AgreementRow>,
    p: &payment::PaymentRequestPayload,
    conversation: &str,
    issuer: Option<&str>,
) -> Result<AgreementRow, (&'static str, bool)> {
    let agr = match agr {
        Some(a) => a,
        None => return Err(("names an agreement this node does not hold", true)),
    };
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(("agreement not complete on this node", true));
    }
    if p.quote_author_did != agr.quote_author_did || p.counterparty_did != agr.counterparty_did {
        return Err(("names the wrong parties", false));
    }
    if p.conversation != conversation || p.conversation != agr.conversation {
        return Err(("card names another conversation", false));
    }
    let commitment = match agr.terms.commitments.iter().find(|c| c.commitment_id == p.commitment_id)
    {
        Some(c) => c,
        None => return Err(("no-such-commitment", false)),
    };
    if commitment.kind != CommitmentKind::Currency {
        return Err(("not-currency-commitment", false));
    }
    let receiver_did = if commitment.giver == DealRole::QuoteAuthor {
        &agr.counterparty_did
    } else {
        &agr.quote_author_did
    };
    if issuer != Some(receiver_did) {
        return Err(("issuer is not the receiver of this commitment", false));
    }
    Ok(agr)
}

pub(crate) async fn file_payment_request_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = payment::verify_payment_request(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "payment request does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    let hidden: PaymentRequestHidden =
        match reveal_hidden_strict(&p.sd, &PAYMENT_REQUEST_SET, disclosures) {
            Ok(h) => h,
            Err(e) => {
                return refuse_card(
                    host,
                    row,
                    format!("payment request disclosures: {}", e.code()),
                )
                .await;
            }
        };
    if let Err(e) = hidden.validate() {
        return refuse_card(host, row, format!("payment request hidden: {}", e.code())).await;
    }
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let agr = match validate_payment_request_agr(agr, p, conversation, v.issuer.as_deref()) {
        Ok(a) => a,
        Err((reason, true)) => return defer_or_refuse(host, row, envelope, now, reason).await,
        Err((reason, false)) => return refuse_card(host, row, reason).await,
    };

    let p_id = payment_id(&p.agreement, &p.commitment_id);
    let payments: PaymentsRow =
        get_row(host, PAYMENTS, &p_id).await?.unwrap_or_else(|| PaymentsRow {
            agreement: p.agreement.clone(),
            commitment_id: p.commitment_id.clone(),
            conversation: conversation.to_string(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        });

    let rec_id = v.record_id.clone().unwrap_or_default();
    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
    let plain_data = disclosures::plain_view(&p_val, disclosures);

    if let Some(existing) = &payments.request {
        if existing.record_id != rec_id {
            row.known = true;
            row.verified = true;
            row.record_id = Some(rec_id);
            row.reason = Some("a second payment request for one commitment".to_string());
            row.data = Some(plain_data);
            put_card(host, &row).await?;
            return Ok(FileCardResult {
                filed: true,
                refused: false,
                unknown: false,
                countersigned: false,
                deferred: false,
            });
        }
        row.repeat = true;
    } else {
        disclosures::store(host, &rec_id, RECORD_PAYMENT_REQUEST, disclosures, now).await?;
        let args = RecordPaymentRequestHalfArgs {
            p_id: &p_id,
            payments,
            rec_id: &rec_id,
            envelope,
            issuer: v.issuer.as_deref().unwrap_or_default(),
            issued_at: v.issued_at_secs.unwrap_or(now),
            commitment_id: &p.commitment_id,
            agreement: &p.agreement,
            owner,
            quote_author_did: &agr.quote_author_did,
            now,
        };
        record_payment_request_half(host, args).await?;
    }

    row.known = true;
    row.verified = true;
    row.record_id = Some(rec_id);
    row.data = Some(plain_data);
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

struct RecordPaymentRequestHalfArgs<'a> {
    p_id: &'a str,
    payments: PaymentsRow,
    rec_id: &'a str,
    envelope: &'a str,
    issuer: &'a str,
    issued_at: u64,
    commitment_id: &'a str,
    agreement: &'a str,
    owner: &'a str,
    quote_author_did: &'a str,
    now: u64,
}

async fn record_payment_request_half<H: AppHost>(
    host: &H,
    args: RecordPaymentRequestHalfArgs<'_>,
) -> Result<(), String> {
    let mut payments = args.payments;
    payments.request = Some(ReceiptHalf {
        record_id: args.rec_id.to_string(),
        envelope: args.envelope.to_string(),
        issuer: args.issuer.to_string(),
        issued_at_secs: args.issued_at,
    });
    payments.updated_at_secs = args.now;
    put_row(host, PAYMENTS, args.p_id, &payments).await?;

    if args.owner == args.quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            args.agreement,
            syneroym_roym_core::progress::AgreementEvent::PaymentRequest {
                commitment_id: args.commitment_id.to_string(),
            },
            args.now,
        )
        .await;
    }
    Ok(())
}

fn decode_ack_payload<'a>(
    v: &'a RecordVerdict<payment::PaymentAcknowledgementPayload>,
    disclosures: &[String],
    conversation: &str,
) -> Result<(&'a payment::PaymentAcknowledgementPayload, PaymentAckHidden), String> {
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return Err("missing payload".to_string()),
    };
    let hidden: PaymentAckHidden = match reveal_hidden_strict(&p.sd, &PAYMENT_ACK_SET, disclosures)
    {
        Ok(h) => h,
        Err(e) => return Err(format!("payment acknowledgement disclosures: {}", e.code())),
    };
    if let Err(e) = hidden.validate() {
        return Err(format!("payment acknowledgement hidden: {}", e.code()));
    }
    if p.conversation != conversation {
        return Err("card names another conversation".to_string());
    }
    Ok((p, hidden))
}

async fn finish_ack_card_put<H: AppHost>(
    host: &H,
    mut row: CardRow,
    record_id: Option<String>,
    is_repeat: bool,
    plain_data: Value,
) -> Result<(), String> {
    row.known = true;
    row.verified = true;
    row.repeat = is_repeat;
    row.record_id = record_id;
    row.data = Some(plain_data);
    put_card(host, &row).await
}

pub(crate) async fn file_payment_ack_card<H: AppHost>(
    host: &H,
    row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = payment::verify_payment_acknowledgement(envelope, now);
    if !v.verified {
        let reason =
            v.reason.unwrap_or_else(|| "payment acknowledgement does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let (p, _) = match decode_ack_payload(&v, disclosures, conversation) {
        Ok(res) => res,
        Err(reason) => return refuse_card(host, row, reason).await,
    };
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let agr = match validate_ack_agreement(agr, p) {
        Ok(a) => a,
        Err((reason, true)) => {
            return defer_or_refuse(host, row, envelope, now, reason).await;
        }
        Err((reason, false)) => return refuse_card(host, row, reason).await,
    };

    let deal_role = match validate_ack_roles(&agr, &p.commitment_id, v.issuer.as_deref(), p.role) {
        Ok(r) => r,
        Err(reason) => return refuse_card(host, row, reason).await,
    };

    let p_id = payment_id(&p.agreement, &p.commitment_id);
    let mut payments: PaymentsRow =
        get_row(host, PAYMENTS, &p_id).await?.unwrap_or_else(|| PaymentsRow {
            agreement: p.agreement.clone(),
            commitment_id: p.commitment_id.clone(),
            conversation: conversation.to_string(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        });

    let env = match Envelope::from_json(envelope) {
        Ok(e) => e,
        Err(e) => return refuse_card(host, row, e.to_string()).await,
    };

    let versions = match deal_role {
        DealRole::Counterparty => &mut payments.counterparty,
        DealRole::QuoteAuthor => &mut payments.quote_author,
    };

    let is_repeat = versions.iter().any(|x| Some(x.record_id.as_str()) == v.record_id.as_deref());

    let _first = match append_ack_half(versions, env.supersedes.as_deref(), envelope, &v, p, now) {
        AckAppendResult::First => true,
        AckAppendResult::Subsequent => false,
        AckAppendResult::Refuse(reason) => return refuse_card(host, row, reason).await,
        AckAppendResult::Defer(reason) => {
            return defer_or_refuse(host, row, envelope, now, reason).await;
        }
    };

    if let Some(rec_id) = v.record_id.as_deref() {
        disclosures::store(host, rec_id, RECORD_PAYMENT_ACKNOWLEDGEMENT, disclosures, now).await?;
    }

    payments.updated_at_secs = now;
    put_row(host, PAYMENTS, &p_id, &payments).await?;

    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
    let plain_data = disclosures::plain_view(&p_val, disclosures);
    finish_ack_card_put(host, row, v.record_id.clone(), is_repeat, plain_data).await?;

    if owner == agr.quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            &p.agreement,
            syneroym_roym_core::progress::AgreementEvent::Half {
                commitment_id: p.commitment_id.clone(),
                role: p.role,
                record_id: v.record_id.clone().unwrap_or_default(),
            },
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

fn validate_fulfilment_card_agr(
    agr: Option<AgreementRow>,
    p: &fulfilment::FulfilmentReceiptPayload,
    conversation: &str,
    issuer: Option<&str>,
) -> Result<(AgreementRow, DealRole), (&'static str, bool)> {
    let agr = match agr {
        Some(a) => a,
        None => return Err(("names an agreement this node does not hold", true)),
    };
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(("agreement not complete on this node", true));
    }
    if p.quote_author_did != agr.quote_author_did || p.counterparty_did != agr.counterparty_did {
        return Err(("names the wrong parties", false));
    }
    if p.conversation != conversation || p.conversation != agr.conversation {
        return Err(("card names another conversation", false));
    }
    let commitment = match agr.terms.commitments.iter().find(|c| c.commitment_id == p.commitment_id)
    {
        Some(c) => c,
        None => return Err(("no-such-commitment", false)),
    };
    if commitment.kind != CommitmentKind::Fulfilment {
        return Err(("not-fulfilment-commitment", false));
    }
    let deal_role = if issuer == Some(&agr.quote_author_did) {
        DealRole::QuoteAuthor
    } else if issuer == Some(&agr.counterparty_did) {
        DealRole::Counterparty
    } else {
        return Err(("not-a-party", false));
    };
    let expected_role = if deal_role == commitment.giver {
        SettlementRole::Giver
    } else {
        SettlementRole::Receiver
    };
    if p.role != expected_role {
        return Err(("role-mismatch", false));
    }
    Ok((agr, deal_role))
}

pub(crate) async fn file_fulfilment_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = fulfilment::verify_fulfilment_receipt(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "fulfilment receipt does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    let hidden: FulfilmentHidden = match reveal_hidden_strict(&p.sd, &FULFILMENT_SET, disclosures) {
        Ok(h) => h,
        Err(e) => {
            return refuse_card(host, row, format!("fulfilment receipt disclosures: {}", e.code()))
                .await;
        }
    };
    if let Err(e) = hidden.validate() {
        return refuse_card(host, row, format!("fulfilment receipt hidden: {}", e.code())).await;
    }
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let (agr, deal_role) =
        match validate_fulfilment_card_agr(agr, p, conversation, v.issuer.as_deref()) {
            Ok(res) => res,
            Err((reason, true)) => return defer_or_refuse(host, row, envelope, now, reason).await,
            Err((reason, false)) => return refuse_card(host, row, reason).await,
        };

    let f_id = fulfilment_id(&p.agreement, &p.commitment_id);
    let fulfilments: FulfilmentsRow =
        get_row(host, FULFILMENTS, &f_id).await?.unwrap_or_else(|| FulfilmentsRow {
            agreement: p.agreement.clone(),
            commitment_id: p.commitment_id.clone(),
            conversation: conversation.to_string(),
            counterparty: None,
            quote_author: None,
            updated_at_secs: now,
        });

    let rec_id = v.record_id.clone().unwrap_or_default();
    let existing = match deal_role {
        DealRole::Counterparty => fulfilments.counterparty.as_ref(),
        DealRole::QuoteAuthor => fulfilments.quote_author.as_ref(),
    };

    if let Some(ex) = existing {
        if ex.record_id == rec_id {
            row.repeat = true;
        } else {
            return refuse_card(host, row, "a second receipt from this party").await;
        }
    } else {
        disclosures::store(host, &rec_id, RECORD_FULFILMENT_RECEIPT, disclosures, now).await?;
        let args = RecordFulfilmentHalfArgs {
            f_id: &f_id,
            fulfilments,
            deal_role,
            rec_id: &rec_id,
            envelope,
            issuer: v.issuer.as_deref().unwrap_or_default(),
            issued_at: v.issued_at_secs.unwrap_or(now),
            commitment_id: &p.commitment_id,
            agreement: &p.agreement,
            role: p.role,
            owner,
            quote_author_did: &agr.quote_author_did,
            now,
        };
        record_fulfilment_half_card(host, args).await?;
    }

    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
    let plain_data = disclosures::plain_view(&p_val, disclosures);

    row.known = true;
    row.verified = true;
    row.record_id = Some(rec_id);
    row.data = Some(plain_data);
    put_card(host, &row).await?;

    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

struct RecordFulfilmentHalfArgs<'a> {
    f_id: &'a str,
    fulfilments: FulfilmentsRow,
    deal_role: DealRole,
    rec_id: &'a str,
    envelope: &'a str,
    issuer: &'a str,
    issued_at: u64,
    commitment_id: &'a str,
    agreement: &'a str,
    role: SettlementRole,
    owner: &'a str,
    quote_author_did: &'a str,
    now: u64,
}

async fn record_fulfilment_half_card<H: AppHost>(
    host: &H,
    args: RecordFulfilmentHalfArgs<'_>,
) -> Result<(), String> {
    let mut fulfilments = args.fulfilments;
    let target = match args.deal_role {
        DealRole::Counterparty => &mut fulfilments.counterparty,
        DealRole::QuoteAuthor => &mut fulfilments.quote_author,
    };

    *target = Some(ReceiptHalf {
        record_id: args.rec_id.to_string(),
        envelope: args.envelope.to_string(),
        issuer: args.issuer.to_string(),
        issued_at_secs: args.issued_at,
    });
    fulfilments.updated_at_secs = args.now;
    put_row(host, FULFILMENTS, args.f_id, &fulfilments).await?;

    if args.owner == args.quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            args.agreement,
            syneroym_roym_core::progress::AgreementEvent::Half {
                commitment_id: args.commitment_id.to_string(),
                role: args.role,
                record_id: args.rec_id.to_string(),
            },
            args.now,
        )
        .await;
    }

    Ok(())
}

fn validate_ack_agreement(
    agr: Option<AgreementRow>,
    p: &payment::PaymentAcknowledgementPayload,
) -> Result<AgreementRow, (&'static str, bool)> {
    let agr = match agr {
        Some(a) => a,
        None => return Err(("names an agreement this node does not hold", true)),
    };
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(("agreement not complete on this node", true));
    }
    if p.quote_author_did != agr.quote_author_did || p.counterparty_did != agr.counterparty_did {
        return Err(("names the wrong parties", false));
    }
    if p.conversation != agr.conversation {
        return Err(("card names another conversation", false));
    }
    Ok(agr)
}

fn validate_ack_roles(
    agr: &AgreementRow,
    commitment_id: &str,
    issuer: Option<&str>,
    p_role: SettlementRole,
) -> Result<DealRole, &'static str> {
    let commitment = match agr.terms.commitments.iter().find(|c| c.commitment_id == commitment_id) {
        Some(c) => c,
        None => return Err("no-such-commitment"),
    };
    if commitment.kind != CommitmentKind::Currency {
        return Err("not-currency-commitment");
    }

    let deal_role = if issuer == Some(&agr.quote_author_did) {
        DealRole::QuoteAuthor
    } else if issuer == Some(&agr.counterparty_did) {
        DealRole::Counterparty
    } else {
        return Err("not-a-party");
    };

    let expected_settlement_role = if deal_role == commitment.giver {
        SettlementRole::Giver
    } else {
        SettlementRole::Receiver
    };
    if p_role != expected_settlement_role {
        return Err("role-mismatch");
    }
    Ok(deal_role)
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
            } else if versions.iter().any(|x| x.record_id == prev) {
                AckAppendResult::Refuse("superseded version was already corrected")
            } else {
                AckAppendResult::Defer("corrects a version this node does not hold")
            }
        }
    }
}
