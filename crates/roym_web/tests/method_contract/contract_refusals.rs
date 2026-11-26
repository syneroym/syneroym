use serde_json::json;

use super::{
    contract::post_rpc,
    deal_fixtures::{seed_row, setup_active_agreement},
    fixtures::{enrol_signing, open_conv, peer_did},
    helpers::harness,
};

#[tokio::test]
async fn argument_refusals_have_matching_message_and_code() {
    let h = harness().await;

    // 1. bad-commitment-list
    let (w, _) = post_rpc(
        &h,
        "agreement.close",
        Some(json!({
            "agreement": "rec_dummy",
            "as": "giver",
            "commitments": []
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "bad-commitment-list");
    assert_eq!(w["error"]["data"]["code"], "bad-commitment-list");

    // 2. bad-period
    let (w, _) = post_rpc(
        &h,
        "statement.export",
        Some(json!({
            "from_secs": 200,
            "to_secs": 100
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "bad-period");
    assert_eq!(w["error"]["data"]["code"], "bad-period");

    // 3. bad-deal-defaults
    let (w, _) = post_rpc(
        &h,
        "deal.defaults-set",
        Some(json!({
            "cancellation_terms": { "lang": "en", "text": "Terms" },
            "refund_terms": { "lang": "en", "text": "Terms" },
            "dispute_path": { "lang": "en", "text": "Terms" },
            "quote_valid_secs": 10
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "bad-deal-defaults");
    assert_eq!(w["error"]["data"]["code"], "bad-deal-defaults");
}

#[tokio::test]
async fn agreement_state_and_overflow_refusals_have_matching_message_and_code() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr, _) = setup_active_agreement(&h, &conv, "after-work").await;

    // 4. no-such-commitment
    let (w, _) = post_rpc(
        &h,
        "agreement.close",
        Some(json!({
            "agreement": agr,
            "as": "giver",
            "commitments": ["c_nonexistent"]
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "no-such-commitment");
    assert_eq!(w["error"]["data"]["code"], "no-such-commitment");

    // 5. not-your-side
    let (w, _) = post_rpc(
        &h,
        "agreement.close",
        Some(json!({
            "agreement": agr,
            "as": "receiver",
            "commitments": ["c1"]
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "not-your-side");
    assert_eq!(w["error"]["data"]["code"], "not-your-side");

    // 6. agreement-not-open (cancel agreement)
    let (c_w, _) = post_rpc(
        &h,
        "agreement.cancel",
        Some(json!({
            "agreement": agr,
            "reason": "cannot-deliver"
        })),
    )
    .await;
    assert!(c_w.get("error").is_none());

    let (w, _) = post_rpc(
        &h,
        "agreement.close",
        Some(json!({
            "agreement": agr,
            "as": "giver",
            "commitments": ["c1"]
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "agreement-not-open");
    assert_eq!(w["error"]["data"]["code"], "agreement-not-open");

    // 7. statement-total-overflow
    let over_row = json!({
        "id": "log_overflow",
        "recorded_at_secs": 150,
        "occurred_at_secs": 150,
        "event": "commitment-settled",
        "agreement": agr,
        "conversation": conv,
        "commitment_id": "c1",
        "counterparty_did": peer_did(),
        "direction": "received",
        "resource_kind": "currency",
        "commitment_kind": "currency",
        "currency": "INR",
        "declared_value_minor": 9_007_199_254_740_992_u64,
        "records": ["rec_1"]
    });
    seed_row(&h, true, "transaction", "event_log", "log_overflow", over_row.clone()).await;
    seed_row(&h, false, "transaction", "event_log", "log_overflow", over_row).await;

    let (w, _) = post_rpc(
        &h,
        "statement.export",
        Some(json!({
            "from_secs": 100,
            "to_secs": 200
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["message"], "statement-total-overflow");
    assert_eq!(w["error"]["data"]["code"], "statement-total-overflow");
}
