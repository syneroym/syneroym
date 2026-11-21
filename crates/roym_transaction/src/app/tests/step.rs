//! `step.add`, `step.list` and `step.verify`: who may record a step, what
//! makes the second call a repeat, and what the stored step shows.

use std::{
    future::Future,
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

use serde_json::{Value, json};
use syneroym_app_host::types::proxy::ProxyError;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    deal::{Details, ExtensionRef, SettlementRole, transition::CommitmentState},
    envelope::Response,
    method::DisclosedVerifyInput,
    record::{Envelope, RECORD_STEP, RecordDraft},
    step::{MAX_STEP_DATA_BYTES, MAX_STEPS_PER_AGREEMENT, STEP_VERSION},
    testing::sd::{PlainStep, conceal_step},
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    TestFixture, fixture,
    refuse::helpers::slot_commitment,
    settle::{make_currency_commitment, make_service_commitment, setup_agreed_deal},
};
use crate::app::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, STEPS, SendState, disclosures, get_row, put_row,
    step_ops::{self, StepAddInput},
};

pub(super) struct StepDeal {
    pub(super) fix: TestFixture,
    pub(super) counterparty: Identity,
    pub(super) cp_did: String,
    pub(super) agreement: String,
    pub(super) agreed_at: u64,
}

/// A formed agreement. `c1` is a slot booking given by `slot_giver`, and
/// `c2` is a payment with no extension. The node under test is the quote
/// author.
pub(super) async fn slot_deal(slot_giver: DealRole) -> StepDeal {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (mut slot, _) = slot_commitment("c1", "listing_1", agreed_at + 1000, agreed_at + 4600);
    slot.giver = slot_giver;
    let payment = make_currency_commitment("c2", &fix.master_did);
    let (agreement, _) =
        setup_agreed_deal(&fix, &counterparty, vec![slot, payment], agreed_at).await;
    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    StepDeal { fix, counterparty, cp_did, agreement, agreed_at }
}

/// A formed agreement whose `c1` is a plain service that names `extension`
/// (version 1), given by the node under test. With an id that this build does
/// not ship, the node cannot check the extension's steps.
pub(super) async fn extension_deal(extension: &str, giver: DealRole) -> StepDeal {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let mut service = make_service_commitment("c1");
    service.giver = giver;
    service.extension = Some(ExtensionRef { id: extension.to_string(), version: 1 });
    service.details = Some(Details {
        extension: extension.to_string(),
        schema: format!("{extension}/order@1"),
        data: json!({}),
        extra: vec![],
    });
    let payment = make_currency_commitment("c2", &fix.master_did);
    let (agreement, _) =
        setup_agreed_deal(&fix, &counterparty, vec![service, payment], agreed_at).await;
    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    StepDeal { fix, counterparty, cp_did, agreement, agreed_at }
}

impl StepDeal {
    /// A step record that the counterparty signs, with its disclosures. `seed`
    /// makes the salt, so two seeds give two different records of one step.
    pub(super) fn counterparty_step(
        &self,
        step_type: &str,
        role: SettlementRole,
        data: Value,
        seed: u64,
    ) -> (String, Vec<String>, String) {
        let extension = step_type.split('/').next().unwrap_or_default();
        let (payload, disclosures) = conceal_step(
            PlainStep {
                agreement: self.agreement.clone(),
                conversation: "conv_1".to_string(),
                commitment_id: "c1".to_string(),
                counterparty_did: self.cp_did.clone(),
                quote_author_did: self.fix.master_did.clone(),
                role,
                extension: ExtensionRef { id: extension.to_string(), version: 1 },
                step_type: step_type.to_string(),
                data,
            },
            seed,
        );
        let draft = RecordDraft {
            version: STEP_VERSION,
            record_type: RECORD_STEP.to_string(),
            subject: self.agreement.clone(),
            payload: serde_json::to_value(&payload).expect("payload to value"),
            expires_at_secs: None,
            supersedes: None,
        };
        let (mut env, bytes) =
            Envelope::unsigned(draft, self.cp_did.clone(), None, self.agreed_at + 30)
                .expect("draft step");
        env.attach_signature(z32::encode(&self.counterparty.sign(&bytes).to_bytes()))
            .expect("attach signature");
        let record_id = env.record_id().expect("record id");
        (env.to_json().expect("envelope json"), disclosures, record_id)
    }
}

pub(super) fn started(agreement: &str) -> StepAddInput {
    StepAddInput {
        agreement: agreement.to_string(),
        commitment_id: "c1".to_string(),
        step_type: "scheduling/started".to_string(),
        data: json!({}),
    }
}

fn code_of(err: &Response) -> Value {
    err.error.as_ref().and_then(|e| e.data.clone()).unwrap_or(Value::Null)
}

fn message_of(err: &Response) -> String {
    err.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()
}

async fn edit_agreement(deal: &StepDeal, edit: impl FnOnce(&mut AgreementRow)) {
    let mut row: AgreementRow =
        get_row(&deal.fix.host, AGREEMENTS, &deal.agreement).await.unwrap().unwrap();
    edit(&mut row);
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &row).await.unwrap();
}

#[tokio::test]
async fn the_giver_adds_a_step_and_it_is_listed() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let out = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("giver adds a step");
    assert_eq!(out.role, DealRole::QuoteAuthor);
    assert_ne!(out.state, SendState::AlreadyRecorded);

    let listed = step_ops::list(
        &deal.fix.host,
        step_ops::StepListInput { agreement: deal.agreement.clone() },
    )
    .await
    .expect("list");
    assert_eq!(listed.steps.len(), 1);
    let step = &listed.steps[0];
    assert_eq!(step.record_id, out.record_id);
    assert_eq!(step.step_type, "scheduling/started");
    assert_eq!(step.extension.id, "scheduling");
    assert_eq!(step.role, SettlementRole::Giver);
    assert_eq!(step.issuer, deal.fix.master_did);
    assert_eq!(step.data, Some(json!({})));
    assert!(step.verified && step.checked);
}

#[tokio::test]
async fn the_own_card_shows_the_plain_data() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let out = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let message_id = out.message_id.expect("the card was sent");
    let card: CardRow = get_row(&deal.fix.host, CARDS, &message_id).await.unwrap().unwrap();
    assert_eq!(card.card_type, "step");
    let data = card.data.expect("plain data");
    assert_eq!(data["step_type"], "scheduling/started");
    assert_eq!(data["data"], json!({}));
    assert!(data.get("_sd").is_none());
}

#[tokio::test]
async fn a_repeat_returns_the_stored_record() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let first = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("first");
    let again = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 20)
        .await
        .expect("repeat");
    assert_eq!(again.record_id, first.record_id);
    assert_eq!(again.state, SendState::AlreadyRecorded);
    assert_eq!(again.message_id, None);
}

#[tokio::test]
async fn a_step_whose_card_was_never_sent_is_sent_by_the_repeat() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    deal.fix
        .host
        .on_call("conversation", "send", |_| Err(ProxyError::Transport("offline".to_string())));
    let first = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("the record is stored even when the send fails");
    assert_eq!(first.state, SendState::NotSent);

    deal.fix.host.on_call("conversation", "send", |_| {
        Ok(json!({ "message_id": "m1", "state": "deposited", "sender_timestamp_ms": 1 })
            .to_string())
    });
    let again = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 20)
        .await
        .expect("repeat");
    assert_eq!(again.record_id, first.record_id, "no second record is signed");
    assert_eq!(again.state, SendState::Deposited);
    assert_eq!(again.message_id.as_deref(), Some("m1"));
}

fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = Waker::noop();
    match pin!(fut).poll(&mut Context::from_waker(waker)) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("test future pended unexpectedly"),
    }
}

#[tokio::test]
async fn concurrent_add_sends_only_one_card_and_leaves_no_orphan_disclosures() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let host = deal.fix.host.clone();
    let agr = deal.agreement.clone();
    let agreed_at = deal.agreed_at;
    let second_done = Arc::new(AtomicBool::new(false));
    let second_done_clone = Arc::clone(&second_done);
    let second_out = Arc::new(Mutex::new(None));
    let second_out_clone = Arc::clone(&second_out);

    deal.fix.host.on_call("conversation", "send", move |_| {
        if !second_done_clone.swap(true, Ordering::SeqCst) {
            let h = host.clone();
            let inp = started(&agr);
            let out = block_on(step_ops::add_at(&h, inp, agreed_at + 15));
            *second_out_clone.lock().unwrap() = Some(out);
        }
        Ok(json!({ "message_id": "m1", "state": "deposited", "sender_timestamp_ms": 1000 })
            .to_string())
    });

    let first = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("first add succeeds");
    assert_ne!(first.state, SendState::AlreadyRecorded);
    assert_eq!(first.message_id.as_deref(), Some("m1"));

    let second =
        second_out.lock().unwrap().take().expect("second call ran").expect("second call succeeds");
    assert_eq!(second.record_id, first.record_id);
    assert_eq!(second.state, SendState::AlreadyRecorded);
    assert_eq!(second.message_id, None);
}

#[tokio::test]
async fn the_receiver_may_not_add_a_giver_step() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let err = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect_err("the node under test only receives c1");
    assert_eq!(code_of(&err), json!({ "code": "not-your-step" }));
}

#[tokio::test]
async fn a_step_type_the_manifest_does_not_declare_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let mut input = started(&deal.agreement);
    input.step_type = "scheduling/finished".to_string();
    let err = step_ops::add_at(&deal.fix.host, input, deal.agreed_at + 10).await.unwrap_err();
    assert_eq!(code_of(&err), json!({ "code": "step-type-unknown" }));
}

#[tokio::test]
async fn data_that_fails_the_step_fields_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let mut input = started(&deal.agreement);
    input.data = json!({ "colour": "red" });
    let err = step_ops::add_at(&deal.fix.host, input, deal.agreed_at + 10).await.unwrap_err();
    assert_eq!(code_of(&err), json!({ "code": "details-invalid" }));
}

#[tokio::test]
async fn data_over_the_size_limit_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let mut input = started(&deal.agreement);
    input.data = json!({ "k": "a".repeat(MAX_STEP_DATA_BYTES) });
    let err = step_ops::add_at(&deal.fix.host, input, deal.agreed_at + 10).await.unwrap_err();
    assert_eq!(code_of(&err), json!({ "code": "step-data-too-large" }));
}

#[tokio::test]
async fn a_commitment_with_no_extension_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let mut input = started(&deal.agreement);
    input.commitment_id = "c2".to_string();
    let err = step_ops::add_at(&deal.fix.host, input, deal.agreed_at + 10).await.unwrap_err();
    assert_eq!(message_of(&err), "commitment-has-no-extension");
}

#[tokio::test]
async fn a_commitment_that_does_not_exist_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let mut input = started(&deal.agreement);
    input.commitment_id = "c9".to_string();
    let err = step_ops::add_at(&deal.fix.host, input, deal.agreed_at + 10).await.unwrap_err();
    assert_eq!(message_of(&err), "no-such-commitment");
}

#[tokio::test]
async fn a_final_commitment_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    edit_agreement(&deal, |row| {
        for c in &mut row.commitments {
            if c.commitment_id == "c1" {
                c.state = CommitmentState::Fulfilled;
            }
        }
    })
    .await;
    let err = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .unwrap_err();
    assert_eq!(message_of(&err), "commitment-final");
}

#[tokio::test]
async fn an_agreement_that_is_not_formed_here_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    edit_agreement(&deal, |row| row.counterparty = None).await;
    let err = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .unwrap_err();
    assert_eq!(message_of(&err), "agreement-incomplete");
}

#[tokio::test]
async fn an_agreement_that_is_not_held_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let err = step_ops::add_at(&deal.fix.host, started("rec_unknown"), deal.agreed_at + 10)
        .await
        .unwrap_err();
    assert_eq!(message_of(&err), "no-such-agreement");
}

#[tokio::test]
async fn verify_shows_the_data_with_its_disclosure() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let out = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let listed = step_ops::list(
        &deal.fix.host,
        step_ops::StepListInput { agreement: deal.agreement.clone() },
    )
    .await
    .unwrap();
    assert_eq!(listed.steps[0].record_id, out.record_id);

    let row: step_ops::StepRow = get_row(
        &deal.fix.host,
        STEPS,
        &step_ops::step_id(&deal.agreement, "c1", "scheduling/started", SettlementRole::Giver),
    )
    .await
    .unwrap()
    .expect("the step row");
    let stored =
        disclosures::load(&deal.fix.host, &out.record_id).await.unwrap().expect("disclosure");

    let shown = step_ops::verify(
        &deal.fix.host,
        DisclosedVerifyInput { envelope: row.half.envelope.clone(), disclosures: stored },
    )
    .await
    .expect("verify");
    assert!(shown.verdict.verified);
    assert_eq!(shown.shown.as_ref().map(|v| v["data"].clone()), Some(json!({})));
    assert!(shown.hidden.is_empty());

    let hidden = step_ops::verify(
        &deal.fix.host,
        DisclosedVerifyInput { envelope: row.half.envelope, disclosures: Vec::new() },
    )
    .await
    .expect("verify without disclosures");
    assert!(hidden.verdict.verified);
    assert_eq!(hidden.disclosure_refused, None);
    assert!(hidden.shown.is_some_and(|v| v.get("data").is_none()));
    assert_eq!(hidden.hidden, vec!["data".to_string()]);
}

#[tokio::test]
async fn the_receiver_of_an_unknown_extension_commitment_is_refused() {
    let deal = extension_deal("future", DealRole::Counterparty).await;
    let mut input = started(&deal.agreement);
    input.step_type = "future/received".to_string();
    let err = step_ops::add_at(&deal.fix.host, input, deal.agreed_at + 10).await.unwrap_err();
    assert_eq!(code_of(&err), json!({ "code": "extension-unknown" }));
}

#[tokio::test]
async fn adding_a_step_beyond_the_limit_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    for n in 0..MAX_STEPS_PER_AGREEMENT {
        let dummy = step_ops::StepRow {
            agreement: deal.agreement.clone(),
            commitment_id: "c1".to_string(),
            conversation: "conv_1".to_string(),
            step_type: format!("future/s{n}"),
            role: SettlementRole::Giver,
            half: ReceiptHalf {
                record_id: format!("rec_{n}"),
                envelope: "{}".to_string(),
                issuer: deal.cp_did.clone(),
                issued_at_secs: deal.agreed_at,
            },
            state: SendState::Delivered,
        };
        let key = step_ops::step_id(
            &deal.agreement,
            "c1",
            &format!("future/s{n}"),
            SettlementRole::Giver,
        );
        put_row(&deal.fix.host, STEPS, &key, &dummy).await.unwrap();
    }
    let err = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), json!({ "code": "too-many-steps" }));
}
