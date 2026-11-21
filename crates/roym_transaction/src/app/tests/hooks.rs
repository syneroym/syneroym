//! The deal core's calls to an extension's reserve hooks: which service they
//! go to, what a refusal does, and the calls that must not repeat.

use serde_json::{Value, json};
use syneroym_identity::Identity;
use syneroym_roym_core::{
    deal::{
        CommitmentKind, Details, ExtensionRef, LocalText, ResourceKind, sd::reveal_terms_strict,
    },
    lang::LangTag,
    record::RECORD_QUOTE,
    transaction::{DealRole, TimeWindow, verify_quote},
};

use super::{
    TestFixture,
    accept::{author_service_commitment, build_and_sign_quote, slot_commitment},
    disclosure::helpers::{build_and_sign_enquiry, sample_terms_input},
    fixture,
};
use crate::app::{
    ENQUIRY_HISTORY, LEDGER,
    agreement_ops::{self, AgreementAcceptInput, QuoteRecordInput},
    disclosures,
    hooks::released_id,
    ledger::{LedgerKind, LedgerRow},
    progress_ops, put_bytes,
    quote_ops::{self, CommitmentInput, QuoteMeasureInput, QuoteSetInput, store_received_quote},
};

/// A quote from another node that this node holds, with one commitment that
/// this node (the counterparty) gives.
async fn stored_quote(
    f: &TestFixture,
    commitment: syneroym_roym_core::deal::Commitment,
    expires_in: u64,
) -> (String, u64) {
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (quote, record_id, envelope) =
        build_and_sign_quote(&author, &f.master_did, vec![commitment], now + expires_in, now);
    let verdict = verify_quote(&envelope, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope, &verdict, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &record_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");
    (record_id, now)
}

fn accept_input(record_id: &str) -> AgreementAcceptInput {
    AgreementAcceptInput {
        quote_record_id: record_id.to_string(),
        viewed_lang: None,
        viewed_via_mt: None,
    }
}

#[tokio::test]
async fn an_extension_reason_that_is_not_a_code_is_an_error() {
    let long = "a".repeat(65);
    for bad in [long.as_str(), "Slot-Taken", "slot taken", ""] {
        let f = fixture();
        f.scheduling.refuse_reserves(bad);
        let (comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
        let (record_id, now) = stored_quote(&f, comm, 3600).await;
        let err = agreement_ops::accept_at(&f.host, accept_input(&record_id), now)
            .await
            .expect_err("a reason that is not a code must not be used");
        assert_eq!(err.error.as_ref().unwrap().code, -32603, "{bad:?}: {err:?}");
        assert!(!f.scheduling.is_held(&record_id, "c1"));
    }

    let f = fixture();
    f.scheduling.refuse_reserves("slot-gone-64");
    let (comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (record_id, now) = stored_quote(&f, comm, 3600).await;
    let err = agreement_ops::accept_at(&f.host, accept_input(&record_id), now)
        .await
        .expect_err("a refused reserve");
    let error = err.error.unwrap();
    assert_eq!(error.message, "reserve-failed");
    assert_eq!(error.data.unwrap()["reason"], "slot-gone-64");
}

#[tokio::test]
async fn a_refused_second_reserve_releases_the_first() {
    let f = fixture();
    let (c1, slot1) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (c2, slot2) = slot_commitment("c2", "listing_123", 2_000_000, 2_003_600);
    f.scheduling.take_seat(&slot2, "agr_earlier_winner");
    let (record_id, now) = stored_quote(&f, c1, 3600).await;
    // The quote holds one commitment: take both from a two-commitment quote.
    let author = Identity::generate().expect("generate author");
    let (quote, record_id2, envelope) = build_and_sign_quote(
        &author,
        &f.master_did,
        vec![slot_commitment("c1", "listing_123", 1_000_000, 1_003_600).0, c2],
        now + 3600,
        now,
    );
    let verdict = verify_quote(&envelope, now);
    store_received_quote(&f.host, &envelope, &verdict, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &record_id2, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");
    let _ = record_id;

    let err = agreement_ops::accept_at(&f.host, accept_input(&record_id2), now)
        .await
        .expect_err("the second slot is taken");
    assert_eq!(err.error.as_ref().unwrap().message, "reserve-failed");
    assert!(f.scheduling.is_released(&record_id2, "c1"), "the first reserve is released");
    assert!(!f.scheduling.is_held(&record_id2, "c2"));
    assert_eq!(f.scheduling.held_seats(&slot1), 0);
}

fn one_shot_given_here() -> syneroym_roym_core::deal::Commitment {
    let mut c = author_service_commitment("c1");
    c.giver = DealRole::Counterparty;
    c.extension = Some(ExtensionRef { id: "one-shot".to_string(), version: 1 });
    c.details = Some(Details {
        extension: "one-shot".to_string(),
        schema: "one-shot/order@1".to_string(),
        data: json!({}),
        extra: vec![],
    });
    c
}

#[tokio::test]
async fn a_commitment_with_no_reserve_hook_calls_no_hook() {
    let f = fixture();
    let (record_id, now) = stored_quote(&f, one_shot_given_here(), 3600).await;
    agreement_ops::accept_at(&f.host, accept_input(&record_id), now)
        .await
        .expect("a one-shot commitment needs no reservation");
    assert!(f.scheduling.call_log().is_empty(), "no hook call: {:?}", f.scheduling.call_log());
}

#[tokio::test]
async fn the_hooks_go_to_the_service_the_manifest_names() {
    let f = fixture();
    let (comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (record_id, now) = stored_quote(&f, comm, 3600).await;
    // The fake answers only as the service named `scheduling`: a call to any
    // other service would be an error, and the accept would fail.
    agreement_ops::accept_at(&f.host, accept_input(&record_id), now).await.expect("accept");
    let methods: Vec<String> = f.scheduling.call_log().into_iter().map(|c| c.method).collect();
    assert_eq!(methods, ["reservation.reserve"]);
}

fn slot_terms_input(commitment_window: Option<TimeWindow>) -> quote_ops::DealTermsInput {
    let (slot, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let mut terms = sample_terms_input("Slot lesson");
    terms.commitments = vec![CommitmentInput {
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Lesson"),
        declared_value_minor: 5000,
        details: slot.details,
        window: commitment_window,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }];
    terms
}

async fn author_with_enquiry() -> (TestFixture, String, u64) {
    let author = fixture();
    let cp = fixture();
    let now = author.host.now();
    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp.master, now);
    put_bytes(&author.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    (author, enq_rec, now)
}

#[tokio::test]
async fn quote_set_fills_the_window_from_the_prepare_hook() {
    let (author, enq_rec, now) = author_with_enquiry().await;
    let set = quote_ops::set_at(
        &author.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec,
            quote_id: None,
            expires_in_secs: 3600,
            terms: slot_terms_input(None),
        },
        now,
    )
    .await
    .expect("quote.set");
    let held = disclosures::load_quote(&author.host, &set.record_id, now).await.expect("held");
    let terms = reveal_terms_strict(&held.signed.terms, &held.disclosures).expect("reveal");
    let window = terms.commitments[0].window;
    assert_eq!(window, Some(TimeWindow { earliest_secs: 1_000_000, latest_secs: 1_003_600 }));
    assert_eq!(author.scheduling.calls("reservation.prepare"), 1);
}

fn refusal_code(err: &syneroym_roym_core::envelope::Response) -> Value {
    err.error.as_ref().and_then(|e| e.data.clone()).map_or(Value::Null, |d| d["code"].clone())
}

#[tokio::test]
async fn quote_measure_refuses_the_same_codes_as_quote_set() {
    let wrong_window = TimeWindow { earliest_secs: 1_000_000, latest_secs: 1_009_000 };
    let cases: [(&str, Option<TimeWindow>, Option<&str>); 3] = [
        ("no-such-slot", None, Some("remove")),
        ("slot-not-in-listing", None, Some("other-listing")),
        ("schedule-differs-from-slot", Some(wrong_window), None),
    ];
    for (code, window, change) in cases {
        let (author, enq_rec, now) = author_with_enquiry().await;
        let mut terms = slot_terms_input(window);
        match change {
            Some("remove") => {
                let slot = terms.commitments[0].details.as_ref().unwrap().data["slot_id"]
                    .as_str()
                    .unwrap()
                    .to_string();
                author.scheduling.remove_slot(&slot);
            }
            Some(_) => terms.listing_id = Some("other-listing".to_string()),
            None => {}
        }
        let set = quote_ops::set_at(
            &author.host,
            QuoteSetInput {
                enquiry_record_id: enq_rec,
                quote_id: None,
                expires_in_secs: 3600,
                terms: terms.clone(),
            },
            now,
        )
        .await
        .expect_err("quote.set refuses");
        let measure = quote_ops::measure_at(&author.host, QuoteMeasureInput { terms }, now)
            .await
            .expect_err("quote.measure refuses");
        assert_eq!(refusal_code(&set), json!(code), "quote.set");
        assert_eq!(refusal_code(&measure), json!(code), "quote.measure");
        assert_eq!(set.error.unwrap().message, code, "the text of today's refusal is kept");
    }
}

#[tokio::test]
async fn the_expiry_release_runs_once_per_agreement() {
    let f = fixture();
    let (comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (record_id, now) = stored_quote(&f, comm, 300).await;
    agreement_ops::accept_at(&f.host, accept_input(&record_id), now).await.expect("accept");

    let get = || QuoteRecordInput { quote_record_id: record_id.clone() };
    agreement_ops::get_at(&f.host, get(), now + 350).await.expect("get after expiry");
    assert_eq!(f.scheduling.calls("reservation.release"), 1);
    let marker: Option<LedgerRow> =
        crate::app::get_row(&f.host, LEDGER, &released_id(&record_id)).await.unwrap();
    assert_eq!(marker.expect("the marker row").kind, LedgerKind::Released);

    agreement_ops::get_at(&f.host, get(), now + 360).await.expect("second get");
    progress_ops::tick_conversation_at(&f.host, "conv_1", now + 370).await.expect("tick");
    assert_eq!(f.scheduling.calls("reservation.release"), 1, "no later call releases again");
}

#[tokio::test]
async fn a_failed_expiry_release_is_tried_again() {
    let f = fixture();
    let (comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (record_id, now) = stored_quote(&f, comm, 300).await;
    agreement_ops::accept_at(&f.host, accept_input(&record_id), now).await.expect("accept");
    f.scheduling.fail_call("reservation.release", 0);

    let get = || QuoteRecordInput { quote_record_id: record_id.clone() };
    agreement_ops::get_at(&f.host, get(), now + 350).await.expect("first get");
    let marker: Option<LedgerRow> =
        crate::app::get_row(&f.host, LEDGER, &released_id(&record_id)).await.unwrap();
    assert!(marker.is_none(), "a failed release writes no marker");
    assert!(f.scheduling.is_held(&record_id, "c1"));

    agreement_ops::get_at(&f.host, get(), now + 360).await.expect("second get");
    assert!(f.scheduling.is_released(&record_id, "c1"));
    let marker: Option<LedgerRow> =
        crate::app::get_row(&f.host, LEDGER, &released_id(&record_id)).await.unwrap();
    assert!(marker.is_some());
}

#[tokio::test]
async fn a_transport_error_on_reserve_is_an_error_and_not_a_refusal() {
    let f = fixture();
    f.scheduling.fail_call("reservation.reserve", 0);
    let (comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (record_id, now) = stored_quote(&f, comm, 3600).await;
    let err = agreement_ops::accept_at(&f.host, accept_input(&record_id), now)
        .await
        .expect_err("a transport error");
    assert_eq!(err.error.as_ref().unwrap().code, -32603);
    assert!(f.scheduling.failure_hit());
}
