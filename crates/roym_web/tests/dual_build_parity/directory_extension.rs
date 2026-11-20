//! Parity test for the `extension` filter of `directory.search`.

use serde_json::json;
use syneroym_roym_core::services;

use super::{fixtures::*, helpers::*};

async fn publish(h: &Harness, params: serde_json::Value) -> String {
    let (id, gw, _gn) = set_and_get(h, params).await;
    let envelope = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(h, &envelope).await;
    id
}

async fn hit_ids(h: &Harness, filter: serde_json::Value) -> Vec<String> {
    let (w, n) = wire_invoke(h, services::DIRECTORY, &env("directory.search", filter)).await;
    assert_eq!(stripped(&w), stripped(&n));
    let mut ids: Vec<String> = w["result"]["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["listing_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn scenario_245_search_by_extension_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let scheduled =
        publish(&h, full_listing_params("hedge-trimming-245", "Scheduled hedges")).await;
    let mut one_shot = full_listing_params("hedge-one-shot-245", "One-shot hedges");
    one_shot["extension"] = json!({ "id": "one-shot", "version": 1 });
    let one_shot = publish(&h, one_shot).await;
    let mut plain = full_listing_params("hedge-plain-245", "Plain hedges");
    plain.as_object_mut().unwrap().remove("extension");
    plain.as_object_mut().unwrap().remove("booking");
    plain.as_object_mut().unwrap().remove("payment");
    publish(&h, plain).await;

    assert_eq!(hit_ids(&h, json!({})).await.len(), 3);
    assert_eq!(hit_ids(&h, json!({ "extension": "scheduling" })).await, [scheduled]);
    assert_eq!(hit_ids(&h, json!({ "extension": "one-shot" })).await, [one_shot]);
    assert!(hit_ids(&h, json!({ "extension": "future" })).await.is_empty());
}
