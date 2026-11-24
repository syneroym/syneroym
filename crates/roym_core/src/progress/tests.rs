use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::{
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind, SettlementRole},
    lang::LangTag,
    record::RECORD_AGREEMENT_PROGRESS,
    transaction::{DealRole, TimeWindow},
};

fn sample_terms() -> DealTerms {
    DealTerms {
        currency: "INR".to_string(),
        quote_expires_at_secs: 100_000,
        home_exchange: None,
        commitments: vec![
            Commitment {
                commitment_id: "c1".to_string(),
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: Vec::new(),
                description: LocalText {
                    lang: LangTag::parse("en").unwrap(),
                    text: "Lesson".to_string(),
                    translations: Vec::new(),
                },
                declared_value_minor: 100,
                details: None,
                window: Some(TimeWindow { earliest_secs: 1_000, latest_secs: 2_000 }),
                location: None,
                payee: None,
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
            Commitment {
                commitment_id: "c2".to_string(),
                giver: DealRole::Counterparty,
                resource_kind: ResourceKind::Currency,
                kind: CommitmentKind::Currency,
                extension: None,
                after: Vec::new(),
                description: LocalText {
                    lang: LangTag::parse("en").unwrap(),
                    text: "INR 100".to_string(),
                    translations: Vec::new(),
                },
                declared_value_minor: 100,
                details: None,
                window: None,
                location: None,
                payee: Some("Author".to_string()),
                payment_methods: Some(vec!["cash".to_string()]),
                tax_minor: Some(0),
                fees_minor: Some(0),
                eep: None,
            },
        ],
        listing_id: None,
        scope: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Tutoring".to_string(),
            translations: Vec::new(),
        },
        cancellation_terms: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Cancel terms".to_string(),
            translations: Vec::new(),
        },
        refund_terms: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Refund terms".to_string(),
            translations: Vec::new(),
        },
        dispute_path: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Dispute path".to_string(),
            translations: Vec::new(),
        },
    }
}

fn sample_progress() -> AgreementProgressPayload {
    open(
        "rec_quote123".to_string(),
        "conv_123".to_string(),
        "did:key:zCounterparty".to_string(),
        "did:key:zQuoteAuthor".to_string(),
        &sample_terms(),
        FormedAs::Quote,
        None,
    )
}

#[test]
fn open_builds_valid_agreed_snapshot() {
    let p = sample_progress();
    assert_eq!(p.state, AgreementState::Agreed);
    assert_eq!(p.seq, 1);
    assert_eq!(p.commitments.len(), 2);
    assert_eq!(p.commitments[0].state, CommitmentState::Promised);
    assert_eq!(p.commitments[1].state, CommitmentState::Promised);
    assert!(p.validate().is_ok());
}

#[test]
fn a_repeated_event_is_no_change() {
    let p = sample_progress();
    let terms = sample_terms();
    let p1 = apply(
        &p,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_half1".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p1.commitments[0].state, CommitmentState::Claimed);

    let p2 = apply(
        &p1,
        &terms,
        1001,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_half1".to_string(),
        },
    )
    .unwrap();
    assert!(p2.is_none());
}

#[test]
fn payment_and_fulfilment_settle_independently() {
    let p = sample_progress();
    let terms = sample_terms();

    // Payment request on c2
    let p1 = apply(
        &p,
        &terms,
        1000,
        &AgreementEvent::PaymentRequest { commitment_id: "c2".to_string() },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p1.commitments[1].state, CommitmentState::PaymentRequested);

    // Receiver half (payee acknowledges) on c2 directly settles c2
    let p2 = apply(
        &p1,
        &terms,
        1001,
        &AgreementEvent::Half {
            commitment_id: "c2".to_string(),
            role: SettlementRole::Receiver,
            record_id: "rec_pay_ack".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p2.commitments[1].state, CommitmentState::Acknowledged);
    assert_eq!(p2.state, AgreementState::Agreed); // agreement stays open because c1 is not final

    // Fulfilment receiver half on c1 settles c1
    let p3 = apply(
        &p2,
        &terms,
        1002,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Receiver,
            record_id: "rec_fulfil_ack".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p3.commitments[0].state, CommitmentState::Fulfilled);
    // When all commitments are final, agreement closes!
    assert_eq!(p3.state, AgreementState::Closed);
}

#[test]
fn cancel_is_refused_once_a_commitment_moves() {
    let p = sample_progress();
    let terms = sample_terms();

    let p1 = apply(
        &p,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_half1".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p1.commitments[0].state, CommitmentState::Claimed);

    let res =
        apply(&p1, &terms, 1001, &AgreementEvent::Cancel { reason: CancelReason::CannotDeliver });
    assert!(matches!(res, Err(TransitionError::CannotCancel)));

    // Cancel on fresh snapshot succeeds
    let p_cancel =
        apply(&p, &terms, 1000, &AgreementEvent::Cancel { reason: CancelReason::ScheduleConflict })
            .unwrap()
            .unwrap();
    assert_eq!(p_cancel.state, AgreementState::Cancelled);
    assert_eq!(p_cancel.cancel_reason, Some(CancelReason::ScheduleConflict));
    assert_eq!(p_cancel.commitments[0].state, CommitmentState::Cancelled);
    assert_eq!(p_cancel.commitments[1].state, CommitmentState::Cancelled);
}

#[test]
fn failure_cascades_to_unsettled_commitments() {
    let p = sample_progress();
    let terms = sample_terms();

    let p_fail = apply(
        &p,
        &terms,
        1000,
        &AgreementEvent::Failure {
            commitment_id: "c1".to_string(),
            reason: "reservation-lost".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p_fail.commitments[0].state, CommitmentState::Failed);
    assert_eq!(p_fail.commitments[0].reason.as_deref(), Some("reservation-lost"));
    // c2 was Promised, so it gets cancelled via OtherFailed
    assert_eq!(p_fail.commitments[1].state, CommitmentState::Cancelled);
    assert_eq!(p_fail.commitments[1].reason.as_deref(), Some("other-commitment-failed"));
    assert_eq!(p_fail.state, AgreementState::Closed);
}

#[test]
fn tick_expires_unconfirmed_commitments() {
    let p = sample_progress();
    let terms = sample_terms();
    let agreed_at = 1_000;

    // Tick before track_end: no change
    let res =
        apply(&p, &terms, agreed_at, &AgreementEvent::Tick { now_secs: agreed_at + 100 }).unwrap();
    assert!(res.is_none());

    // Tick after track_end (window.latest_secs (2000) + 30 days):
    let past_end = 2_000 + 30 * 24 * 3600 + 1;
    let p_ticked = apply(&p, &terms, agreed_at, &AgreementEvent::Tick { now_secs: past_end })
        .unwrap()
        .unwrap();
    assert_eq!(p_ticked.commitments[0].state, CommitmentState::EndedUnconfirmed);
    assert_eq!(p_ticked.commitments[0].claimed, Some(false));
    assert_eq!(p_ticked.commitments[1].state, CommitmentState::EndedUnconfirmed);
    assert_eq!(p_ticked.commitments[1].claimed, Some(false));
    assert_eq!(p_ticked.state, AgreementState::Closed);
}

#[test]
fn verify_valid_agreement_progress() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let p = sample_progress();
    let draft = RecordDraft {
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
        version: AGREEMENT_PROGRESS_VERSION,
        subject: p.agreement.clone(),
        payload: serde_json::to_value(&p).unwrap(),
        supersedes: None,
        expires_at_secs: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, 1000).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();

    let v = verify_agreement_progress(&env_json, 1000);
    assert!(v.verified);
    assert_eq!(v.payload.unwrap(), p);
}

#[test]
fn verify_refuses_expiry() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let p = sample_progress();
    let draft = RecordDraft {
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
        version: AGREEMENT_PROGRESS_VERSION,
        subject: p.agreement.clone(),
        payload: serde_json::to_value(&p).unwrap(),
        supersedes: None,
        expires_at_secs: Some(2000),
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, 1000).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();

    let v = verify_agreement_progress(&env_json, 1000);
    assert!(!v.verified);
    assert!(v.reason.unwrap().contains("may not declare an expiry"));
}

#[test]
fn notices_match_spec_text() {
    assert_eq!(
        ONE_PAYMENT_NOTICE,
        "Each payment in this quote is paid once. A deposit is its own payment."
    );
    assert_eq!(TRACK_UNCONFIRMED, "No confirmation was recorded before the window closed.");
}

#[test]
fn snapshot_from_another_signer_is_not_filed() {
    let author_key = Identity::generate().unwrap();
    let author_did = substrate::derive_did_key(&author_key.public_key());

    let other_key = Identity::generate().unwrap();
    let other_did = substrate::derive_did_key(&other_key.public_key());

    let mut p = sample_progress();
    p.quote_author_did = author_did;

    let draft = RecordDraft {
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
        version: AGREEMENT_PROGRESS_VERSION,
        subject: p.agreement.clone(),
        payload: serde_json::to_value(&p).unwrap(),
        supersedes: None,
        expires_at_secs: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, other_did.clone(), None, 1000).unwrap();
    let sig = z32::encode(&other_key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();

    let v = verify_agreement_progress(&env_json, 1000);
    assert!(v.verified);
    assert_ne!(v.signer_did.as_deref(), Some(p.quote_author_did.as_str()));
    assert_eq!(v.signer_did.as_deref(), Some(other_did.as_str()));
}

#[test]
fn formed_as_is_kept_by_every_apply() {
    let terms = sample_terms();
    let mut p = open(
        "rec_quote123".to_string(),
        "conv_123".to_string(),
        "did:key:zCounterparty".to_string(),
        "did:key:zQuoteAuthor".to_string(),
        &terms,
        FormedAs::AsListedConfirmed,
        None,
    );
    assert_eq!(p.formed_as, FormedAs::AsListedConfirmed);

    let events = [
        AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_half1".to_string(),
        },
        AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_half1".to_string(),
        },
        AgreementEvent::Tick { now_secs: 1_500 },
    ];
    for event in events {
        if let Some(next) = apply(&p, &terms, 1_000, &event).expect("apply") {
            assert_eq!(next.formed_as, FormedAs::AsListedConfirmed, "{event:?}");
            p = next;
        }
    }

    let fresh = open(
        "rec_quote123".to_string(),
        "conv_123".to_string(),
        "did:key:zCounterparty".to_string(),
        "did:key:zQuoteAuthor".to_string(),
        &terms,
        FormedAs::AsListedInstant,
        None,
    );
    let cancelled =
        apply(&fresh, &terms, 1_000, &AgreementEvent::Cancel { reason: CancelReason::Other })
            .expect("apply")
            .expect("a change");
    assert_eq!(cancelled.formed_as, FormedAs::AsListedInstant);

    let not_formed = open(
        "rec_quote123".to_string(),
        "conv_123".to_string(),
        "did:key:zCounterparty".to_string(),
        "did:key:zQuoteAuthor".to_string(),
        &terms,
        FormedAs::AsListedInstant,
        Some("slot-taken".to_string()),
    );
    assert_eq!(not_formed.state, AgreementState::NotFormed);
    assert_eq!(not_formed.formed_as, FormedAs::AsListedInstant);
}
