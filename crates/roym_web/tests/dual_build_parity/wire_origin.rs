use serde_json::{Value, json};
use syneroym_roym_core::services;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_67_every_service_invoke_over_the_wire_is_refused_parity() {
    let h = harness().await;
    for (svc, method) in WIRE_REFUSED_VERBS {
        let (w, n) = h.wire_invoke(svc, &env(method, json!({}))).await;
        assert_eq!(w, n, "{} wire parity", svc.name);
        assert_eq!(w["error"]["code"], -32013, "{}.{method} must be wire-refused: {w}", svc.name);
    }
}

#[tokio::test]
async fn scenario_68_every_service_invoke_locally_is_not_wire_refused_parity() {
    let h = harness().await;
    for (svc, method) in WIRE_REFUSED_VERBS {
        let (w, n) = h.local_invoke(svc, &env(method, json!({}))).await;
        assert_ne!(w["error"]["code"].as_i64(), Some(-32013), "{}.{method} wasm: {w}", svc.name);
        assert_ne!(n["error"]["code"].as_i64(), Some(-32013), "{}.{method} native: {n}", svc.name);
    }
}

// Scenario 69 (`same_verbs_locally_are_admitted`) was folded into
// scenario 68, which now drives `local_invoke` for a verb of every one of
// the six services and asserts not-`-32013`.

#[tokio::test]
async fn scenario_70_local_call_with_delegated_caller_admitted_on_both_builds() {
    // A regression guard: the parity driver already presents a verified
    // delegated caller on a purely local drive. A native mapping that read
    // the caller's auth level on a local path would answer -32013 here while
    // the wasm build passed.
    let h = harness().await;
    for (svc, method, params) in [
        (services::CATALOG, "listing.get", json!({ "listing_id": "lst_x" })),
        (services::CONVERSATION, "conversation.history", json!({ "conversation": "c" })),
    ] {
        let (w, n) = h.local_invoke(svc, &env(method, params)).await;
        assert_ne!(w["error"]["code"].as_i64(), Some(-32013), "{method} wasm: {w}");
        assert_ne!(n["error"]["code"].as_i64(), Some(-32013), "{method} native: {n}");
    }
}

#[tokio::test]
async fn scenario_71_api_status_over_the_wire_is_never_refused_parity() {
    let h = harness().await;
    for svc in services::ALL {
        let (w, n) = h.wire_status(svc).await;
        assert_eq!(w, n, "status mismatch on {}", svc.name);
        assert_ne!(w["error"]["code"].as_i64(), Some(-32013));
        assert_eq!(w["service"], svc.name);
    }
}

#[tokio::test]
async fn scenario_72_web_http_path_unaffected_parity() {
    let h = harness().await;
    let (w, n) = both_rpc(&h, "listing.list", json!({})).await;
    assert_eq!(w, n);
    assert!(w["result"]["listings"].is_array(), "listing.list via /rpc must succeed: {w}");
}

#[tokio::test]
async fn scenario_73_guard_no_c5_verb_answers_method_not_found_or_wire_refused() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    enrol_signing(&h, "conversation").await;

    let (listing_id, gw, _gn) =
        set_and_get(&h, full_listing_params("guard-listing", "Guard listing")).await;
    let env_val = gw["result"]["envelope"].clone();
    let conv_id = open_conv(&h, "did:key:zPeer73").await;
    let (sent, _) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "guard" }))
            .await;
    let message_id = sent["result"]["message_id"].as_str().unwrap().to_string();

    // The guard test: every listing, availability and conversation verb,
    // driven once through the local path with params
    // that reach the handler. A -32601 means the verb was never wired; a
    // -32013 means the local admission rule is wrong.
    let calls: Vec<(&str, Value)> = vec![
        ("listing.set", full_listing_params("guard-listing", "Guard listing")),
        ("listing.get", json!({ "listing_id": listing_id })),
        ("listing.list", json!({})),
        ("listing.history", json!({ "listing_id": listing_id })),
        ("listing.withdraw", json!({ "listing_id": listing_id })),
        ("listing.verify", json!({ "envelope": env_val })),
        ("listing.limits", json!({})),
        ("listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 20 })),
        (
            "availability.set",
            json!({ "listing_id": listing_id, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 }
        ] }),
        ),
        ("availability.list", json!({ "listing_id": listing_id })),
        ("availability.get", json!({ "slot_id": "slot_missing" })),
        ("availability.remove", json!({ "slot_id": "slot_missing" })),
        ("catalog.export", json!({})),
        ("catalog.signing-status", json!({})),
        ("conversation.open", json!({ "address": "did:key:zPeer73b" })),
        ("conversation.list", json!({})),
        ("conversation.send", json!({ "conversation": conv_id, "body": "again" })),
        ("conversation.history", json!({ "conversation": conv_id })),
        ("conversation.delivery-status", json!({ "message_id": message_id })),
        ("conversation.outbox", json!({})),
        ("conversation.retry", json!({ "message_id": message_id })),
        ("conversation.delete-message", json!({ "message_id": message_id })),
        ("conversation.search", json!({ "query": "guard" })),
        ("conversation.export", json!({})),
        ("conversation.signing-status", json!({})),
    ];

    for (method, params) in calls {
        let (w, n) = both_rpc(&h, method, params).await;
        for (label, v) in [("wasm", &w), ("native", &n)] {
            let code = v["error"]["code"].as_i64();
            assert_ne!(code, Some(-32601), "{label} {method} answered method-not-found: {v}");
            assert_ne!(code, Some(-32013), "{label} {method} answered wire-refused: {v}");
        }
    }
}

#[tokio::test]
async fn scenario_74_synorg_settings_round_trip_parity() {
    let h = harness().await;
    let settings = json!({
        "name": "Bengaluru Trades Guild",
        "rules": "Be kind. Do good work.",
        "area": [],
        "categories": ["plumbing", "gardening"],
        "support_contact": "support@example.org",
        "dispute_path": "Contact the owner.",
        "retention_secs": 2_592_000,
        "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
    });
    let (sw, sn) = both_rpc(&h, "directory.set-settings", settings.clone()).await;
    assert_eq!(sw, sn);
    assert!(sw["result"]["name"].is_string(), "{sw}");
    let (gw, gn) = both_rpc(&h, "directory.settings", json!({})).await;
    assert_eq!(gw, gn);
    assert_eq!(gw["result"]["retention_secs"], 2_592_000);
    assert_eq!(gw["result"]["categories"], json!(["plumbing", "gardening"]));
}

#[tokio::test]
async fn scenario_75_synorg_settings_validation_refuses_bad_input_parity() {
    let h = harness().await;
    let bad = json!({
        "name": "",
        "rules": "x",
        "area": [],
        "categories": [],
        "support_contact": "x",
        "dispute_path": "x",
        "retention_secs": 2_592_000,
        "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
    });
    let (w, n) = both_rpc(&h, "directory.set-settings", bad).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602), "{w}");
}
