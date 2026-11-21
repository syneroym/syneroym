//! Export and import of the rows that reservation upkeep leaves in the
//! ledger: the failure, the confirm marker and the release marker.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use syneroym_identity::Identity;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    envelope::Response,
    method::NoParams,
    record::Envelope,
    signing,
};

use super::{
    commitment_failure_writer::{
        card, counterparty_failure, failure_envelope_at, file, three_part_deal,
    },
    fixture_with_master,
    step::StepDeal,
};
use crate::app::{
    CARDS, LEDGER, SCHEMA_VERSION, backup, get_row,
    hooks::released_commitment_id,
    ledger::{FailureRow, LedgerKind, LedgerRow, confirmed_id, failure_id},
    put_row,
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

async fn import_error(deal: &StepDeal, bundle: Bundle) -> String {
    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    let refused: Response =
        backup::import(&target.host, ImportInput { bundle }).await.expect_err("refused");
    refused.error.map(|e| e.message).unwrap_or_default()
}

fn ledger_row<'a>(sections: &'a mut BTreeMap<String, Vec<Value>>, id: &str) -> &'a mut Value {
    sections
        .get_mut("ledger")
        .expect("a ledger section")
        .iter_mut()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("ledger row '{id}'"))
}

async fn marker(deal: &StepDeal, id: &str, kind: LedgerKind, cid: &str) {
    let row = LedgerRow {
        kind,
        agreement: deal.agreement.clone(),
        state: None,
        quote_id: None,
        commitment_id: Some(cid.to_string()),
        reason: None,
        half: None,
        created_at_secs: deal.agreed_at + 50,
    };
    put_row(&deal.fix.host, LEDGER, id, &row).await.unwrap();
}

#[tokio::test]
async fn export_and_import_keep_the_failure_and_its_card_verified() {
    let deal = three_part_deal().await;
    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    let bundle = exported(&deal).await;

    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    backup::import(&target.host, ImportInput { bundle }).await.expect("import");

    let key = failure_id(&deal.agreement, "c1");
    let row: FailureRow = get_row(&target.host, LEDGER, &key).await.unwrap().expect("failure row");
    assert_eq!(row.commitment_id, "c1");
    assert!(card(&target, "msg_f1").await.verified, "the failure card verifies after import");
    assert!(get_row::<Value, _>(&target.host, CARDS, "msg_f1").await.unwrap().is_some());
}

#[tokio::test]
async fn the_markers_of_reservation_upkeep_survive_an_import() {
    let deal = three_part_deal().await;
    let confirmed = confirmed_id(&deal.agreement, "c1");
    let released = released_commitment_id(&deal.agreement, "c2");
    marker(&deal, &confirmed, LedgerKind::Confirmed, "c1").await;
    marker(&deal, &released, LedgerKind::Released, "c2").await;

    let bundle = exported(&deal).await;
    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    backup::import(&target.host, ImportInput { bundle }).await.expect("import");
    for id in [&confirmed, &released] {
        assert!(get_row::<LedgerRow, _>(&target.host, LEDGER, id).await.unwrap().is_some(), "{id}");
    }
}

#[tokio::test]
async fn a_failure_row_whose_signer_is_not_the_giver_is_refused() {
    let deal = three_part_deal().await;
    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    let bundle = exported(&deal).await;
    let stranger = Identity::generate().unwrap();
    let forged = failure_envelope_at(&deal, "c1", &stranger, deal.agreed_at + 30);
    let record_id = Envelope::from_json(&forged).unwrap().record_id().unwrap();

    let id = failure_id(&deal.agreement, "c1");
    let bundle = edited(&deal, bundle, |sections| {
        let row = ledger_row(sections, &id);
        row["payload"]["envelope"] = json!(forged);
        row["payload"]["record_id"] = json!(record_id);
    })
    .await;
    let message = import_error(&deal, bundle).await;
    assert!(message.contains("issuer is not the giver"), "{message}");
}

#[tokio::test]
async fn a_failure_row_that_does_not_match_its_record_is_refused() {
    let deal = three_part_deal().await;
    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    let bundle = exported(&deal).await;
    let id = failure_id(&deal.agreement, "c1");
    let bundle = edited(&deal, bundle, |sections| {
        ledger_row(sections, &id)["payload"]["record_id"] = json!("rec_other");
    })
    .await;
    let message = import_error(&deal, bundle).await;
    assert!(message.contains("record id mismatch"), "{message}");
}

#[tokio::test]
async fn a_marker_for_an_unknown_commitment_or_agreement_is_refused() {
    let deal = three_part_deal().await;
    let confirmed = confirmed_id(&deal.agreement, "c9");
    marker(&deal, &confirmed, LedgerKind::Confirmed, "c9").await;
    let message = import_error(&deal, exported(&deal).await).await;
    assert!(message.contains("unknown commitment id 'c9'"), "{message}");

    let deal = three_part_deal().await;
    let released = released_commitment_id("rec_nowhere", "c1");
    let row = LedgerRow {
        kind: LedgerKind::Released,
        agreement: "rec_nowhere".to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some("c1".to_string()),
        reason: None,
        half: None,
        created_at_secs: deal.agreed_at,
    };
    put_row(&deal.fix.host, LEDGER, &released, &row).await.unwrap();
    let message = import_error(&deal, exported(&deal).await).await;
    assert!(message.contains("unheld agreement"), "{message}");
}

#[tokio::test]
async fn a_marker_whose_row_does_not_match_its_id_is_refused() {
    let deal = three_part_deal().await;
    let confirmed = confirmed_id(&deal.agreement, "c1");
    // A release row under a confirm id.
    marker(&deal, &confirmed, LedgerKind::Released, "c1").await;
    let message = import_error(&deal, exported(&deal).await).await;
    assert!(message.contains("does not match its id"), "{message}");
}
