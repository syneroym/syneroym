use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::record::RECORD_BOOKING_PROGRESS;

fn sample_booking() -> BookingProgressPayload {
    open(
        "rec_quote123".to_string(),
        "conv_123".to_string(),
        "did:key:zConsumer".to_string(),
        "did:key:zProvider".to_string(),
        None,
        100_000,
    )
}

#[test]
fn a_repeated_event_is_no_change() {
    let b = sample_booking();
    let b1 = apply(&b, &BookingEvent::Start, 1000).unwrap().unwrap();
    assert_eq!(b1.state, BookingState::InProgress);
    let b2 = apply(&b1, &BookingEvent::Start, 1001).unwrap();
    assert!(b2.is_none());
}

#[test]
fn a_self_serving_half_never_acknowledges() {
    let b = sample_booking();
    // Consumer claiming payment
    let b1 = apply(&b, &BookingEvent::Half { track: Track::Payment, role: Role::Consumer }, 1000)
        .unwrap()
        .unwrap();
    assert_eq!(b1.payment, TrackState::Claimed);

    // Another claim by consumer stays claimed
    let b2 = apply(&b1, &BookingEvent::Half { track: Track::Payment, role: Role::Consumer }, 1001)
        .unwrap();
    assert!(b2.is_none());

    // Provider claiming fulfilment
    let b3 =
        apply(&b, &BookingEvent::Half { track: Track::Fulfilment, role: Role::Provider }, 1000)
            .unwrap()
            .unwrap();
    assert_eq!(b3.fulfilment, TrackState::Claimed);
}

#[test]
fn completed_needs_both_acknowledged() {
    let b = sample_booking();
    let b1 = apply(&b, &BookingEvent::Half { track: Track::Payment, role: Role::Provider }, 1000)
        .unwrap()
        .unwrap();
    assert_eq!(b1.payment, TrackState::Acknowledged);
    assert_eq!(b1.state, BookingState::InProgress);

    let b2 =
        apply(&b1, &BookingEvent::Half { track: Track::Fulfilment, role: Role::Consumer }, 1001)
            .unwrap()
            .unwrap();
    assert_eq!(b2.fulfilment, TrackState::Acknowledged);
    assert_eq!(b2.state, BookingState::Completed);
}

#[test]
fn cancel_is_refused_once_a_track_moved() {
    let b = sample_booking();
    let b1 = apply(&b, &BookingEvent::Half { track: Track::Payment, role: Role::Consumer }, 1000)
        .unwrap()
        .unwrap();
    assert_eq!(b1.payment, TrackState::Claimed);

    let res = apply(&b1, &BookingEvent::Cancel { reason: "change of mind".to_string() }, 1001);
    assert!(matches!(res, Err(TransitionError::CannotCancel)));

    // Cancel on fresh booking succeeds
    let b_cancel =
        apply(&b, &BookingEvent::Cancel { reason: "schedule conflict".to_string() }, 1000)
            .unwrap()
            .unwrap();
    assert_eq!(b_cancel.state, BookingState::Cancelled);
    assert_eq!(b_cancel.cancelled_by, Some(Role::Provider));
    assert_eq!(b_cancel.cancel_reason.as_deref(), Some("schedule conflict"));

    // Long reason fails
    let long_reason = "x".repeat(MAX_CANCEL_REASON_LEN + 1);
    let res_long = apply(&b, &BookingEvent::Cancel { reason: long_reason }, 1000);
    assert!(matches!(res_long, Err(TransitionError::ReasonTooLong)));
}

#[test]
fn the_time_edge_runs_before_the_event() {
    let b = sample_booking();
    // After track_window_ends_at_secs (100_000)
    let res =
        apply(&b, &BookingEvent::Half { track: Track::Payment, role: Role::Provider }, 100_001)
            .unwrap()
            .unwrap();
    // Tracks expired before event, moving both None to Unconfirmed ->
    // EndedUnconfirmed
    assert_eq!(res.state, BookingState::EndedUnconfirmed);
    assert_eq!(res.payment, TrackState::Unconfirmed);
    assert_eq!(res.fulfilment, TrackState::Unconfirmed);
}

#[test]
fn a_terminal_track_ignores_a_late_half() {
    let mut b = sample_booking();
    b.state = BookingState::InProgress;
    b.payment = TrackState::Unconfirmed;
    b.track_window_ends_at_secs = 200_000;

    // A half arriving for an unconfirmed track does not change it
    let res = apply(&b, &BookingEvent::Half { track: Track::Payment, role: Role::Provider }, 1000)
        .unwrap();
    assert!(res.is_none());
}

#[test]
fn one_acknowledged_one_unconfirmed_ends_unconfirmed() {
    let b = sample_booking();
    let b1 = apply(&b, &BookingEvent::Half { track: Track::Payment, role: Role::Provider }, 1000)
        .unwrap()
        .unwrap();
    assert_eq!(b1.payment, TrackState::Acknowledged);

    // Tick past window
    let b2 = apply(&b1, &BookingEvent::Tick, 100_001).unwrap().unwrap();
    assert_eq!(b2.payment, TrackState::Acknowledged);
    assert_eq!(b2.fulfilment, TrackState::Unconfirmed);
    assert_eq!(b2.state, BookingState::EndedUnconfirmed);
}

#[test]
fn validate_refuses_invalid_snapshots() {
    let mut b = sample_booking();
    assert!(b.validate().is_ok());

    b.agreement = "bad_prefix".to_string();
    assert!(matches!(b.validate(), Err(BookingPayloadError::InvalidAgreement(_))));
    b.agreement = "rec_ok".to_string();

    b.consumer_did = "invalid_did".to_string();
    assert!(matches!(b.validate(), Err(BookingPayloadError::InvalidConsumerDid(_))));
    b.consumer_did = "did:key:zConsumer".to_string();

    b.provider_did = b.consumer_did.clone();
    assert!(matches!(b.validate(), Err(BookingPayloadError::SamePartyDid)));
    b.provider_did = "did:key:zProvider".to_string();

    b.conversation = String::new();
    assert!(matches!(b.validate(), Err(BookingPayloadError::ConversationEmpty)));
    b.conversation = "conv_1".to_string();

    b.seq = 0;
    assert!(matches!(b.validate(), Err(BookingPayloadError::InvalidSeq(0))));
    b.seq = 1;

    // conflict without state=Conflict
    b.conflict = Some(ConflictReason::SlotTaken);
    assert!(matches!(b.validate(), Err(BookingPayloadError::ConflictStateMismatch)));
    b.state = BookingState::Conflict;
    assert!(b.validate().is_ok());

    // cancelled without reason/by
    b.conflict = None;
    b.state = BookingState::Cancelled;
    assert!(matches!(b.validate(), Err(BookingPayloadError::CancelledStateMismatch)));
    b.cancelled_by = Some(Role::Provider);
    b.cancel_reason = Some("reason".to_string());
    assert!(b.validate().is_ok());

    // completed without tracks acknowledged
    b.cancelled_by = None;
    b.cancel_reason = None;
    b.state = BookingState::Completed;
    assert!(matches!(b.validate(), Err(BookingPayloadError::CompletedTracksNotAcknowledged)));
    b.payment = TrackState::Acknowledged;
    b.fulfilment = TrackState::Acknowledged;
    assert!(b.validate().is_ok());
}

#[test]
fn next_step_branches() {
    let mut b = sample_booking();

    // Before-work: provider sees RequestPayment, consumer sees PayOutsideRoym
    assert_eq!(next_step(&b, PaymentTiming::BeforeWork, Role::Provider), NextStep::RequestPayment);
    assert_eq!(next_step(&b, PaymentTiming::BeforeWork, Role::Consumer), NextStep::PayOutsideRoym);

    // After-work: provider sees MarkWorkComplete, consumer sees WaitForProvider
    assert_eq!(next_step(&b, PaymentTiming::AfterWork, Role::Provider), NextStep::MarkWorkComplete);
    assert_eq!(next_step(&b, PaymentTiming::AfterWork, Role::Consumer), NextStep::WaitForProvider);

    // Consumer claimed payment
    b.payment = TrackState::Claimed;
    assert_eq!(
        next_step(&b, PaymentTiming::BeforeWork, Role::Provider),
        NextStep::ConfirmPaymentReceived
    );

    // Provider claimed fulfilment
    b.fulfilment = TrackState::Claimed;
    assert_eq!(
        next_step(&b, PaymentTiming::BeforeWork, Role::Consumer),
        NextStep::ConfirmWorkComplete
    );

    // Terminal booking
    b.state = BookingState::Completed;
    assert_eq!(next_step(&b, PaymentTiming::BeforeWork, Role::Provider), NextStep::Nothing);
}

#[test]
fn no_notice_says_verified() {
    let notices = [
        PAYMENT_NOTICE,
        PROGRESS_NOTICE,
        PAYMENT_CLAIMED,
        PAYMENT_ACKNOWLEDGED,
        FULFILMENT_CLAIMED,
        FULFILMENT_ACKNOWLEDGED,
        TRACK_UNCONFIRMED,
        ONE_PAYMENT_NOTICE,
    ];
    for notice in notices {
        assert!(!notice.to_lowercase().contains("verif"), "notice must not say verified: {notice}");
    }
}

#[test]
fn verify_booking_progress_envelope() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let mut b = sample_booking();
    b.provider_did = issuer.clone();
    let draft = RecordDraft {
        version: BOOKING_PROGRESS_VERSION,
        record_type: RECORD_BOOKING_PROGRESS.to_string(),
        subject: b.agreement.clone(),
        payload: serde_json::to_value(&b).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.clone(), None, 1000).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();

    let verdict = verify_booking_progress(&env_json, 1000);
    assert!(verdict.verified);
    assert_eq!(verdict.issuer.as_deref(), Some(issuer.as_str()));
    assert_eq!(verdict.payload, Some(b));
}

#[test]
fn the_ui_wording_matches_this_crate() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wording_path = manifest_dir.join("../roym_web/ui/src/cards/wording.ts");
    assert!(wording_path.exists(), "missing ../roym_web/ui/src/cards/wording.ts");

    let content = std::fs::read_to_string(&wording_path)
        .expect("Failed to read ../roym_web/ui/src/cards/wording.ts");

    let pinned: &[(&str, &str)] = &[
        ("PAYMENT_NOTICE", PAYMENT_NOTICE),
        ("PROGRESS_NOTICE", PROGRESS_NOTICE),
        ("PAYMENT_CLAIMED", PAYMENT_CLAIMED),
        ("PAYMENT_ACKNOWLEDGED", PAYMENT_ACKNOWLEDGED),
        ("FULFILMENT_CLAIMED", FULFILMENT_CLAIMED),
        ("FULFILMENT_ACKNOWLEDGED", FULFILMENT_ACKNOWLEDGED),
        ("TRACK_UNCONFIRMED", TRACK_UNCONFIRMED),
        ("ONE_PAYMENT_NOTICE", ONE_PAYMENT_NOTICE),
    ];
    for (name, value) in pinned {
        assert!(
            content.contains(value),
            "wording.ts is missing the {name} sentence verbatim: {value:?}"
        );
    }
}
