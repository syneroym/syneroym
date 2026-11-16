use crate::{
    deal::{
        Commitment, CommitmentKind, DealRole, DealTerms, LocalText, ResourceKind, SettlementRole,
        transition::{
            CommitmentEvent, CommitmentState, Step, TRACK_WINDOW_SECS, is_final, step, track_end,
        },
    },
    lang::LangTag,
    progress::{AgreementEvent, AgreementState, CancelReason, TransitionError},
    transaction::TimeWindow,
};

#[test]
fn fulfilment_table_covers_every_state_and_event() {
    test_fulfilment_promised();
    test_fulfilment_claimed();
    test_fulfilment_final();
}

fn test_fulfilment_promised() {
    let k = CommitmentKind::Fulfilment;
    let s = CommitmentState::Promised;
    assert_eq!(
        step(k, s, CommitmentEvent::GiverHalf),
        Step::To { state: CommitmentState::Claimed, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::ReceiverHalf),
        Step::To { state: CommitmentState::Fulfilled, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Tick),
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(false) }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Cancel),
        Step::To { state: CommitmentState::Cancelled, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Failure),
        Step::To {
            state: CommitmentState::Failed,
            reason: Some("reservation-lost"),
            claimed: None,
        }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::OtherFailed),
        Step::To {
            state: CommitmentState::Cancelled,
            reason: Some("other-commitment-failed"),
            claimed: None,
        }
    );
    assert_eq!(step(k, s, CommitmentEvent::PaymentRequest), Step::Refused);
}

fn test_fulfilment_claimed() {
    let k = CommitmentKind::Fulfilment;
    let s = CommitmentState::Claimed;
    assert_eq!(step(k, s, CommitmentEvent::GiverHalf), Step::NoChange);
    assert_eq!(
        step(k, s, CommitmentEvent::ReceiverHalf),
        Step::To { state: CommitmentState::Fulfilled, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Tick),
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(true) }
    );
    assert_eq!(step(k, s, CommitmentEvent::Cancel), Step::Refused);
    assert_eq!(step(k, s, CommitmentEvent::Failure), Step::Refused);
    assert_eq!(step(k, s, CommitmentEvent::OtherFailed), Step::NoChange);
    assert_eq!(step(k, s, CommitmentEvent::PaymentRequest), Step::Refused);
}

fn test_fulfilment_final() {
    let k = CommitmentKind::Fulfilment;
    for final_state in [
        CommitmentState::Fulfilled,
        CommitmentState::Acknowledged,
        CommitmentState::EndedUnconfirmed,
        CommitmentState::Failed,
        CommitmentState::Cancelled,
    ] {
        assert!(is_final(final_state));
        assert_eq!(step(k, final_state, CommitmentEvent::GiverHalf), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::ReceiverHalf), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::Tick), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::OtherFailed), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::Cancel), Step::Refused);
        assert_eq!(step(k, final_state, CommitmentEvent::Failure), Step::Refused);
        assert_eq!(step(k, final_state, CommitmentEvent::PaymentRequest), Step::Refused);
    }
}

#[test]
fn currency_table_covers_every_state_and_event() {
    test_currency_promised();
    test_currency_payment_requested();
    test_currency_claimed();
    test_currency_final();
}

fn test_currency_promised() {
    let k = CommitmentKind::Currency;
    let s = CommitmentState::Promised;
    assert_eq!(
        step(k, s, CommitmentEvent::PaymentRequest),
        Step::To { state: CommitmentState::PaymentRequested, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::GiverHalf),
        Step::To { state: CommitmentState::Claimed, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::ReceiverHalf),
        Step::To { state: CommitmentState::Acknowledged, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Tick),
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(false) }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Cancel),
        Step::To { state: CommitmentState::Cancelled, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::OtherFailed),
        Step::To {
            state: CommitmentState::Cancelled,
            reason: Some("other-commitment-failed"),
            claimed: None,
        }
    );
    assert_eq!(step(k, s, CommitmentEvent::Failure), Step::Refused);
}

fn test_currency_payment_requested() {
    let k = CommitmentKind::Currency;
    let s = CommitmentState::PaymentRequested;
    assert_eq!(step(k, s, CommitmentEvent::PaymentRequest), Step::NoChange);
    assert_eq!(
        step(k, s, CommitmentEvent::GiverHalf),
        Step::To { state: CommitmentState::Claimed, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::ReceiverHalf),
        Step::To { state: CommitmentState::Acknowledged, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Tick),
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(false) }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Cancel),
        Step::To { state: CommitmentState::Cancelled, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::OtherFailed),
        Step::To {
            state: CommitmentState::Cancelled,
            reason: Some("other-commitment-failed"),
            claimed: None,
        }
    );
    assert_eq!(step(k, s, CommitmentEvent::Failure), Step::Refused);
}

fn test_currency_claimed() {
    let k = CommitmentKind::Currency;
    let s = CommitmentState::Claimed;
    assert_eq!(step(k, s, CommitmentEvent::PaymentRequest), Step::NoChange);
    assert_eq!(step(k, s, CommitmentEvent::GiverHalf), Step::NoChange);
    assert_eq!(
        step(k, s, CommitmentEvent::ReceiverHalf),
        Step::To { state: CommitmentState::Acknowledged, reason: None, claimed: None }
    );
    assert_eq!(
        step(k, s, CommitmentEvent::Tick),
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(true) }
    );
    assert_eq!(step(k, s, CommitmentEvent::Cancel), Step::Refused);
    assert_eq!(step(k, s, CommitmentEvent::OtherFailed), Step::NoChange);
    assert_eq!(step(k, s, CommitmentEvent::Failure), Step::Refused);
}

fn test_currency_final() {
    let k = CommitmentKind::Currency;
    for final_state in [
        CommitmentState::Fulfilled,
        CommitmentState::Acknowledged,
        CommitmentState::EndedUnconfirmed,
        CommitmentState::Failed,
        CommitmentState::Cancelled,
    ] {
        assert!(is_final(final_state));
        assert_eq!(step(k, final_state, CommitmentEvent::PaymentRequest), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::GiverHalf), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::ReceiverHalf), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::Tick), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::OtherFailed), Step::NoChange);
        assert_eq!(step(k, final_state, CommitmentEvent::Cancel), Step::Refused);
        assert_eq!(step(k, final_state, CommitmentEvent::Failure), Step::Refused);
    }
}

#[test]
fn track_end_uses_the_later_of_window_and_agreement() {
    let agreed_at = 1_000_000;

    // No window: agreed_at + 30 days
    assert_eq!(track_end(None, agreed_at), agreed_at + TRACK_WINDOW_SECS);

    // Window ended before agreement: uses agreed_at
    let early_window = TimeWindow { earliest_secs: 500_000, latest_secs: 600_000 };
    assert_eq!(track_end(Some(&early_window), agreed_at), agreed_at + TRACK_WINDOW_SECS);

    // Window ends after agreement: uses window.latest_secs
    let late_window = TimeWindow { earliest_secs: 1_200_000, latest_secs: 1_500_000 };
    assert_eq!(track_end(Some(&late_window), agreed_at), 1_500_000 + TRACK_WINDOW_SECS);
}

#[test]
fn ended_unconfirmed_says_whether_the_giver_claimed() {
    // Fulfilment: from Promised -> claimed: false
    let step_promised =
        step(CommitmentKind::Fulfilment, CommitmentState::Promised, CommitmentEvent::Tick);
    assert_eq!(
        step_promised,
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(false) }
    );

    // Fulfilment: from Claimed -> claimed: true
    let step_claimed =
        step(CommitmentKind::Fulfilment, CommitmentState::Claimed, CommitmentEvent::Tick);
    assert_eq!(
        step_claimed,
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(true) }
    );

    // Currency: from PaymentRequested -> claimed: false
    let step_payreq =
        step(CommitmentKind::Currency, CommitmentState::PaymentRequested, CommitmentEvent::Tick);
    assert_eq!(
        step_payreq,
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(false) }
    );

    // Currency: from Claimed -> claimed: true
    let step_curr_claimed =
        step(CommitmentKind::Currency, CommitmentState::Claimed, CommitmentEvent::Tick);
    assert_eq!(
        step_curr_claimed,
        Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(true) }
    );
}

fn sample_terms_two_commitments() -> DealTerms {
    DealTerms {
        listing_id: None,
        currency: "EUR".to_string(),
        quote_expires_at_secs: 15000,
        home_exchange: None,
        commitments: vec![
            Commitment {
                commitment_id: "c1".to_string(),
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: Vec::new(),
                description: LocalText::plain(LangTag::parse("en").unwrap(), "Service"),
                declared_value_minor: 5000,
                details: None,
                window: None,
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
                description: LocalText::plain(LangTag::parse("en").unwrap(), "Payment"),
                declared_value_minor: 5000,
                details: None,
                window: None,
                location: None,
                payee: Some("did:key:author".to_string()),
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
        ],
        scope: LocalText::plain(LangTag::parse("en").unwrap(), "Scope"),
        cancellation_terms: LocalText::plain(LangTag::parse("en").unwrap(), "Cancel terms"),
        refund_terms: LocalText::plain(LangTag::parse("en").unwrap(), "Refund terms"),
        dispute_path: LocalText::plain(LangTag::parse("en").unwrap(), "Dispute"),
    }
}

#[test]
fn progress_apply_closes_only_when_every_commitment_is_final() {
    let terms = sample_terms_two_commitments();
    let snap0 = crate::progress::open(
        "agr_1".to_string(),
        "conv_1".to_string(),
        "did:key:cp".to_string(),
        "did:key:author".to_string(),
        &terms,
        None,
    );
    assert_eq!(snap0.state, AgreementState::Agreed);

    let snap1 = crate::progress::apply(
        &snap0,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_g1".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap1.state, AgreementState::Agreed);

    let snap2 = crate::progress::apply(
        &snap1,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Receiver,
            record_id: "rec_r1".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap2.state, AgreementState::Agreed);
    assert_eq!(snap2.commitments[0].state, CommitmentState::Fulfilled);
    assert_eq!(snap2.commitments[1].state, CommitmentState::Promised);

    let snap3 = crate::progress::apply(
        &snap2,
        &terms,
        1000,
        &AgreementEvent::PaymentRequest { commitment_id: "c2".to_string() },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap3.state, AgreementState::Agreed);

    let snap4 = crate::progress::apply(
        &snap3,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c2".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_g2".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap4.state, AgreementState::Agreed);

    let snap5 = crate::progress::apply(
        &snap4,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c2".to_string(),
            role: SettlementRole::Receiver,
            record_id: "rec_r2".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap5.commitments[0].state, CommitmentState::Fulfilled);
    assert_eq!(snap5.commitments[1].state, CommitmentState::Acknowledged);
    assert_eq!(snap5.state, AgreementState::Closed);
}

#[test]
fn a_failed_commitment_cancels_the_open_ones() {
    let terms = sample_terms_two_commitments();
    let snap0 = crate::progress::open(
        "agr_1".to_string(),
        "conv_1".to_string(),
        "did:key:cp".to_string(),
        "did:key:author".to_string(),
        &terms,
        None,
    );
    assert_eq!(snap0.commitments[0].state, CommitmentState::Promised);
    assert_eq!(snap0.commitments[1].state, CommitmentState::Promised);

    let snap1 = crate::progress::apply(
        &snap0,
        &terms,
        1000,
        &AgreementEvent::Failure {
            commitment_id: "c1".to_string(),
            reason: "equipment-breakdown".to_string(),
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(snap1.commitments[0].state, CommitmentState::Failed);
    assert_eq!(snap1.commitments[0].reason.as_deref(), Some("equipment-breakdown"));
    assert_eq!(snap1.commitments[1].state, CommitmentState::Cancelled);
    assert_eq!(snap1.commitments[1].reason.as_deref(), Some("other-commitment-failed"));
}

#[test]
fn cancel_needs_every_commitment_open() {
    let terms = sample_terms_two_commitments();
    let snap0 = crate::progress::open(
        "agr_1".to_string(),
        "conv_1".to_string(),
        "did:key:cp".to_string(),
        "did:key:author".to_string(),
        &terms,
        None,
    );

    let snap_cancel = crate::progress::apply(
        &snap0,
        &terms,
        1000,
        &AgreementEvent::Cancel { reason: CancelReason::Other },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap_cancel.state, AgreementState::Cancelled);
    assert_eq!(snap_cancel.commitments[0].state, CommitmentState::Cancelled);
    assert_eq!(snap_cancel.commitments[1].state, CommitmentState::Cancelled);

    let snap_moved = crate::progress::apply(
        &snap0,
        &terms,
        1000,
        &AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_g1".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(snap_moved.commitments[0].state, CommitmentState::Claimed);

    let err = crate::progress::apply(
        &snap_moved,
        &terms,
        1000,
        &AgreementEvent::Cancel { reason: CancelReason::Other },
    )
    .unwrap_err();
    assert!(matches!(err, TransitionError::CannotCancel));
}
