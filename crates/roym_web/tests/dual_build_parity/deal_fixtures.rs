use serde_json::{Value, json};
use syneroym_data_db::host_store::RecordWriteValue;
use syneroym_roym_core::transaction;
use syneroym_signed_record::Envelope;

use super::{fixtures::*, helpers::*};

pub(crate) async fn setup_peer_request(h: &Harness, conv: &str) -> (String, String) {
    let (req_id, req_env) = peer_signed_request(conv, 1, 1_000_000);
    deliver_peer_card(
        h,
        &format!("m-req-{req_id}"),
        conv,
        &peer_did(),
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    (req_id, req_env)
}

pub(crate) async fn setup_active_agreement(
    h: &Harness,
    conv: &str,
    timing: &str,
) -> (String, Value) {
    let (req_rec_id, _req_env) = setup_peer_request(h, conv).await;
    let mut q_params = valid_quote_params(conv, &req_rec_id);
    q_params["terms"]["payment_timing"] = json!(timing);
    let (qw, qn) = both_rpc(h, "quote.set", q_params).await;
    assert_eq!(
        qw["result"]["record_id"], qn["result"]["record_id"],
        "quote.set record_id parity failed: qw: {qw}, qn: {qn}"
    );
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env = Envelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let terms = quote_env.payload["terms"].clone();

    let (agr_id, agr_env) = peer_counterparty_receipt(
        conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        terms.clone(),
        1_001_000,
    );
    deliver_peer_card(
        h,
        &format!("m-agr-{agr_id}"),
        conv,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    (quote_rec_id, terms)
}

pub(crate) async fn setup_listing_with_slot(
    h: &Harness,
    slug: &str,
    start: u64,
    end: u64,
    cap: u32,
) -> (String, String) {
    enrol_signing(h, "catalog").await;
    let (set, _) = both_rpc(h, "listing.set", full_listing_params(slug, "Fence Repair")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();

    let slots = json!([{ "start_secs": start, "end_secs": end, "capacity": cap }]);
    let (w, _) =
        both_rpc(h, "availability.set", json!({ "listing_id": listing_id, "slots": slots })).await;
    let slot_id = w["result"]["slot_ids"][0].as_str().unwrap().to_string();
    (listing_id, slot_id)
}

#[allow(dead_code)]
pub(crate) async fn setup_author_slot_quote_accepted_by_peer(
    h: &Harness,
    conv: &str,
    listing_name: &str,
) -> String {
    let start = wall_now() + 1_000_000;
    let end = start + 3600;
    let (listing_id, slot_id) = setup_listing_with_slot(h, listing_name, start, end, 1).await;
    let (peer_req_id, _) = setup_peer_request(h, conv).await;
    let mut q_slot_params = valid_quote_params(conv, &peer_req_id);
    q_slot_params["listing_id"] = json!(listing_id);
    q_slot_params["slot_id"] = json!(slot_id);
    q_slot_params["terms"]["schedule"] = json!({ "earliest_secs": start, "latest_secs": end });
    let (_, q_slot_set_nat) = both_rpc(h, "quote.set", q_slot_params).await;
    let q_slot_rec_id = q_slot_set_nat["result"]["record_id"].as_str().unwrap().to_string();

    let (_, get_q_slot_nat) = both_rpc(
        h,
        "quote.get",
        json!({ "quote_id": q_slot_set_nat["result"]["quote_id"].as_str().unwrap() }),
    )
    .await;
    let env_slot =
        Envelope::from_json(get_q_slot_nat["result"]["envelope"].as_str().unwrap()).unwrap();
    let (agr_id, agr_env) = peer_counterparty_receipt(
        conv,
        &q_slot_rec_id,
        &peer_did(),
        &owner_did(),
        env_slot.payload["terms"].clone(),
        1_001_000,
    );
    deliver_peer_card(
        h,
        &format!("m-agr-{agr_id}"),
        conv,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    q_slot_rec_id
}

pub(crate) async fn setup_synced_counterparty_quote(
    h: &Harness,
    conv: &str,
    seq: u32,
    msg_id: &str,
) -> String {
    let req = both_rpc(
        h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Counterparty deal",
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await
    .1;
    let req_rec_id = req["result"]["record_id"].as_str().unwrap().to_string();

    let (q_rec_id, q_env) = peer_signed_quote(conv, seq, &req_rec_id, &owner_did(), None, 1_000);
    deliver_peer_card(
        h,
        msg_id,
        conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    q_rec_id
}

pub(crate) async fn setup_accepted_counterparty_quote(h: &Harness, conv: &str) -> String {
    let q_rec_id = setup_synced_counterparty_quote(h, conv, 1, "m-quote-157").await;
    both_rpc(h, "agreement.accept", json!({ "quote_record_id": q_rec_id })).await;
    q_rec_id
}

pub(crate) async fn seed_row(
    h: &Harness,
    wasm: bool,
    service: &str,
    collection: &str,
    id: &str,
    row: Value,
) {
    let (storage, ks) =
        if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
    let db = storage.open_service_db(&did_for_service(service), ks).await.unwrap();
    let write = RecordWriteValue { id: id.to_string(), payload: serde_json::to_vec(&row).unwrap() };
    db.put(collection, &write, "seed", None).await.unwrap();
}
