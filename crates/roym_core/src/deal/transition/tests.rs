use crate::{
    deal::{
        CommitmentKind,
        transition::{
            CommitmentEvent, CommitmentState, Step, TRACK_WINDOW_SECS, is_final, step, track_end,
        },
    },
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
