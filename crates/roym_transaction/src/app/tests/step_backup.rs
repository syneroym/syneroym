//! Export and import of steps. An import checks every step row as it would
//! check a card that had just arrived.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    deal::SettlementRole,
    envelope::Response,
    method::NoParams,
    record::RECORD_STEP,
    signing,
    step::{MAX_STEP_DATA_BYTES, MAX_STEPS_PER_AGREEMENT},
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    fixture_with_master,
    step::{StepDeal, extension_deal, slot_deal, started},
};
use crate::app::{
    CARDS, CardRow, DISCLOSURES, SCHEMA_VERSION, STEPS, SendState, backup, disclosures, get_row,
    put_row,
    step_ops::{self, StepListInput, StepRow, step_id},
};

async fn exported(deal: &StepDeal) -> Bundle {
    backup::export(&deal.fix.host, NoParams {}).await.expect("export")
}

/// The same sections, edited and signed again by the owner.
async fn edited(
    deal: &StepDeal,
    bundle: Bundle,
    edit: impl FnOnce(&mut BTreeMap<String, Vec<Value>>),
) -> Bundle {
    let mut sections = bundle.sections;
    edit(&mut sections);
    let mut out =
        Bundle::new(deal.fix.master_did.clone(), SCHEMA_VERSION, sections).expect("bundle");
    signing::sign_bundle(&deal.fix.host, &mut out, deal.agreed_at + 100).await.expect("sign");
    out
}

async fn import_error(deal: &StepDeal, bundle: Bundle) -> Response {
    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    backup::import(&target.host, ImportInput { bundle }).await.expect_err("the import is refused")
}

fn message(err: &Response) -> String {
    err.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()
}

/// Stores a step that the counterparty signed, as filing would have, but
/// without filing's checks.
async fn insert_step(
    deal: &StepDeal,
    step_type: &str,
    role: SettlementRole,
    data: Value,
    seed: u64,
) {
    let (envelope, disclosures_list, record_id) =
        deal.counterparty_step(step_type, role, data, seed);
    disclosures::store(&deal.fix.host, &record_id, RECORD_STEP, &disclosures_list, deal.agreed_at)
        .await
        .unwrap();
    let row = StepRow {
        agreement: deal.agreement.clone(),
        commitment_id: "c1".to_string(),
        conversation: "conv_1".to_string(),
        step_type: step_type.to_string(),
        role,
        half: ReceiptHalf {
            record_id,
            envelope,
            issuer: deal.cp_did.clone(),
            issued_at_secs: deal.agreed_at + 30,
        },
        state: SendState::Delivered,
    };
    let key = step_id(&deal.agreement, "c1", step_type, role);
    put_row(&deal.fix.host, STEPS, &key, &row).await.unwrap();
}

fn step_rows(sections: &mut BTreeMap<String, Vec<Value>>) -> &mut Vec<Value> {
    sections.get_mut("steps").expect("a steps section")
}

#[tokio::test]
async fn export_and_import_keep_a_step_with_its_data() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let out = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let own_card = out.message_id.expect("sent");

    let bundle = exported(&deal).await;
    assert_eq!(bundle.sections["steps"].len(), 1);

    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    let imported = backup::import(&target.host, ImportInput { bundle }).await.expect("import");
    assert!(imported.imported);

    let listed = step_ops::list(&target.host, StepListInput { agreement: deal.agreement.clone() })
        .await
        .expect("list after import");
    assert_eq!(listed.steps.len(), 1);
    assert_eq!(listed.steps[0].record_id, out.record_id);
    assert_eq!(listed.steps[0].data, Some(json!({})));
    assert!(listed.steps[0].verified && listed.steps[0].checked);
    assert!(disclosures::load(&target.host, &out.record_id).await.unwrap().is_some());

    let card: CardRow = get_row(&target.host, CARDS, &own_card).await.unwrap().expect("card");
    assert!(card.verified, "the step card verifies after import");
}

#[tokio::test]
async fn imported_step_card_has_its_plain_data() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let out = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let own_card = out.message_id.expect("sent");
    let before: CardRow = get_row(&deal.fix.host, CARDS, &own_card).await.unwrap().unwrap();
    assert_eq!(before.data.as_ref().map(|d| d["data"].clone()), Some(json!({})));

    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    backup::import(&target.host, ImportInput { bundle: exported(&deal).await })
        .await
        .expect("import");
    let after: CardRow = get_row(&target.host, CARDS, &own_card).await.unwrap().unwrap();
    let data = after.data.expect("an imported step card has its plain data");
    assert_eq!(data["step_type"], "scheduling/started");
    assert_eq!(data["data"], json!({}));
    assert!(data.get("_sd").is_none());
}

#[tokio::test]
async fn an_import_row_whose_disclosure_is_missing_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let out = step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let bundle = edited(&deal, exported(&deal).await, |s| {
        let rows = s.get_mut(DISCLOSURES).expect("disclosures");
        rows.retain(|r| r["id"] != json!(out.record_id));
    })
    .await;
    let err = import_error(&deal, bundle).await;
    assert!(message(&err).contains("its disclosure is not in the bundle"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_whose_role_differs_from_its_record_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let bundle = edited(&deal, exported(&deal).await, |s| {
        step_rows(s)[0]["payload"]["role"] = json!("receiver");
    })
    .await;
    let err = import_error(&deal, bundle).await;
    assert!(message(&err).contains("does not match its row"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_whose_conversation_differs_from_its_record_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let bundle = edited(&deal, exported(&deal).await, |s| {
        step_rows(s)[0]["payload"]["conversation"] = json!("conv_other");
    })
    .await;
    let err = import_error(&deal, bundle).await;
    assert!(message(&err).contains("does not match its row"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_whose_issued_time_differs_from_its_envelope_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let bundle = edited(&deal, exported(&deal).await, |s| {
        step_rows(s)[0]["payload"]["half"]["issued_at_secs"] = json!(deal.agreed_at + 999);
    })
    .await;
    let err = import_error(&deal, bundle).await;
    assert!(message(&err).contains("does not match its row"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_under_the_wrong_key_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let bundle = edited(&deal, exported(&deal).await, |s| {
        let key = step_id(&deal.agreement, "c1", "scheduling/started", SettlementRole::Receiver);
        step_rows(s)[0]["id"] = json!(key);
    })
    .await;
    let err = import_error(&deal, bundle).await;
    assert!(message(&err).contains("the key is not made from the record"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_that_names_a_missing_commitment_is_refused() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    step_ops::add_at(&deal.fix.host, started(&deal.agreement), deal.agreed_at + 10)
        .await
        .expect("add");
    let bundle = edited(&deal, exported(&deal).await, |s| {
        step_rows(s)[0]["payload"]["commitment_id"] = json!("c9");
    })
    .await;
    let err = import_error(&deal, bundle).await;
    assert!(message(&err).contains("not in the agreement"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_signed_by_the_wrong_party_is_refused() {
    // The counterparty receives `c1`, so a `giver` record from it is forged.
    let deal = extension_deal("future", DealRole::QuoteAuthor).await;
    insert_step(&deal, "future/forged", SettlementRole::Giver, json!({}), 1).await;
    let err = import_error(&deal, exported(&deal).await).await;
    assert!(message(&err).contains("issuer is not the party its role names"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_row_with_data_over_the_limit_is_refused() {
    let deal = extension_deal("future", DealRole::QuoteAuthor).await;
    let big = json!({ "k": "a".repeat(MAX_STEP_DATA_BYTES) });
    insert_step(&deal, "future/big", SettlementRole::Receiver, big, 1).await;
    let err = import_error(&deal, exported(&deal).await).await;
    assert!(message(&err).contains("step-data-too-large"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_with_too_many_steps_for_one_agreement_is_refused() {
    let deal = extension_deal("future", DealRole::QuoteAuthor).await;
    for n in 0..=MAX_STEPS_PER_AGREEMENT {
        let kind = format!("future/s{n}");
        insert_step(&deal, &kind, SettlementRole::Receiver, json!({}), n as u64).await;
    }
    let err = import_error(&deal, exported(&deal).await).await;
    assert!(message(&err).contains("too many steps"), "{}", message(&err));
}

#[tokio::test]
async fn an_import_keeps_steps_of_an_extension_it_does_not_know() {
    let deal = extension_deal("future", DealRole::QuoteAuthor).await;
    insert_step(&deal, "future/anything", SettlementRole::Receiver, json!({ "k": 1 }), 1).await;
    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    backup::import(&target.host, ImportInput { bundle: exported(&deal).await })
        .await
        .expect("import");
    let listed = step_ops::list(&target.host, StepListInput { agreement: deal.agreement.clone() })
        .await
        .unwrap();
    assert_eq!(listed.steps.len(), 1);
    assert_eq!(listed.steps[0].data, Some(json!({ "k": 1 })));
}
