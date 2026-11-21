use serde_json::{Value, json};

use super::{fixtures::*, helpers::*};

/// The commitments `(id, state, reason)` of the newest snapshot a build holds.
fn commitments_of(progress: &Value) -> Vec<(String, String, Option<String>)> {
    progress["result"]["snapshots"][0]["snapshot"]["commitments"]
        .as_array()
        .expect("a snapshot")
        .iter()
        .map(|c| {
            (
                c["commitment_id"].as_str().unwrap().to_string(),
                c["state"].as_str().unwrap().to_string(),
                c["reason"].as_str().map(str::to_string),
            )
        })
        .collect()
}

#[tokio::test]
async fn scenario_251_a_peers_commitment_failure_is_filed_and_ends_the_deal_on_both_builds() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;
    // The peer gives the slot; this node is the writer and gives nothing that
    // needs a reservation.
    let agreement =
        setup_slot_quote_accepted_by_peer(&h, &conv, "failure-listing", "counterparty").await;

    let payload = json!({
        "agreement": agreement,
        "conversation": conv,
        "commitment_id": "c1",
        "reason": "reservation-lost",
    });
    let envelope = sign_as_peer("commitment-failure", 1, &agreement, payload, None, wall_now());
    deliver_peer_card(&h, "m-failure", &conv, &peer_did(), "commitment-failure", 1, &envelope)
        .await;
    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw["result"]["filed"], 1, "{sw}");
    assert_eq!(sw["result"]["refused"], 0, "{sw}");
    assert_eq!(sn["result"]["filed"], 1, "{sn}");
    assert_eq!(sn["result"]["refused"], 0, "{sn}");

    let (gw, gn) = both_rpc(&h, "agreement.get", json!({ "quote_record_id": agreement })).await;
    assert_eq!(gw["result"]["state"], "closed", "{gw}");
    assert_eq!(gn["result"]["state"], "closed", "{gn}");
    let (pw, pn) = both_rpc(&h, "agreement.progress", json!({ "agreement": agreement })).await;
    let expected = vec![
        ("c1".to_string(), "failed".to_string(), Some("reservation-lost".to_string())),
        ("c2".to_string(), "cancelled".to_string(), Some("other-commitment-failed".to_string())),
    ];
    assert_eq!(commitments_of(&pw), expected);
    assert_eq!(commitments_of(&pn), expected);

    // A repeat of the card makes no second snapshot on either build.
    deliver_peer_card(&h, "m-failure-2", &conv, &peer_did(), "commitment-failure", 1, &envelope)
        .await;
    let (rw, rn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(rw["result"]["refused"], 0, "{rw}");
    assert_eq!(rn["result"]["refused"], 0, "{rn}");
    let (qw, qn) = both_rpc(&h, "agreement.progress", json!({ "agreement": agreement })).await;
    assert_eq!(
        qw["result"]["snapshots"].as_array().map(Vec::len),
        pw["result"]["snapshots"].as_array().map(Vec::len)
    );
    assert_eq!(
        qn["result"]["snapshots"].as_array().map(Vec::len),
        pn["result"]["snapshots"].as_array().map(Vec::len)
    );
}
