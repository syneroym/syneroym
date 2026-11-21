use serde_json::{Value, json};
use syneroym_roym_core::{backup::Bundle, services};

use super::{fixtures::*, helpers::*};

const LISTING: &str = "lst_parity_slots";

/// The id of a slot with `capacity` seats, stored on both builds.
async fn seeded_slot(h: &Harness, capacity: u32) -> String {
    let (w, n) = both_rpc(
        h,
        "availability.set",
        json!({ "listing_id": LISTING, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": capacity }
        ] }),
    )
    .await;
    assert_eq!(w, n);
    w["result"]["slot_ids"][0].as_str().unwrap().to_string()
}

fn details(slot_id: &str) -> Value {
    json!({
        "extension": "scheduling",
        "schema": "scheduling/slot@1",
        "data": { "listing_id": LISTING, "slot_id": slot_id },
    })
}

/// One hook call on both builds. The hooks are not routed from `web`: the
/// transaction service reaches them by a local call, as this does.
async fn hook(h: &Harness, method: &str, params: Value) -> Value {
    let (w, n) = h.local_invoke(services::SCHEDULING, &env(method, params)).await;
    assert_eq!(w, n, "{method} must answer the same on both builds");
    w
}

fn key(agreement: &str) -> Value {
    json!({ "agreement": agreement, "commitment_id": "c1" })
}

#[tokio::test]
async fn scenario_247_reservation_hooks_agree_on_both_builds() {
    let h = harness().await;
    let slot_id = seeded_slot(&h, 1).await;
    let reserve = |agreement: &str| json!({ "agreement": agreement, "commitment_id": "c1", "details": details(&slot_id) });

    let prepared = hook(&h, "reservation.prepare", json!({ "details": details(&slot_id) })).await;
    assert_eq!(prepared["result"]["window"]["earliest_secs"], 1_000_000);

    let held = hook(&h, "reservation.reserve", reserve("rec_a")).await;
    assert_eq!(held["result"]["state"], "held", "{held}");
    let repeat = hook(&h, "reservation.reserve", reserve("rec_a")).await;
    assert_eq!(repeat, held, "a repeat reserve returns the same reservation");

    let full = hook(&h, "reservation.reserve", reserve("rec_b")).await;
    assert_eq!(full["error"]["data"]["code"], "slot-taken", "{full}");

    let confirmed = hook(&h, "reservation.confirm", key("rec_a")).await;
    assert_eq!(confirmed["result"]["state"], "confirmed", "{confirmed}");

    let released = hook(&h, "reservation.release", key("rec_a")).await;
    assert_eq!(released["result"]["released"], true);
    let again = hook(&h, "reservation.release", key("rec_a")).await;
    assert_eq!(again["result"]["released"], false);

    // The seat is free again. A reservation that was released before it was
    // confirmed cannot be confirmed: a confirm never takes a seat.
    let taken = hook(&h, "reservation.reserve", reserve("rec_b")).await;
    assert_eq!(taken["result"]["state"], "held", "{taken}");
    hook(&h, "reservation.release", key("rec_b")).await;
    let lost = hook(&h, "reservation.confirm", key("rec_b")).await;
    assert_eq!(lost["error"]["data"]["code"], "reservation-lost", "{lost}");
    let free = hook(&h, "reservation.reserve", reserve("rec_c")).await;
    assert_eq!(free["result"]["state"], "held", "a refused confirm took no seat: {free}");
}

#[tokio::test]
async fn scenario_248_scheduling_export_is_signed_and_has_both_sections_parity() {
    let h = harness().await;
    enrol_signing(&h, "scheduling").await;
    let slot_id = seeded_slot(&h, 2).await;
    hook(
        &h,
        "reservation.reserve",
        json!({ "agreement": "rec_a", "commitment_id": "c1", "details": details(&slot_id) }),
    )
    .await;

    let (mut w, mut n) = both_rpc(&h, "scheduling.export", json!({})).await;
    for side in [&w, &n] {
        let bundle: Bundle = serde_json::from_value(side["result"].clone()).unwrap();
        bundle.check_integrity().expect("exported bundle integrity");
        let sections = &bundle.manifest.sections;
        assert_eq!(sections["availability"].record_count, 1);
        assert_eq!(sections["reservations"].record_count, 3, "rsv, seat and key rows");
    }
    verify_and_strip_manifest_signature(&mut w);
    verify_and_strip_manifest_signature(&mut n);
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);
}

#[tokio::test]
async fn scenario_249_scheduling_import_keeps_the_held_seat_parity() {
    let h = harness().await;
    enrol_signing(&h, "scheduling").await;
    let slot_id = seeded_slot(&h, 1).await;
    let reserve = |agreement: &str| json!({ "agreement": agreement, "commitment_id": "c1", "details": details(&slot_id) });
    hook(&h, "reservation.reserve", reserve("rec_a")).await;

    let (exp, _) = both_rpc(&h, "scheduling.export", json!({})).await;
    let bundle = exp["result"].clone();
    let (impw, impn) = both_rpc(&h, "scheduling.import", json!({ "bundle": bundle })).await;
    assert!(!is_err(&impw, -32602), "import failed: {impw}");
    assert_eq!(impw["result"]["imported"], impn["result"]["imported"]);

    // The seat is still held after the import, on both builds.
    let full = hook(&h, "reservation.reserve", reserve("rec_b")).await;
    assert_eq!(full["error"]["data"]["code"], "slot-taken", "{full}");
}

#[tokio::test]
async fn scenario_250_scheduling_import_of_a_tampered_bundle_is_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "scheduling").await;
    let slot_id = seeded_slot(&h, 1).await;
    hook(
        &h,
        "reservation.reserve",
        json!({ "agreement": "rec_a", "commitment_id": "c1", "details": details(&slot_id) }),
    )
    .await;
    let (exp, _) = both_rpc(&h, "scheduling.export", json!({})).await;
    let mut bundle = exp["result"].clone();
    // A seat row with no reservation: the bundle digest is stale, so this is
    // refused by the integrity check before any row is read.
    bundle["sections"]["reservations"]
        .as_array_mut()
        .unwrap()
        .retain(|r| !r["id"].as_str().unwrap().starts_with("rsv:"));
    let (w, n) = both_rpc(&h, "scheduling.import", json!({ "bundle": bundle })).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602), "{w}");
}
