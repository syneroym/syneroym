use serde_json::{Value, json};
use syneroym_roym_core::{backup::Bundle, listing};

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_37_listing_set_without_certificate_refused_parity() {
    let h = harness().await;
    let (w, n) = both_rpc(&h, "listing.set", full_listing_params("hedge", "Hedge trimming")).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
    assert!(w["error"]["message"].as_str().unwrap().contains("signing-not-enrolled"));
}

#[tokio::test]
async fn scenario_38_listing_set_all_blocks_byte_identical_envelope_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (listing_id, mut w, mut n) =
        set_and_get(&h, full_listing_params("hedge-trimming", "Hedge trimming")).await;

    assert_eq!(
        w["result"]["envelope"], n["result"]["envelope"],
        "the signed listing envelope must be byte-identical"
    );
    assert!(w["result"]["envelope"].is_string(), "no envelope in listing.get: {w}");

    // The stored pointer row round-trips identically once the wall-clock
    // `updated_at_secs` is stripped.
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);

    let expected = listing::derive_listing_id(&owner_did(), "hedge-trimming").unwrap();
    assert_eq!(listing_id, expected);
}

#[tokio::test]
async fn scenario_39_listing_edit_is_a_new_version_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (v1w, v1n) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    assert!(!is_err(&v1w, -32602), "v1 wasm: {v1w}");
    assert!(v1w["result"]["record_id"].is_string(), "v1 wasm not ok: {v1w} / native: {v1n}");
    let (v2w, v2n) = both_rpc(
        &h,
        "listing.set",
        full_listing_params("hedge-trimming", "Hedge trimming (weekly)"),
    )
    .await;
    assert_eq!(v2w["result"]["version_count"], 2, "v2 wasm: {v2w} / native: {v2n}");
    assert_eq!(v2w["result"]["record_id"], v2n["result"]["record_id"]);
    assert_ne!(v1w["result"]["record_id"], v2w["result"]["record_id"]);

    let listing_id = v2w["result"]["listing_id"].as_str().unwrap().to_string();

    // One pointer row, two history rows, on both builds.
    let (getw, getn) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(getw["result"]["record_id"], v2w["result"]["record_id"]);
    assert_eq!(getw["result"]["record_id"], getn["result"]["record_id"]);

    let (histw, histn) = both_rpc(&h, "listing.history", json!({ "listing_id": listing_id })).await;
    assert_eq!(histw["result"]["history"].as_array().unwrap().len(), 2);
    assert_eq!(histw["result"]["history"], histn["result"]["history"]);
}

#[tokio::test]
async fn scenario_40_listing_set_float_amount_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let mut params = full_listing_params("hedge-trimming", "Hedge trimming");
    params["payment"]["amount_minor"] = json!(3500.5);
    let (w, n) = both_rpc(&h, "listing.set", params).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
    assert!(
        w["error"]["message"].as_str().unwrap().contains("listing params"),
        "message should report the rejected params: {w}"
    );
}

#[tokio::test]
async fn scenario_41_listing_address_from_profile_parity() {
    let h = harness().await;
    enrol_signing(&h, "profile").await;
    enrol_signing(&h, "catalog").await;

    both_rpc(
        &h,
        "profile.set",
        json!({ "display_name": "Alice", "conversation_address": "did:key:zAliceConvFromProfile" }),
    )
    .await;

    let mut params = full_listing_params("hedge-trimming", "Hedge trimming");
    params.as_object_mut().unwrap().remove("conversation_address");
    let (_id, gw, gn) = set_and_get(&h, params).await;
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);

    let (vw, vn) =
        both_rpc(&h, "listing.verify", json!({ "envelope": gw["result"]["envelope"].clone() }))
            .await;
    assert_eq!(vw, vn);
    assert_eq!(vw["result"]["conversation_address"], "did:key:zAliceConvFromProfile");
}

#[tokio::test]
async fn scenario_42_listing_address_missing_no_profile_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let mut params = full_listing_params("hedge-trimming", "Hedge trimming");
    params.as_object_mut().unwrap().remove("conversation_address");
    let (w, n) = both_rpc(&h, "listing.set", params).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
}

#[tokio::test]
async fn scenario_43_publication_rate_limit_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    both_rpc(&h, "listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;

    let params = full_listing_params("hedge-trimming", "Hedge trimming");
    let mut outcomes_w = Vec::new();
    let mut outcomes_n = Vec::new();
    for _ in 0..4 {
        let (w, n) = both_rpc(&h, "listing.set", params.clone()).await;
        outcomes_w.push(if is_err(&w, -32602) { "rate-limited" } else { "allow" });
        outcomes_n.push(if is_err(&n, -32602) { "rate-limited" } else { "allow" });
        if is_err(&w, -32602) {
            let retry = w["error"]["data"]["retry_after_secs"].as_u64().unwrap();
            // Close to a full window: the oldest counted publication is only
            // seconds old. A wide lower bound keeps a slow CI host honest.
            assert!((3300..=3600).contains(&retry), "retry_after_secs out of range: {retry}");
            assert_eq!(w["error"]["data"]["admission"], "rate-limited");
        }
    }
    assert_eq!(outcomes_w, ["allow", "allow", "rate-limited", "rate-limited"]);
    assert_eq!(outcomes_n, outcomes_w);
}

#[tokio::test]
async fn scenario_44_withdraw_ignores_publication_budget_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    both_rpc(&h, "listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;

    let params = full_listing_params("hedge-trimming", "Hedge trimming");
    let (first, _) = both_rpc(&h, "listing.set", params.clone()).await;
    let listing_id = first["result"]["listing_id"].as_str().unwrap().to_string();
    // Exhaust the budget.
    both_rpc(&h, "listing.set", params.clone()).await;
    let (blocked, _) = both_rpc(&h, "listing.set", params).await;
    assert!(is_err(&blocked, -32602));

    // Withdrawal is still admitted on both builds.
    let (w, n) = both_rpc(&h, "listing.withdraw", json!({ "listing_id": listing_id })).await;
    assert!(!is_err(&w, -32602), "withdraw refused: {w}");
    assert!(w["result"]["record_id"].is_string(), "withdraw not ok: {w}");
    assert!(n["result"]["record_id"].is_string(), "withdraw not ok: {n}");

    let (gw, gn) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(gw["result"]["status"], "withdrawn");
    assert_eq!(gn["result"]["status"], "withdrawn");
}

#[tokio::test]
async fn scenario_45_withdraw_then_get_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (first, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = first["result"]["listing_id"].as_str().unwrap().to_string();

    both_rpc(&h, "listing.withdraw", json!({ "listing_id": listing_id })).await;
    let (mut w, mut n) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(w["result"]["status"], "withdrawn");
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);

    let (histw, _) = both_rpc(&h, "listing.history", json!({ "listing_id": listing_id })).await;
    assert_eq!(histw["result"]["history"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_46_listing_verify_good_envelope_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let env = gw["result"]["envelope"].clone();

    let (w, n) = both_rpc(&h, "listing.verify", json!({ "envelope": env })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["verified"], true, "verify refused the envelope: {w}");
    assert_eq!(w["result"]["conversation_address"], "did:key:zProviderConv");
}

#[tokio::test]
async fn scenario_47_listing_verify_tampered_envelope_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let env_str = gw["result"]["envelope"].as_str().unwrap();
    let mut env: Value = serde_json::from_str(env_str).unwrap();
    // Edit the payload's listing_id -- the signature no longer covers it.
    env["payload"]["listing_id"] = json!("lst_forged");

    let (w, n) = both_rpc(&h, "listing.verify", json!({ "envelope": env.to_string() })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["verified"], false);
    assert!(w["result"]["reason"].is_string());
}

#[tokio::test]
async fn scenario_48_availability_set_and_list_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (set, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();

    let slots = json!([
        { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 },
        { "start_secs": 1_010_000, "end_secs": 1_013_600, "capacity": 2 },
        // A duplicate of the first slot: content-derived id, so one row.
        { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 5 }
    ]);
    let (w, n) =
        both_rpc(&h, "availability.set", json!({ "listing_id": listing_id, "slots": slots })).await;
    assert_eq!(w["result"]["slot_ids"], n["result"]["slot_ids"]);

    let (lw, ln) = both_rpc(&h, "availability.list", json!({ "listing_id": listing_id })).await;
    assert_eq!(lw, ln);
    let listed = lw["result"]["slots"].as_array().unwrap();
    assert_eq!(listed.len(), 2, "the duplicate slot must converge on one row");
    assert!(
        listed[0]["start_secs"].as_u64().unwrap() <= listed[1]["start_secs"].as_u64().unwrap(),
        "slots must be ordered by start_secs"
    );
}

#[tokio::test]
async fn scenario_49_catalog_export_integrity_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (set, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();
    both_rpc(
        &h,
        "availability.set",
        json!({ "listing_id": listing_id, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 }
        ] }),
    )
    .await;

    let (mut w, mut n) = both_rpc(&h, "catalog.export", json!({})).await;

    for side in [&w, &n] {
        let bundle: Bundle = serde_json::from_value(side["result"].clone()).unwrap();
        bundle.check_integrity().expect("exported bundle integrity");
        let sections = &bundle.manifest.sections;
        assert_eq!(sections["listings"].schema_version, 2);
        assert!(sections.contains_key("availability"));
    }

    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);
}

#[tokio::test]
async fn scenario_50_catalog_import_roundtrip_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (set, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();
    both_rpc(
        &h,
        "availability.set",
        json!({ "listing_id": listing_id, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 }
        ] }),
    )
    .await;

    let (exp, _) = both_rpc(&h, "catalog.export", json!({})).await;
    let bundle = exp["result"].clone();
    let (impw, impn) = both_rpc(&h, "catalog.import", json!({ "bundle": bundle })).await;
    assert!(!is_err(&impw, -32602), "import failed: {impw}");
    assert_eq!(impw["result"]["imported"], impn["result"]["imported"]);

    // The listing re-verifies and its id is preserved on both builds.
    let (getw, getn) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(getw["result"]["listing_id"], listing_id);
    let (vw, vn) =
        both_rpc(&h, "listing.verify", json!({ "envelope": getw["result"]["envelope"].clone() }))
            .await;
    assert_eq!(vw["result"]["verified"], true);
    assert_eq!(vn["result"]["verified"], true);
    let (aw, an) = both_rpc(&h, "availability.list", json!({ "listing_id": listing_id })).await;
    assert_eq!(aw["result"]["slots"], an["result"]["slots"]);
    let _ = getn;
}

#[tokio::test]
async fn scenario_51_catalog_import_tampered_listing_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;

    let (exp, _) = both_rpc(&h, "catalog.export", json!({})).await;
    let mut bundle = exp["result"].clone();
    // Edit one stored envelope byte without touching the manifest digest.
    let rows = bundle["sections"]["listings"].as_array_mut().unwrap();
    let env_str = rows[0]["payload"]["envelope"].as_str().unwrap();
    let mut env: Value = serde_json::from_str(env_str).unwrap();
    env["payload"]["title"] = json!("Tampered title");
    rows[0]["payload"]["envelope"] = json!(env.to_string());

    let (w, n) = both_rpc(&h, "catalog.import", json!({ "bundle": bundle })).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
}
