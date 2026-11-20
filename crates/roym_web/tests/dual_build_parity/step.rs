use serde_json::json;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_246_step_add_list_and_verify_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;
    let agreement = setup_author_slot_quote_accepted_by_peer(&h, &conv, "step-listing").await;

    let input = json!({
        "agreement": agreement,
        "commitment_id": "c1",
        "step_type": "scheduling/started",
    });
    let (w, n) = both_rpc(&h, "step.add", input.clone()).await;
    assert_eq!(w["result"]["role"], "quote-author", "{w}");
    assert_eq!(w["result"]["record_id"], n["result"]["record_id"], "{w} {n}");
    assert_eq!(w["result"]["state"], n["result"]["state"], "{w} {n}");
    assert_ne!(w["result"]["state"], "already-recorded");

    // A repeat answers with the stored record on both builds.
    let (rw, rn) = both_rpc(&h, "step.add", input).await;
    assert_eq!(rw["result"]["state"], "already-recorded", "{rw}");
    assert_eq!(rw["result"]["record_id"], w["result"]["record_id"]);
    assert_eq!(rn["result"]["record_id"], n["result"]["record_id"]);

    let (lw, ln) = both_rpc(&h, "step.list", json!({ "agreement": agreement })).await;
    assert_eq!(lw, ln);
    let step = &lw["result"]["steps"][0];
    assert_eq!(step["step_type"], "scheduling/started");
    assert_eq!(step["role"], "giver");
    assert_eq!(step["data"], json!({}));
    assert_eq!(step["verified"], true);
    assert_eq!(step["checked"], true);

    let envelope = step["envelope"].as_str().unwrap();
    let (vw, vn) = both_rpc(&h, "step.verify", json!({ "envelope": envelope })).await;
    assert_eq!(vw, vn);
    assert_eq!(vw["result"]["verified"], true, "{vw}");
    assert_eq!(vw["result"]["hidden"], json!(["data"]));

    // Refusals are the same on both builds.
    let mut other_type = json!({ "agreement": agreement, "commitment_id": "c1" });
    other_type["step_type"] = json!("scheduling/finished");
    let (xw, xn) = both_rpc(&h, "step.add", other_type).await;
    assert_eq!(xw, xn);
    assert_eq!(xw["error"]["data"]["code"], "step-type-unknown", "{xw}");

    let no_extension = json!({
        "agreement": agreement,
        "commitment_id": "c2",
        "step_type": "scheduling/started",
    });
    let (cw, cn) = both_rpc(&h, "step.add", no_extension).await;
    assert_eq!(cw, cn);
    assert_eq!(cw["error"]["message"], "commitment-has-no-extension", "{cw}");
}
