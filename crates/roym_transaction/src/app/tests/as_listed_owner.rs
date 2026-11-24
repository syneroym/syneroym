//! Service tests for the owner's answer to an as-listed tap.

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    as_listed::{self, AsListed},
    deal::transition::CommitmentState,
    listing::{self, Acceptance, DEFAULT_INSTANT_CAP, ListingPayload, ListingStatus},
    paging,
    progress::{AgreementState, CommitmentProgress, FormedAs},
    record::{Envelope, RecordDraft},
    signing::{CERTIFICATE_ID, CERTIFICATES},
    testing::TestHost,
    transaction::{self, DEFAULT_DATA_USE_NOTICE, ENQUIRY_VERSION, RECORD_ENQUIRY},
};
use z32;

use super::{
    as_listed_fixture::{CONVERSATION, consent, listing, sign_listing},
    fixture,
};
use crate::app::{
    AGREEMENTS, AgreementRow, LEDGER, QUOTES, RecordPointerRow,
    as_listed_ops::{
        self, AnswerRow, AnswerState, FromListingInput, OPEN_INSTANT_SECS, Trigger,
        answer::{Outcome, answer_from_listing_at, answer_id},
    },
    backup,
    decide::{self, DecisionRow, DecisionState},
    decline_ops, delete_row, enquiry_ops, get_row,
    ledger::LedgerKind,
    put_row,
};

fn stub_catalog(host: &TestHost, envelope: &str, record_id: &str, listing_id: &str, status: &str) {
    let value = json!({
        "envelope": envelope,
        "record_id": record_id,
        "listing_id": listing_id,
        "status": status,
        "updated_at_secs": 0,
    })
    .to_string();
    host.on_call("catalog", "listing.get", move |_params| Ok(value.clone()));
}

fn stub_catalog_missing(host: &TestHost) {
    host.on_call("catalog", "listing.get", |_params| Ok("null".to_string()));
}

fn enquiry(
    peer: &Identity,
    listing_id: &str,
    consent: AsListed,
    sequence: u32,
    issued_at: u64,
) -> (String, transaction::EnquiryPayload) {
    let peer_did = substrate::derive_did_key(&peer.public_key());
    let enquiry_id =
        transaction::derive_enquiry_id(CONVERSATION, &peer_did, sequence).expect("enquiry id");
    let payload = transaction::EnquiryPayload {
        enquiry_id: enquiry_id.clone(),
        conversation: CONVERSATION.to_string(),
        sequence,
        listing_id: Some(listing_id.to_string()),
        categories: Vec::new(),
        description: None,
        as_listed: Some(consent),
        area: None,
        window: None,
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    };
    let draft = RecordDraft {
        version: ENQUIRY_VERSION,
        record_type: RECORD_ENQUIRY.to_string(),
        subject: enquiry_id,
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, peer_did, None, issued_at).expect("envelope");
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    (env.to_json().expect("json"), payload)
}

async fn hold(
    f: &super::TestFixture,
    envelope: &str,
    payload: &transaction::EnquiryPayload,
    now: u64,
    owner: &str,
) -> String {
    let v = transaction::verify_enquiry(envelope, now);
    assert!(v.verified, "{:?}", v.reason);
    enquiry_ops::store_received_enquiry(&f.host, envelope, &v, payload, now, owner)
        .await
        .expect("store enquiry");
    v.record_id.expect("record id")
}

/// Signs a listing owned by this fixture's owner, serves it from the catalog
/// stub, and holds one tap from `peer`.
async fn tap(
    f: &super::TestFixture,
    peer: &Identity,
    acceptance: Acceptance,
    cap: u16,
    now: u64,
) -> (String, ListingPayload, String) {
    let (envelope, payload) = sign_listing(&f.master, listing(acceptance, cap), now);
    let v = listing::verify_envelope(&envelope, now);
    let record_id = v.record_id.expect("listing record id");
    stub_catalog(&f.host, &envelope, &record_id, &payload.listing_id, "active");
    let (env, enquiry_payload) =
        enquiry(peer, &payload.listing_id, consent(&record_id, now + 3_600), 1, now);
    let enquiry_record = hold(f, &env, &enquiry_payload, now, &f.master_did).await;
    (enquiry_record, payload, record_id)
}

#[tokio::test]
async fn an_instant_offering_is_answered_by_the_driver_with_no_owner_action() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, _, _) =
        tap(&f, &peer, Acceptance::Instant, DEFAULT_INSTANT_CAP, now).await;

    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Quoted(_)), "the driver must answer an instant offering");

    let answer: AnswerRow = get_row(&f.host, LEDGER, &answer_id(&enquiry_record))
        .await
        .expect("read")
        .expect("answer row");
    assert_eq!(answer.state, AnswerState::Answered);
    let ptr: RecordPointerRow =
        get_row(&f.host, QUOTES, &answer.quote_id).await.expect("read").expect("quote");
    assert_eq!(ptr.formed_as, Some(FormedAs::AsListedInstant));
}

#[tokio::test]
async fn a_confirm_first_offering_waits_and_from_listing_answers_it() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, _, _) =
        tap(&f, &peer, Acceptance::ConfirmFirst, DEFAULT_INSTANT_CAP, now).await;

    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Waiting("confirm-first")));
    assert!(
        get_row::<AnswerRow, _>(&f.host, LEDGER, &answer_id(&enquiry_record))
            .await
            .expect("read")
            .is_none(),
        "a waiting outcome writes nothing"
    );

    answer_from_listing_at(
        &f.host,
        FromListingInput { enquiry_record_id: enquiry_record.clone() },
        now,
    )
    .await
    .expect("owner answer");
    let answer: AnswerRow = get_row(&f.host, LEDGER, &answer_id(&enquiry_record))
        .await
        .expect("read")
        .expect("answer row");
    assert_eq!(answer.formed_as, FormedAs::AsListedConfirmed);
    assert_eq!(answer.state, AnswerState::Answered);
}

#[tokio::test]
async fn a_withdrawn_offering_is_declined_with_offering_withdrawn() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, payload, _) =
        tap(&f, &peer, Acceptance::Instant, DEFAULT_INSTANT_CAP, now).await;

    let mut withdrawn = payload.clone();
    withdrawn.status = ListingStatus::Withdrawn;
    let (envelope, withdrawn) = sign_listing(&f.master, withdrawn, now);
    let record_id = listing::verify_envelope(&envelope, now).record_id.expect("record");
    stub_catalog(&f.host, &envelope, &record_id, &withdrawn.listing_id, "withdrawn");

    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Declined));
    let answer: AnswerRow = get_row(&f.host, LEDGER, &answer_id(&enquiry_record))
        .await
        .expect("read")
        .expect("answer row");
    assert_eq!(answer.code.as_deref(), Some("offering-withdrawn"));
}

#[tokio::test]
async fn a_missing_offering_is_declined_with_offering_withdrawn() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, _, _) =
        tap(&f, &peer, Acceptance::Instant, DEFAULT_INSTANT_CAP, now).await;
    stub_catalog_missing(&f.host);

    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Declined));
}

#[tokio::test]
async fn an_expired_consent_is_declined_with_consent_expired() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (envelope, payload) = sign_listing(&f.master, listing(Acceptance::Instant, 20), now);
    let record_id = listing::verify_envelope(&envelope, now).record_id.expect("record");
    stub_catalog(&f.host, &envelope, &record_id, &payload.listing_id, "active");
    let (env, enquiry_payload) =
        enquiry(&peer, &payload.listing_id, consent(&record_id, now + 60), 1, now);
    let enquiry_record = hold(&f, &env, &enquiry_payload, now, &f.master_did).await;

    let outcome =
        as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now + 61)
            .await
            .expect("answer");
    assert!(matches!(outcome, Outcome::Declined));
    let answer: AnswerRow = get_row(&f.host, LEDGER, &answer_id(&enquiry_record))
        .await
        .expect("read")
        .expect("answer row");
    assert_eq!(answer.code.as_deref(), Some("consent-expired"));
}

#[tokio::test]
async fn the_cap_makes_an_instant_offering_wait() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, payload, record_id) = tap(&f, &peer, Acceptance::Instant, 1, now).await;

    let terms = as_listed::terms_from_listing(
        &payload,
        &f.master_did,
        &consent(&record_id, now + 3_600),
        now,
    )
    .expect("terms");
    put_row(
        &f.host,
        AGREEMENTS,
        "rec_open_instant",
        &AgreementRow {
            quote_record_id: "rec_open_instant".to_string(),
            quote_id: "quo_open".to_string(),
            conversation: "conv_other".to_string(),
            counterparty_did: "did:key:zSomeoneElse".to_string(),
            quote_author_did: f.master_did.clone(),
            terms,
            state: Some(syneroym_roym_core::progress::AgreementState::Agreed),
            state_reason: None,
            cancel_reason: None,
            seq: Some(1),
            commitments: Vec::new(),
            agreed_at_secs: Some(now),
            counterparty: None,
            quote_author: None,
            formed_as: Some(FormedAs::AsListedInstant),
            updated_at_secs: now,
        },
    )
    .await
    .expect("seed agreement");

    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Waiting("cap-reached")));
}

#[tokio::test]
async fn a_second_tap_from_one_counterparty_waits() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, payload, record_id) = tap(&f, &peer, Acceptance::Instant, 20, now).await;
    let first = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("first answer");
    assert!(matches!(first, Outcome::Quoted(_)));

    let (env, enquiry_payload) =
        enquiry(&peer, &payload.listing_id, consent(&record_id, now + 3_600), 2, now);
    let second = hold(&f, &env, &enquiry_payload, now, &f.master_did).await;
    let outcome = as_listed_ops::answer_as_listed(&f.host, &second, Trigger::Driver, now)
        .await
        .expect("second answer");
    assert!(matches!(outcome, Outcome::Waiting("counterparty-holds-one")));
}

#[tokio::test]
async fn a_failing_certificate_makes_the_offering_wait() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, _, _) = tap(&f, &peer, Acceptance::Instant, 20, now).await;
    delete_row(&f.host, CERTIFICATES, CERTIFICATE_ID).await.expect("remove certificate");

    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Waiting("signing-not-ready")));
}

#[tokio::test]
async fn two_answers_of_one_tap_make_one_quote() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, _, _) = tap(&f, &peer, Acceptance::Instant, 20, now).await;

    let first = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("first answer");
    let second = as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("second answer");
    let (Outcome::Quoted(first), Outcome::Quoted(second)) = (first, second) else {
        panic!("both answers must quote");
    };
    assert_eq!(first.quote_id, second.quote_id);
    assert_eq!(first.record_id, second.record_id);

    let quotes = paging::count(&f.host, QUOTES, None).await.expect("count");
    assert_eq!(quotes, 1, "one tap makes at most one quote");
}

async fn agreement_row(
    f: &super::TestFixture,
    listing: &ListingPayload,
    record_id: &str,
    quote_id: &str,
    counterparty: &str,
    now: u64,
) -> AgreementRow {
    let terms = as_listed::terms_from_listing(
        listing,
        &f.master_did,
        &consent(record_id, now + 3_600),
        now,
    )
    .expect("terms");
    AgreementRow {
        quote_record_id: record_id.to_string(),
        quote_id: quote_id.to_string(),
        conversation: "conv_other".to_string(),
        counterparty_did: counterparty.to_string(),
        quote_author_did: f.master_did.clone(),
        terms,
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: Vec::new(),
        agreed_at_secs: Some(now),
        counterparty: None,
        quote_author: None,
        formed_as: Some(FormedAs::AsListedInstant),
        updated_at_secs: now,
    }
}

#[tokio::test]
async fn many_taps_before_any_acceptance_make_only_cap_quotes() {
    let f = fixture();
    let now = f.host.now();
    let mut taps = Vec::new();
    for _ in 0..3 {
        let peer = Identity::generate().expect("peer");
        let (enquiry, _, _) = tap(&f, &peer, Acceptance::Instant, 2, now).await;
        taps.push(enquiry);
    }

    let first = as_listed_ops::answer_as_listed(&f.host, &taps[0], Trigger::Driver, now)
        .await
        .expect("first");
    let second = as_listed_ops::answer_as_listed(&f.host, &taps[1], Trigger::Driver, now)
        .await
        .expect("second");
    let third = as_listed_ops::answer_as_listed(&f.host, &taps[2], Trigger::Driver, now)
        .await
        .expect("third");
    assert!(matches!(first, Outcome::Quoted(_)));
    assert!(matches!(second, Outcome::Quoted(_)));
    assert!(matches!(third, Outcome::Waiting("cap-reached")));
}

#[tokio::test]
async fn an_agreement_with_a_filed_half_no_longer_counts() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry, payload, record_id) = tap(&f, &peer, Acceptance::Instant, 1, now).await;
    as_listed_ops::answer_as_listed(&f.host, &enquiry, Trigger::Driver, now).await.expect("answer");
    let answer: AnswerRow =
        get_row(&f.host, LEDGER, &answer_id(&enquiry)).await.expect("read").expect("answer");

    put_row(
        &f.host,
        LEDGER,
        &decide::decision_id(&answer.quote_id),
        &DecisionRow {
            kind: "decision".to_string(),
            state: DecisionState::Agreed,
            agreement: "rec_filed".to_string(),
            quote_id: answer.quote_id.clone(),
            reason: None,
            created_at_secs: now,
        },
    )
    .await
    .expect("decision");
    let mut row =
        agreement_row(&f, &payload, &record_id, &answer.quote_id, &f.master_did, now).await;
    row.commitments = vec![CommitmentProgress {
        commitment_id: "c1".to_string(),
        state: CommitmentState::Fulfilled,
        reason: None,
        claimed: Some(true),
        halves: vec!["rec_half".to_string()],
    }];
    put_row(&f.host, AGREEMENTS, "rec_filed", &row).await.expect("agreement");

    let (next, _, _) =
        tap(&f, &Identity::generate().expect("peer"), Acceptance::Instant, 1, now).await;
    let outcome = as_listed_ops::answer_as_listed(&f.host, &next, Trigger::Driver, now)
        .await
        .expect("second answer");
    assert!(matches!(outcome, Outcome::Quoted(_)));
}

#[tokio::test]
async fn an_agreement_older_than_seven_days_no_longer_counts() {
    let f = fixture();
    let now = f.host.now();
    let (_, payload, record_id) =
        tap(&f, &Identity::generate().expect("peer"), Acceptance::Instant, 1, now).await;
    let mut row = agreement_row(
        &f,
        &payload,
        &record_id,
        "quo_old",
        "did:key:zSomeoneElse",
        now - OPEN_INSTANT_SECS - 1,
    )
    .await;
    row.agreed_at_secs = Some(now - OPEN_INSTANT_SECS - 1);
    put_row(&f.host, AGREEMENTS, "rec_old", &row).await.expect("agreement");

    let (enquiry, _, _) =
        tap(&f, &Identity::generate().expect("peer"), Acceptance::Instant, 1, now).await;
    let outcome = as_listed_ops::answer_as_listed(&f.host, &enquiry, Trigger::Driver, now)
        .await
        .expect("answer");
    assert!(matches!(outcome, Outcome::Quoted(_)));
}

#[tokio::test]
async fn a_declined_instant_quote_frees_its_place() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry, _, _) = tap(&f, &peer, Acceptance::Instant, 1, now).await;
    as_listed_ops::answer_as_listed(&f.host, &enquiry, Trigger::Driver, now).await.expect("answer");
    let answer: AnswerRow =
        get_row(&f.host, LEDGER, &answer_id(&enquiry)).await.expect("read").expect("answer");
    let mut ptr: RecordPointerRow =
        get_row(&f.host, QUOTES, &answer.quote_id).await.expect("read").expect("quote");
    ptr.declined_at_secs = Some(now);
    put_row(&f.host, QUOTES, &answer.quote_id, &ptr).await.expect("decline the quote");

    let (next, _, _) =
        tap(&f, &Identity::generate().expect("peer"), Acceptance::Instant, 1, now).await;
    let outcome = as_listed_ops::answer_as_listed(&f.host, &next, Trigger::Driver, now)
        .await
        .expect("second answer");
    assert!(matches!(outcome, Outcome::Quoted(_)));
}

#[tokio::test]
async fn one_counterparty_tapping_twice_gets_one_instant_quote() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (first, payload, record_id) = tap(&f, &peer, Acceptance::Instant, 20, now).await;
    as_listed_ops::answer_as_listed(&f.host, &first, Trigger::Driver, now)
        .await
        .expect("first answer");

    let (env, enquiry_payload) =
        enquiry(&peer, &payload.listing_id, consent(&record_id, now + 3_600), 2, now);
    let second = hold(&f, &env, &enquiry_payload, now, &f.master_did).await;
    let outcome = as_listed_ops::answer_as_listed(&f.host, &second, Trigger::Driver, now)
        .await
        .expect("second answer");
    assert!(matches!(outcome, Outcome::Waiting("counterparty-holds-one")));
    let quotes = paging::count(&f.host, QUOTES, None).await.expect("count");
    assert_eq!(quotes, 1, "one tap makes at most one quote");
}

#[tokio::test]
async fn an_answer_that_reached_its_fence_is_resumed_with_the_same_quote_id() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry, payload, _) = tap(&f, &peer, Acceptance::Instant, 20, now).await;
    let quote_id = transaction::derive_quote_id(CONVERSATION, &f.master_did, 1).expect("quote id");
    put_row(
        &f.host,
        LEDGER,
        &answer_id(&enquiry),
        &AnswerRow {
            kind: LedgerKind::Answer,
            enquiry_record_id: enquiry.clone(),
            conversation: CONVERSATION.to_string(),
            listing_id: payload.listing_id.clone(),
            quote_id: quote_id.clone(),
            sequence: 1,
            quote_record_id: None,
            formed_as: FormedAs::AsListedInstant,
            state: AnswerState::Answering,
            code: None,
            created_at_secs: now,
        },
    )
    .await
    .expect("fence");

    let mut warnings = Vec::new();
    as_listed_ops::answer_waiting(&f.host, now, &mut warnings).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let answer: AnswerRow =
        get_row(&f.host, LEDGER, &answer_id(&enquiry)).await.expect("read").expect("answer");
    assert_eq!(answer.state, AnswerState::Answered);
    assert_eq!(answer.quote_id, quote_id);
    assert!(
        get_row::<RecordPointerRow, _>(&f.host, QUOTES, &quote_id).await.expect("read").is_some(),
        "the resume signs the same quote"
    );
}

#[tokio::test]
async fn a_resumed_answer_whose_consent_passed_ends_without_a_card() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry, payload, _) = tap(&f, &peer, Acceptance::Instant, 20, now).await;
    let quote_id = transaction::derive_quote_id(CONVERSATION, &f.master_did, 1).expect("quote id");
    put_row(
        &f.host,
        LEDGER,
        &answer_id(&enquiry),
        &AnswerRow {
            kind: LedgerKind::Answer,
            enquiry_record_id: enquiry.clone(),
            conversation: CONVERSATION.to_string(),
            listing_id: payload.listing_id.clone(),
            quote_id: quote_id.clone(),
            sequence: 1,
            quote_record_id: None,
            formed_as: FormedAs::AsListedInstant,
            state: AnswerState::Answering,
            code: None,
            created_at_secs: now,
        },
    )
    .await
    .expect("fence");

    let later = now + 3_601;
    let mut warnings = Vec::new();
    as_listed_ops::answer_waiting(&f.host, later, &mut warnings).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let answer: AnswerRow =
        get_row(&f.host, LEDGER, &answer_id(&enquiry)).await.expect("read").expect("answer");
    assert_eq!(answer.state, AnswerState::Declined);
    assert_eq!(answer.code.as_deref(), Some("consent-expired"));
    assert!(
        get_row::<RecordPointerRow, _>(&f.host, QUOTES, &quote_id).await.expect("read").is_none(),
        "a passed consent signs no quote"
    );
    assert!(
        get_row::<decline_ops::DeclineRow, _>(&f.host, LEDGER, &decline_ops::decline_id(&enquiry))
            .await
            .expect("read")
            .is_none(),
        "the owner sends no decline card for a tap that ended"
    );

    let mut again = Vec::new();
    as_listed_ops::answer_waiting(&f.host, later, &mut again).await;
    assert!(again.is_empty(), "{again:?}");
}

#[tokio::test]
async fn the_import_checks_answer_rows() {
    let f = fixture();
    let now = f.host.now();
    let peer = Identity::generate().expect("peer");
    let (enquiry_record, _, _) = tap(&f, &peer, Acceptance::Instant, 20, now).await;
    as_listed_ops::answer_as_listed(&f.host, &enquiry_record, Trigger::Driver, now)
        .await
        .expect("answer");

    let bundle =
        backup::export(&f.host, syneroym_roym_core::method::NoParams {}).await.expect("export");
    let mut sections = bundle.sections.clone();
    let rows = sections.get_mut(syneroym_roym_core::backup::SECTION_LEDGER).expect("ledger");
    let row = rows
        .iter_mut()
        .find(|row| row["id"].as_str().is_some_and(|id| id.starts_with("answer:")))
        .expect("answer row");
    row["payload"]["quote_record_id"] = json!("rec_unknown");
    let mut tampered = syneroym_roym_core::backup::Bundle::new(
        f.master_did.clone(),
        crate::app::SCHEMA_VERSION,
        sections,
    )
    .expect("bundle");
    syneroym_roym_core::signing::sign_bundle(&f.host, &mut tampered, now).await.expect("sign");
    let err = backup::import(&f.host, syneroym_roym_core::backup::ImportInput { bundle: tampered })
        .await
        .expect_err("refused");
    let message = err.error.expect("error").message;
    assert!(message.contains("answer:"), "{message}");
}
