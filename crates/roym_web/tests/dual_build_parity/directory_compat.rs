//! Parity tests for directory compatibility: newer node answers with
//! extra unknown keys, and a remote source running no SynOrg.

use serde_json::{Value, json};

use super::{fixtures::*, helpers::*, trust_harness::listing_owner_did};

#[tokio::test]
async fn scenario_229_a_newer_node_answer_with_unknown_keys_is_read_parity() {
    let h = harness().await;

    let (aw, an) = both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hNewer" })).await;
    assert_eq!(stripped(&aw), stripped(&an));

    let (pw, pn) = both_rpc(&h, "directory.probe-info", json!({ "did": "did:key:hNewer" })).await;
    assert_eq!(stripped(&pw), stripped(&pn));
    assert_eq!(pw["result"]["name"], "Newer Guild");
    assert!(pw["result"].get("extra").is_none(), "extra key must be dropped: {pw}");

    let (rw, rn) = both_rpc(&h, "directory.start-run", json!({})).await;
    assert_eq!(stripped(&rw), stripped(&rn));
    let run_id = rw["result"]["run_id"].as_str().unwrap();

    let (qw, qn) = both_rpc(
        &h,
        "directory.query-source",
        json!({ "run_id": run_id, "source": "did:key:hNewer", "query": {} }),
    )
    .await;
    assert_eq!(stripped(&qw), stripped(&qn));
    assert_eq!(qw["result"]["verified"], 1, "hit must be stored: {qw}");
    assert_eq!(qw["result"]["error"], Value::Null, "error must be null: {qw}");

    let (cw, cn) = both_rpc(
        &h,
        "directory.check-standing",
        json!({ "source": "did:key:hNewer", "member_did": listing_owner_did() }),
    )
    .await;
    assert_eq!(stripped(&cw), stripped(&cn));
    assert_eq!(cw["result"]["refreshed"], true, "check-standing must be refreshed: {cw}");
    assert!(cw["result"].get("extra").is_none(), "extra key must be dropped: {cw}");

    enrol_signing(&h, "catalog").await;
    let (lw, ln) =
        both_rpc(&h, "listing.set", full_listing_params("newer-compat-listing", "Newer listing"))
            .await;
    assert_eq!(stripped(&lw), stripped(&ln));
    let listing_id = lw["result"]["listing_id"].as_str().unwrap();

    let (pub_w, pub_n) = both_rpc(
        &h,
        "directory.publish-to-source",
        json!({ "source": "did:key:hNewer", "listing_id": listing_id }),
    )
    .await;
    assert_eq!(stripped(&pub_w), stripped(&pub_n));
    assert_eq!(pub_w["result"]["listing_id"], listing_id);
    assert!(pub_w["result"]["record_id"].is_string());
    assert!(pub_w["result"].get("receipt").is_none(), "receipt key must be dropped: {pub_w}");
}

#[tokio::test]
async fn scenario_230_a_source_with_no_synorg_is_noted_and_probes_as_null_parity() {
    let h = harness().await;

    let (aw, an) =
        both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_eq!(stripped(&aw), stripped(&an));
    assert!(
        aw["result"]["probe"].as_str().unwrap().contains("runs no directory"),
        "probe note expected: {aw}"
    );

    let (pw, pn) =
        both_rpc(&h, "directory.probe-info", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_eq!(stripped(&pw), stripped(&pn));
    assert_eq!(pw["result"], Value::Null, "probe-info must answer null: {pw}");
}
