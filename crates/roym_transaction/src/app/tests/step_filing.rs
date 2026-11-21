//! Filing a step card that arrives from the other party.

use serde_json::json;
use syneroym_roym_core::{
    deal::SettlementRole,
    extension::Registry,
    paging,
    record::RECORD_STEP,
    step::{MAX_STEP_DATA_BYTES, MAX_STEPS_PER_AGREEMENT, STEP_VERSION},
    transaction::DealRole,
};

use super::{
    TestFixture,
    settle::incoming_card_row,
    step::{StepDeal, extension_deal, slot_deal, step_row},
};
use crate::app::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, STEPS, SendState, delete_row, disclosures, get_row,
    put_row,
    step_ops::{self, StepAddInput, StepListInput, StepRow, step_id},
    sync::{
        FileCardResult,
        steps::{Stored, claim_received_row, file_step_card, file_step_card_in},
    },
};

/// An extension that no build ships, whose one step either party may record.
const DEMO_MANIFEST: &str = r#"{
    "id": "demo",
    "version": 1,
    "kind": "workflow",
    "resource_kinds": ["goods-or-service"],
    "details": [{ "schema": "demo/order@1", "fields": [] }],
    "steps": [{ "type": "demo/noted", "by": "either", "fields": [] }]
}"#;

fn demo_registry() -> Registry {
    Registry::parse(&[DEMO_MANIFEST]).expect("the demo manifest loads")
}

async fn file(
    deal: &StepDeal,
    msg: &str,
    envelope: &str,
    disclosures: &[String],
) -> FileCardResult {
    let row = incoming_card_row(
        msg,
        "conv_1",
        RECORD_STEP,
        STEP_VERSION,
        &deal.cp_did,
        deal.agreed_at + 30,
    );
    file_step_card(&deal.fix.host, row, envelope, disclosures, "conv_1", deal.agreed_at + 40)
        .await
        .expect("filing does not fail")
}

async fn card(fix: &TestFixture, msg: &str) -> CardRow {
    get_row(&fix.host, CARDS, msg).await.unwrap().expect("the card row")
}

async fn listed(deal: &StepDeal) -> Vec<step_ops::StepView> {
    step_ops::list(&deal.fix.host, StepListInput { agreement: deal.agreement.clone() })
        .await
        .expect("list")
        .steps
}

#[tokio::test]
async fn a_step_from_the_giver_is_filed_and_listed() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let (env, disc, rec_id) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.filed && !filed.refused && !filed.deferred);

    let c = card(&deal.fix, "msg_s1").await;
    assert!(c.known && c.verified);
    assert_eq!(c.record_id.as_deref(), Some(rec_id.as_str()));
    let data = c.data.expect("plain data");
    assert_eq!(data["data"], json!({}));
    assert!(data.get("_sd").is_none());

    let steps = listed(&deal).await;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].record_id, rec_id);
    assert_eq!(steps[0].role, SettlementRole::Giver);
    assert_eq!(steps[0].issuer, deal.cp_did);
    assert!(steps[0].verified && steps[0].checked);
}

#[tokio::test]
async fn the_same_card_twice_is_a_repeat() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let (env, disc, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 1);
    file(&deal, "msg_s1", &env, &disc).await;
    let again = file(&deal, "msg_s2", &env, &disc).await;
    assert!(again.filed && !again.refused);
    assert!(card(&deal.fix, "msg_s2").await.repeat);
    assert_eq!(listed(&deal).await.len(), 1);
}

#[tokio::test]
async fn a_second_record_for_the_same_key_is_refused_and_stores_no_disclosure() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let (env1, disc1, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 1);
    file(&deal, "msg_s1", &env1, &disc1).await;

    let (env2, disc2, rec2) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 2);
    let second = file(&deal, "msg_s2", &env2, &disc2).await;
    assert!(second.refused);
    assert_eq!(
        card(&deal.fix, "msg_s2").await.reason.as_deref(),
        Some("a second step of this type by this role")
    );
    assert!(disclosures::load(&deal.fix.host, &rec2).await.unwrap().is_none());
    assert_eq!(listed(&deal).await.len(), 1);
}

#[tokio::test]
async fn losing_the_race_to_store_a_received_row_drops_its_disclosure() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let key = step_id(&deal.agreement, "c1", "scheduling/started", SettlementRole::Giver);
    let (_, disc, rec) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 2);
    disclosures::store(&deal.fix.host, &rec, RECORD_STEP, &disc, deal.agreed_at).await.unwrap();
    // A different record took the key after this one stored its disclosure.
    let winner = step_row(&deal, "rec_winner", SendState::Delivered);
    put_row(&deal.fix.host, STEPS, &key, &winner).await.unwrap();

    let loser = step_row(&deal, &rec, SendState::Delivered);
    let stored = claim_received_row(&deal.fix.host, &key, &loser).await.unwrap();
    assert!(matches!(stored, Stored::Refused(_)));
    assert!(disclosures::load(&deal.fix.host, &rec).await.unwrap().is_none());
    assert_eq!(listed(&deal).await.len(), 1);
}

#[tokio::test]
async fn a_card_whose_issuer_does_not_match_its_role_is_refused() {
    let deal = slot_deal(DealRole::Counterparty).await;
    // The counterparty gives `c1`, so a record from it must say `giver`.
    let (env, disc, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Receiver, json!({}), 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.refused);
    assert_eq!(card(&deal.fix, "msg_s1").await.reason.as_deref(), Some("role-mismatch"));
    assert!(listed(&deal).await.is_empty());
}

#[tokio::test]
async fn a_step_the_manifest_does_not_declare_is_refused() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let (env, disc, _) =
        deal.counterparty_step("scheduling/finished", SettlementRole::Giver, json!({}), 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.refused);
    assert_eq!(card(&deal.fix, "msg_s1").await.reason.as_deref(), Some("step: step-type-unknown"));
}

#[tokio::test]
async fn a_step_for_the_other_party_is_refused() {
    // The node under test gives `c1`, so only it may record `started`.
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let (env, disc, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Receiver, json!({}), 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.refused);
    assert_eq!(card(&deal.fix, "msg_s1").await.reason.as_deref(), Some("step: not-your-step"));
}

#[tokio::test]
async fn a_step_that_names_another_extension_is_refused() {
    let deal = extension_deal("future", DealRole::QuoteAuthor).await;
    let (env, disc, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Receiver, json!({}), 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.refused);
    assert_eq!(
        card(&deal.fix, "msg_s1").await.reason.as_deref(),
        Some("the commitment names another extension")
    );
}

#[tokio::test]
async fn a_card_without_its_disclosure_is_refused() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let (env, _, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 1);
    let filed = file(&deal, "msg_s1", &env, &[]).await;
    assert!(filed.refused);
    assert!(card(&deal.fix, "msg_s1").await.reason.unwrap().starts_with("step disclosures"));
}

#[tokio::test]
async fn a_card_for_an_agreement_not_formed_here_waits_and_is_filed_after() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let full: AgreementRow =
        get_row(&deal.fix.host, AGREEMENTS, &deal.agreement).await.unwrap().unwrap();
    let mut partial = full.clone();
    partial.quote_author = None;
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &partial).await.unwrap();

    let (env, disc, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 1);
    let waiting = file(&deal, "msg_s1", &env, &disc).await;
    assert!(waiting.deferred && !waiting.filed);
    assert!(listed_without_check(&deal).await.is_empty());

    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &full).await.unwrap();
    let after = file(&deal, "msg_s1", &env, &disc).await;
    assert!(after.filed && !after.refused);
    assert_eq!(listed(&deal).await.len(), 1);
}

#[tokio::test]
async fn a_card_for_an_agreement_not_yet_held_waits_and_is_filed_after() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let full: AgreementRow =
        get_row(&deal.fix.host, AGREEMENTS, &deal.agreement).await.unwrap().unwrap();
    delete_row(&deal.fix.host, AGREEMENTS, &deal.agreement).await.unwrap();

    let (env, disc, _) =
        deal.counterparty_step("scheduling/started", SettlementRole::Giver, json!({}), 1);
    let waiting = file(&deal, "msg_s1", &env, &disc).await;
    assert!(waiting.deferred && !waiting.filed);
    assert!(listed_without_check(&deal).await.is_empty());

    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &full).await.unwrap();
    let after = file(&deal, "msg_s1", &env, &disc).await;
    assert!(after.filed && !after.refused);
    assert_eq!(listed(&deal).await.len(), 1);
}

/// `step.list` refuses an agreement that is not held; this reads the rows.
async fn listed_without_check(deal: &StepDeal) -> Vec<StepRow> {
    paging::query_all(
        &deal.fix.host,
        STEPS,
        Some(json!({ "agreement": deal.agreement }).to_string()),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_step_of_an_unknown_extension_is_filed_and_not_checked() {
    let deal = extension_deal("future", DealRole::Counterparty).await;
    let (env, disc, _) =
        deal.counterparty_step("future/anything", SettlementRole::Giver, json!({ "k": [1, 2] }), 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.filed && !filed.refused);

    let steps = listed(&deal).await;
    assert_eq!(steps.len(), 1);
    assert!(steps[0].verified);
    assert!(!steps[0].checked, "this node cannot check a step of an unknown extension");
    assert_eq!(steps[0].data, Some(json!({ "k": [1, 2] })));
}

#[tokio::test]
async fn data_over_the_size_limit_is_refused_when_filed() {
    let deal = extension_deal("future", DealRole::Counterparty).await;
    let big = json!({ "k": "a".repeat(MAX_STEP_DATA_BYTES) });
    let (env, disc, _) = deal.counterparty_step("future/big", SettlementRole::Giver, big, 1);
    let filed = file(&deal, "msg_s1", &env, &disc).await;
    assert!(filed.refused);
    assert_eq!(
        card(&deal.fix, "msg_s1").await.reason.as_deref(),
        Some("step: step-data-too-large")
    );
}

#[tokio::test]
async fn an_agreement_holds_at_most_the_step_limit() {
    let deal = extension_deal("future", DealRole::Counterparty).await;
    for n in 0..MAX_STEPS_PER_AGREEMENT {
        let kind = format!("future/s{n}");
        let (env, disc, _) =
            deal.counterparty_step(&kind, SettlementRole::Giver, json!({}), n as u64);
        let filed = file(&deal, &format!("msg_{n}"), &env, &disc).await;
        assert!(filed.filed && !filed.refused, "step {n} is filed");
    }
    let (env, disc, rec) =
        deal.counterparty_step("future/one-more", SettlementRole::Giver, json!({}), 99);
    let over = file(&deal, "msg_over", &env, &disc).await;
    assert!(over.refused);
    assert_eq!(card(&deal.fix, "msg_over").await.reason.as_deref(), Some("too-many-steps"));
    assert!(disclosures::load(&deal.fix.host, &rec).await.unwrap().is_none());

    // A card that is already stored is a repeat, not a 65th step.
    let (env0, disc0, _) = deal.counterparty_step("future/s0", SettlementRole::Giver, json!({}), 0);
    let repeat = file(&deal, "msg_again", &env0, &disc0).await;
    assert!(repeat.filed && !repeat.refused);
    assert!(card(&deal.fix, "msg_again").await.repeat);
}

#[tokio::test]
async fn either_party_may_record_a_step_that_declares_either() {
    let registry = demo_registry();
    let deal = extension_deal("demo", DealRole::QuoteAuthor).await;
    let input = || StepAddInput {
        agreement: deal.agreement.clone(),
        commitment_id: "c1".to_string(),
        step_type: "demo/noted".to_string(),
        data: json!({}),
    };

    let mine = step_ops::add_in(&deal.fix.host, input(), deal.agreed_at + 10, &registry)
        .await
        .expect("the giver records the step");
    let again = step_ops::add_in(&deal.fix.host, input(), deal.agreed_at + 20, &registry)
        .await
        .expect("a repeat");
    assert_eq!(again.record_id, mine.record_id);

    let (env, disc, rec) =
        deal.counterparty_step("demo/noted", SettlementRole::Receiver, json!({}), 1);
    let row = incoming_card_row(
        "msg_s1",
        "conv_1",
        RECORD_STEP,
        STEP_VERSION,
        &deal.cp_did,
        deal.agreed_at + 30,
    );
    let filed = file_step_card_in(
        &registry,
        &deal.fix.host,
        row,
        &env,
        &disc,
        "conv_1",
        deal.agreed_at + 40,
    )
    .await
    .expect("file");
    assert!(filed.filed && !filed.refused);

    let giver_key = step_id(&deal.agreement, "c1", "demo/noted", SettlementRole::Giver);
    let receiver_key = step_id(&deal.agreement, "c1", "demo/noted", SettlementRole::Receiver);
    let giver: StepRow = get_row(&deal.fix.host, STEPS, &giver_key).await.unwrap().unwrap();
    let receiver: StepRow = get_row(&deal.fix.host, STEPS, &receiver_key).await.unwrap().unwrap();
    assert_eq!(giver.half.record_id, mine.record_id);
    assert_eq!(receiver.half.record_id, rec);
}
