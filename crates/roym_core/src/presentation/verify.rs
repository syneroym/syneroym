//! Verification logic for presentation bundles shown to third parties.

use serde_json::{Map, Value};
use syneroym_signed_record::Envelope;

use super::{
    types::{
        CommitmentOutcome, DealHeader, MAX_PRESENTATION_RECORDS, PRESENTATION_VERSION,
        Presentation, PresentationError, PresentationView, RecordCheck, SettlementView,
        WriterCheck,
    },
    validate::validate_partial_terms_fields,
};
use crate::{
    deal::{
        CommitmentKind, SettlementRole,
        sd::{SignedCommitment, SignedTerms},
    },
    disclosure::{
        DisclosureIndex, FULFILMENT_SET, HiddenSet, PAYMENT_ACK_SET, PAYMENT_REQUEST_SET,
        check_all_used, reveal_object,
    },
    fulfilment::{self, FulfilmentHidden, FulfilmentReceiptPayload},
    payment::{
        self, PaymentAckHidden, PaymentAcknowledgementPayload, PaymentRequestHidden,
        PaymentRequestPayload,
    },
    progress::{self, AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload},
    record::{
        RECORD_AGREEMENT_PROGRESS, RECORD_AGREEMENT_RECEIPT, RECORD_FULFILMENT_RECEIPT,
        RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST, RECORD_QUOTE,
    },
    transaction::{
        self, AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DealRole, QUOTE_VERSION,
        QuotePayload, RecordVerdict,
    },
};

enum VerifiedRecord {
    Quote(RecordVerdict<QuotePayload>, QuotePayload),
    Receipt(RecordVerdict<AgreementReceiptPayload>, AgreementReceiptPayload),
    Progress(RecordVerdict<AgreementProgressPayload>, AgreementProgressPayload),
    PaymentRequest(RecordVerdict<PaymentRequestPayload>, PaymentRequestPayload),
    PaymentAck(RecordVerdict<PaymentAcknowledgementPayload>, PaymentAcknowledgementPayload),
    Fulfilment(RecordVerdict<FulfilmentReceiptPayload>, FulfilmentReceiptPayload),
}

impl VerifiedRecord {
    fn record_check(&self) -> RecordCheck {
        match self {
            Self::Quote(v, _) => check_from_verdict(RECORD_QUOTE, v),
            Self::Receipt(v, _) => check_from_verdict(RECORD_AGREEMENT_RECEIPT, v),
            Self::Progress(v, _) => check_from_verdict(RECORD_AGREEMENT_PROGRESS, v),
            Self::PaymentRequest(v, _) => check_from_verdict(RECORD_PAYMENT_REQUEST, v),
            Self::PaymentAck(v, _) => check_from_verdict(RECORD_PAYMENT_ACKNOWLEDGEMENT, v),
            Self::Fulfilment(v, _) => check_from_verdict(RECORD_FULFILMENT_RECEIPT, v),
        }
    }
}

fn check_from_verdict<P>(record_type: &str, v: &RecordVerdict<P>) -> RecordCheck {
    RecordCheck {
        record_type: record_type.to_string(),
        record_id: v.record_id.clone().unwrap_or_default(),
        issuer: v.issuer.clone().unwrap_or_default(),
        issued_at_secs: v.issued_at_secs.unwrap_or(0),
        verified: v.verified,
    }
}

/// Verifies a presentation from a third party according to selective disclosure
/// rules.
pub fn verify_presentation(
    p: &Presentation,
    now_secs: u64,
) -> Result<PresentationView, PresentationError> {
    if p.presentation_version != PRESENTATION_VERSION {
        return Err(PresentationError::UnsupportedVersion(p.presentation_version));
    }
    if p.records.is_empty() || p.records.len() > MAX_PRESENTATION_RECORDS {
        return Err(PresentationError::RecordCount(p.records.len()));
    }

    let mut verified_records = Vec::with_capacity(p.records.len());
    for raw in &p.records {
        let vr = verify_envelope_record(raw, now_secs)?;
        verified_records.push(vr);
    }

    let deal_records = partition_and_check_deal(&verified_records)?;
    let writer = check_writer_binding(&deal_records);

    let max_disclosures = calculate_max_disclosures(&deal_records);
    let index = DisclosureIndex::build(&p.disclosures, max_disclosures)?;

    let partial_terms = deal_records.signed_terms.reveal(&index)?;
    let mut all_used = partial_terms.used.clone();

    let settlement_views =
        reveal_settlement_views(&deal_records.settlements, &index, &mut all_used)?;
    check_all_used(&index, &all_used)?;

    let mut not_checked = Vec::new();
    validate_partial_terms_fields(
        &partial_terms,
        deal_records.signed_terms,
        deal_records.receipts.as_slice(),
        &mut not_checked,
    )?;

    let commitments = build_commitment_outcomes(
        &deal_records.signed_terms.commitments,
        deal_records.progress.as_ref(),
    );

    let records = verified_records.iter().map(VerifiedRecord::record_check).collect();
    let hidden = partial_terms.hidden.into_iter().map(|p| p.to_string()).collect();

    Ok(PresentationView {
        deal: deal_records.header,
        records,
        terms: partial_terms.value,
        hidden,
        commitments,
        settlement: settlement_views,
        writer,
        not_checked,
    })
}

struct DealRecords<'a> {
    header: DealHeader,
    signed_terms: &'a SignedTerms,
    quote: Option<(&'a RecordVerdict<QuotePayload>, &'a QuotePayload)>,
    receipts: Vec<(&'a RecordVerdict<AgreementReceiptPayload>, &'a AgreementReceiptPayload)>,
    progress: Option<(&'a RecordVerdict<AgreementProgressPayload>, &'a AgreementProgressPayload)>,
    settlements: Vec<&'a VerifiedRecord>,
}

fn verify_envelope_record(raw: &str, now_secs: u64) -> Result<VerifiedRecord, PresentationError> {
    let env =
        Envelope::from_json(raw).map_err(|e| PresentationError::RecordRefused(e.to_string()))?;
    match (env.record_type.as_str(), env.version) {
        (RECORD_QUOTE, QUOTE_VERSION) => {
            let v = transaction::verify_quote(raw, now_secs);
            let payload = extract_payload(v.verified, v.reason.as_deref(), v.payload.clone())?;
            Ok(VerifiedRecord::Quote(v, payload))
        }
        (RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION) => {
            let v = transaction::verify_agreement_receipt(raw, now_secs);
            let payload = extract_payload(v.verified, v.reason.as_deref(), v.payload.clone())?;
            Ok(VerifiedRecord::Receipt(v, payload))
        }
        (RECORD_AGREEMENT_PROGRESS, AGREEMENT_PROGRESS_VERSION) => {
            let v = progress::verify_agreement_progress(raw, now_secs);
            let payload = extract_payload(v.verified, v.reason.as_deref(), v.payload.clone())?;
            Ok(VerifiedRecord::Progress(v, payload))
        }
        (RECORD_PAYMENT_REQUEST, payment::PAYMENT_REQUEST_VERSION) => {
            let v = payment::verify_payment_request(raw, now_secs);
            let payload = extract_payload(v.verified, v.reason.as_deref(), v.payload.clone())?;
            Ok(VerifiedRecord::PaymentRequest(v, payload))
        }
        (RECORD_PAYMENT_ACKNOWLEDGEMENT, payment::PAYMENT_ACKNOWLEDGEMENT_VERSION) => {
            let v = payment::verify_payment_acknowledgement(raw, now_secs);
            let payload = extract_payload(v.verified, v.reason.as_deref(), v.payload.clone())?;
            Ok(VerifiedRecord::PaymentAck(v, payload))
        }
        (RECORD_FULFILMENT_RECEIPT, fulfilment::FULFILMENT_RECEIPT_VERSION) => {
            let v = fulfilment::verify_fulfilment_receipt(raw, now_secs);
            let payload = extract_payload(v.verified, v.reason.as_deref(), v.payload.clone())?;
            Ok(VerifiedRecord::Fulfilment(v, payload))
        }
        (other, _) => Err(PresentationError::UnsupportedRecordType(other.to_string())),
    }
}

fn extract_payload<P>(
    verified: bool,
    reason: Option<&str>,
    payload: Option<P>,
) -> Result<P, PresentationError> {
    if !verified {
        return Err(PresentationError::RecordRefused(
            reason.unwrap_or("record not verified").to_string(),
        ));
    }
    payload.ok_or_else(|| PresentationError::RecordRefused("missing payload".to_string()))
}

fn check_receipt_half<'a>(
    v: &'a RecordVerdict<AgreementReceiptPayload>,
    p: &'a AgreementReceiptPayload,
    quote_author_half: &mut Option<(
        &'a RecordVerdict<AgreementReceiptPayload>,
        &'a AgreementReceiptPayload,
    )>,
    counterparty_half: &mut Option<(
        &'a RecordVerdict<AgreementReceiptPayload>,
        &'a AgreementReceiptPayload,
    )>,
    receipts: &mut Vec<(&'a RecordVerdict<AgreementReceiptPayload>, &'a AgreementReceiptPayload)>,
) -> Result<(), PresentationError> {
    match p.role {
        DealRole::QuoteAuthor => {
            if quote_author_half.is_some() {
                return Err(PresentationError::DuplicateRecord("quote-author-receipt".to_string()));
            }
            *quote_author_half = Some((v, p));
        }
        DealRole::Counterparty => {
            if counterparty_half.is_some() {
                return Err(PresentationError::DuplicateRecord("counterparty-receipt".to_string()));
            }
            *counterparty_half = Some((v, p));
        }
    }
    receipts.push((v, p));
    Ok(())
}

fn partition_and_check_deal<'a>(
    records: &'a [VerifiedRecord],
) -> Result<DealRecords<'a>, PresentationError> {
    let mut quote = None;
    let mut receipts = Vec::new();
    let mut progress_rec = None;
    let mut settlements = Vec::new();

    let mut quote_author_half = None;
    let mut counterparty_half = None;

    for vr in records {
        match vr {
            VerifiedRecord::Quote(v, p) => {
                if quote.is_some() {
                    return Err(PresentationError::DuplicateRecord("quote".to_string()));
                }
                quote = Some((v, p));
            }
            VerifiedRecord::Receipt(v, p) => {
                check_receipt_half(
                    v,
                    p,
                    &mut quote_author_half,
                    &mut counterparty_half,
                    &mut receipts,
                )?;
            }
            VerifiedRecord::Progress(v, p) => {
                if progress_rec.is_some() {
                    return Err(PresentationError::DuplicateRecord(
                        "agreement-progress".to_string(),
                    ));
                }
                progress_rec = Some((v, p));
            }
            VerifiedRecord::PaymentRequest(_, _)
            | VerifiedRecord::PaymentAck(_, _)
            | VerifiedRecord::Fulfilment(_, _) => {
                settlements.push(vr);
            }
        }
    }

    let agreement_id = determine_agreement_id(quote, &receipts, progress_rec, &settlements)?;
    check_all_records_match_agreement(
        agreement_id.as_str(),
        quote,
        &receipts,
        progress_rec,
        &settlements,
    )?;

    let (signed_terms, quote_id, quote_author_did, counterparty_did, quote_checked) =
        extract_deal_terms_and_parties(quote, &receipts)?;

    if let (Some((_, a)), Some((_, b))) = (quote_author_half, counterparty_half)
        && !transaction::halves_agree(a, b)
    {
        return Err(PresentationError::HalvesDisagree);
    }

    check_settlement_and_progress_records(
        &quote_author_did,
        &counterparty_did,
        signed_terms,
        progress_rec,
        &settlements,
    )?;

    let header = DealHeader {
        quote_id,
        agreement: agreement_id,
        quote_author_did,
        counterparty_did,
        currency: signed_terms.currency.clone(),
        quote_expires_at_secs: signed_terms.quote_expires_at_secs,
        quote_checked,
    };

    Ok(DealRecords { header, signed_terms, quote, receipts, progress: progress_rec, settlements })
}

fn determine_agreement_id(
    quote: Option<(&RecordVerdict<QuotePayload>, &QuotePayload)>,
    receipts: &[(&RecordVerdict<AgreementReceiptPayload>, &AgreementReceiptPayload)],
    progress_rec: Option<(&RecordVerdict<AgreementProgressPayload>, &AgreementProgressPayload)>,
    settlements: &[&VerifiedRecord],
) -> Result<String, PresentationError> {
    if let Some((v, _)) = quote {
        return v.record_id.clone().ok_or_else(|| {
            PresentationError::RecordRefused("quote missing record_id".to_string())
        });
    }
    if let Some((_, p)) = receipts.first() {
        return Ok(p.quote_record_id.clone());
    }
    if let Some((_, p)) = progress_rec {
        return Ok(p.agreement.clone());
    }
    if let Some(s) = settlements.first() {
        return match s {
            VerifiedRecord::PaymentRequest(_, p) => Ok(p.agreement.clone()),
            VerifiedRecord::PaymentAck(_, p) => Ok(p.agreement.clone()),
            VerifiedRecord::Fulfilment(_, p) => Ok(p.agreement.clone()),
            _ => Err(PresentationError::MissingTerms),
        };
    }
    Err(PresentationError::MissingTerms)
}

fn check_all_records_match_agreement(
    expected: &str,
    quote: Option<(&RecordVerdict<QuotePayload>, &QuotePayload)>,
    receipts: &[(&RecordVerdict<AgreementReceiptPayload>, &AgreementReceiptPayload)],
    progress_rec: Option<(&RecordVerdict<AgreementProgressPayload>, &AgreementProgressPayload)>,
    settlements: &[&VerifiedRecord],
) -> Result<(), PresentationError> {
    if let Some((v, _)) = quote
        && v.record_id.as_deref() != Some(expected)
    {
        return Err(PresentationError::MultipleDeals(format!(
            "quote id '{}' does not match agreement '{expected}'",
            v.record_id.as_deref().unwrap_or_default()
        )));
    }
    for (_, p) in receipts {
        if p.quote_record_id != expected {
            return Err(PresentationError::MultipleDeals(format!(
                "receipt quote_record_id '{}' does not match agreement '{expected}'",
                p.quote_record_id
            )));
        }
    }
    if let Some((_, p)) = progress_rec
        && p.agreement != expected
    {
        return Err(PresentationError::MultipleDeals(format!(
            "progress agreement '{}' does not match '{expected}'",
            p.agreement
        )));
    }
    for s in settlements {
        let agr = match s {
            VerifiedRecord::PaymentRequest(_, p) => &p.agreement,
            VerifiedRecord::PaymentAck(_, p) => &p.agreement,
            VerifiedRecord::Fulfilment(_, p) => &p.agreement,
            _ => expected,
        };
        if agr != expected {
            return Err(PresentationError::MultipleDeals(format!(
                "settlement agreement '{agr}' does not match '{expected}'"
            )));
        }
    }
    Ok(())
}

fn extract_deal_terms_and_parties<'a>(
    quote: Option<(&'a RecordVerdict<QuotePayload>, &'a QuotePayload)>,
    receipts: &[(&'a RecordVerdict<AgreementReceiptPayload>, &'a AgreementReceiptPayload)],
) -> Result<(&'a SignedTerms, Option<String>, String, String, bool), PresentationError> {
    match (quote, receipts.first()) {
        (Some((q_v, q_p)), Some((_, r_p))) => {
            if q_p.terms != r_p.terms {
                return Err(PresentationError::TermsMismatch);
            }
            let author_did = q_v.issuer.clone().ok_or_else(|| {
                PresentationError::RecordRefused("missing quote issuer".to_string())
            })?;
            for (_, p) in receipts {
                if p.quote_author_did != author_did || p.counterparty_did != q_p.counterparty_did {
                    return Err(PresentationError::PartyMismatch);
                }
            }
            Ok((
                &q_p.terms,
                Some(q_p.quote_id.clone()),
                author_did,
                q_p.counterparty_did.clone(),
                true,
            ))
        }
        (Some((q_v, q_p)), None) => {
            let author_did = q_v.issuer.clone().ok_or_else(|| {
                PresentationError::RecordRefused("missing quote issuer".to_string())
            })?;
            Ok((
                &q_p.terms,
                Some(q_p.quote_id.clone()),
                author_did,
                q_p.counterparty_did.clone(),
                true,
            ))
        }
        (None, Some((_, r_p))) => Ok((
            &r_p.terms,
            None,
            r_p.quote_author_did.clone(),
            r_p.counterparty_did.clone(),
            false,
        )),
        (None, None) => Err(PresentationError::MissingTerms),
    }
}

fn check_settlement_and_progress_records(
    author_did: &str,
    counterparty_did: &str,
    terms: &SignedTerms,
    progress: Option<(&RecordVerdict<AgreementProgressPayload>, &AgreementProgressPayload)>,
    settlements: &[&VerifiedRecord],
) -> Result<(), PresentationError> {
    if let Some((_, p)) = progress {
        if p.quote_author_did != author_did || p.counterparty_did != counterparty_did {
            return Err(PresentationError::PartyMismatch);
        }
        for cp in &p.commitments {
            if !terms.commitments.iter().any(|c| c.commitment_id == cp.commitment_id) {
                return Err(PresentationError::CommitmentMismatch(format!(
                    "snapshot commitment '{}' not found in deal terms",
                    cp.commitment_id
                )));
            }
        }
    }

    for s in settlements {
        let (author, cp, cid, is_payment) = match s {
            VerifiedRecord::PaymentRequest(_, p) => {
                (&p.quote_author_did, &p.counterparty_did, &p.commitment_id, true)
            }
            VerifiedRecord::PaymentAck(_, p) => {
                (&p.quote_author_did, &p.counterparty_did, &p.commitment_id, true)
            }
            VerifiedRecord::Fulfilment(_, p) => {
                (&p.quote_author_did, &p.counterparty_did, &p.commitment_id, false)
            }
            _ => continue,
        };

        if author != author_did || cp != counterparty_did {
            return Err(PresentationError::PartyMismatch);
        }

        let commitment =
            terms.commitments.iter().find(|c| &c.commitment_id == cid).ok_or_else(|| {
                PresentationError::CommitmentMismatch(format!(
                    "commitment '{cid}' not found in deal terms"
                ))
            })?;

        if is_payment && commitment.kind != CommitmentKind::Currency {
            return Err(PresentationError::CommitmentMismatch(format!(
                "payment record commitment '{cid}' is not currency"
            )));
        }
        if !is_payment && commitment.kind == CommitmentKind::Currency {
            return Err(PresentationError::CommitmentMismatch(format!(
                "fulfilment record commitment '{cid}' is currency"
            )));
        }
    }

    Ok(())
}

fn check_writer_binding(deal: &DealRecords<'_>) -> WriterCheck {
    let Some((snap_v, _)) = deal.progress else {
        return WriterCheck::NoSnapshot;
    };
    let Some(snap_issuer) = snap_v.issuer.as_deref() else {
        return WriterCheck::NotChecked;
    };

    let author_half = deal.receipts.iter().find(|(_, p)| p.role == DealRole::QuoteAuthor);
    if let Some((v, _)) = author_half
        && v.signer_did.as_deref() == Some(snap_issuer)
    {
        return WriterCheck::Checked;
    }

    if let Some((v, _)) = deal.quote
        && v.signer_did.as_deref() == Some(snap_issuer)
    {
        return WriterCheck::Checked;
    }

    WriterCheck::NotChecked
}

fn calculate_max_disclosures(deal: &DealRecords<'_>) -> usize {
    let mut total = deal.signed_terms.commitments.len() * 10 + 5;
    for s in &deal.settlements {
        total += match s {
            VerifiedRecord::PaymentRequest(_, _) => 1,
            VerifiedRecord::PaymentAck(_, _) => 3,
            VerifiedRecord::Fulfilment(_, _) => 1,
            _ => 0,
        };
    }
    total
}

fn reveal_settlement_views(
    settlements: &[&VerifiedRecord],
    index: &DisclosureIndex,
    all_used: &mut Vec<String>,
) -> Result<Vec<SettlementView>, PresentationError> {
    let mut views = Vec::with_capacity(settlements.len());
    for s in settlements {
        let view = match s {
            VerifiedRecord::PaymentRequest(v, p) => reveal_settlement_record(
                v.record_id.as_deref().unwrap_or_default(),
                &p.commitment_id,
                SettlementRole::Receiver,
                &p.sd,
                &PAYMENT_REQUEST_SET,
                index,
                all_used,
            )?,
            VerifiedRecord::PaymentAck(v, p) => reveal_settlement_record(
                v.record_id.as_deref().unwrap_or_default(),
                &p.commitment_id,
                p.role,
                &p.sd,
                &PAYMENT_ACK_SET,
                index,
                all_used,
            )?,
            VerifiedRecord::Fulfilment(v, p) => reveal_settlement_record(
                v.record_id.as_deref().unwrap_or_default(),
                &p.commitment_id,
                p.role,
                &p.sd,
                &FULFILMENT_SET,
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
    record_id: &str,
    commitment_id: &str,
    role: SettlementRole,
    sd: &[String],
    set: &HiddenSet,
    index: &DisclosureIndex,
    all_used: &mut Vec<String>,
) -> Result<SettlementView, PresentationError> {
    let mut obj = Map::new();
    obj.insert("_sd".to_string(), Value::Array(sd.iter().cloned().map(Value::String).collect()));
    let revealed = reveal_object(&obj, set, index).map_err(PresentationError::Sd)?;

    for u in revealed.used {
        all_used.push(u);
    }

    validate_revealed_settlement_fields(&revealed.fields, set)?;

    let hidden = revealed.hidden.into_iter().map(|s| s.to_string()).collect();
    Ok(SettlementView {
        record_id: record_id.to_string(),
        commitment_id: commitment_id.to_string(),
        role,
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

fn build_commitment_outcomes(
    commitments: &[SignedCommitment],
    progress_rec: Option<&(&RecordVerdict<AgreementProgressPayload>, &AgreementProgressPayload)>,
) -> Vec<CommitmentOutcome> {
    let mut outcomes = Vec::with_capacity(commitments.len());
    for c in commitments {
        let (state, reason, claimed) = if let Some((_, p)) = progress_rec {
            p.commitments
                .iter()
                .find(|cp| cp.commitment_id == c.commitment_id)
                .map(|cp| (Some(cp.state), cp.reason.clone(), cp.claimed))
                .unwrap_or((None, None, None))
        } else {
            (None, None, None)
        };

        outcomes.push(CommitmentOutcome {
            id: c.commitment_id.clone(),
            giver: c.giver,
            resource_kind: c.resource_kind,
            kind: c.kind,
            extension: c.extension.clone(),
            state,
            reason,
            claimed,
        });
    }
    outcomes
}
