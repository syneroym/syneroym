use serde_json::{Value, json};
use syneroym_roym_core::services;

use super::{
    fixtures::{both_rpc, env},
    helpers::{Harness, harness},
    shapes::{assert_shape, enrol_native_signing, rpc},
};

async fn availability_shapes(h: &Harness, listing_id: &str) {
    let avail_list_empty =
        rpc(h, "availability.list", json!({ "listing_id": "nonexistent" })).await;
    assert_shape("availability_list_empty", &avail_list_empty);

    let slots = json!([
        { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 2 },
        { "start_secs": 1_010_000, "end_secs": 1_013_600, "capacity": 1 }
    ]);
    let avail_set =
        rpc(h, "availability.set", json!({ "listing_id": listing_id, "slots": slots })).await;
    assert_shape("availability_set", &avail_set);
    let slot_id1 = avail_set["slot_ids"][0].as_str().unwrap().to_string();
    let slot_id2 = avail_set["slot_ids"][1].as_str().unwrap().to_string();

    let avail_get_found = rpc(h, "availability.get", json!({ "slot_id": slot_id1 })).await;
    assert_shape("availability_get_found", &avail_get_found);

    let avail_get_missing = rpc(h, "availability.get", json!({ "slot_id": "nonexistent" })).await;
    assert_shape("availability_get_missing", &avail_get_missing);

    let avail_list_two = rpc(h, "availability.list", json!({ "listing_id": listing_id })).await;
    assert_shape("availability_list_two_slots", &avail_list_two);

    let avail_rem_found = rpc(h, "availability.remove", json!({ "slot_id": slot_id2 })).await;
    assert_shape("availability_remove_found", &avail_rem_found);

    let avail_rem_missing =
        rpc(h, "availability.remove", json!({ "slot_id": "nonexistent" })).await;
    assert_shape("availability_remove_missing", &avail_rem_missing);
}

/// Calls a hook on both builds. The hooks are not routed from `web`, so they
/// are reached the way `transaction` reaches them: a local dispatch.
async fn hook(h: &Harness, method: &str, params: Value) -> Value {
    let (wasm, native) = h.local_invoke(services::SCHEDULING, &env(method, params)).await;
    assert_eq!(wasm, native, "{method} differs between the builds");
    native
}

async fn reservation_shapes(h: &Harness) {
    let slots = json!([{ "start_secs": 2_000_000, "end_secs": 2_003_600, "capacity": 1 }]);
    // Both builds need the slot: a hook is called on each of them.
    let (_, set) =
        both_rpc(h, "availability.set", json!({ "listing_id": "lst_shape", "slots": slots })).await;
    let slot_id = set["result"]["slot_ids"][0].as_str().unwrap().to_string();
    let details = json!({
        "extension": "scheduling",
        "schema": "scheduling/slot@1",
        "data": { "listing_id": "lst_shape", "slot_id": slot_id },
    });

    let prepared = hook(h, "reservation.prepare", json!({ "details": details })).await;
    assert_shape("reservation_prepare", &prepared["result"]);

    let reserve = json!({ "agreement": "rec_shape", "commitment_id": "c1", "details": details });
    let held = hook(h, "reservation.reserve", reserve).await;
    assert_shape("reservation_reserve", &held["result"]);

    let key = json!({ "agreement": "rec_shape", "commitment_id": "c1" });
    let confirmed = hook(h, "reservation.confirm", key.clone()).await;
    assert_shape("reservation_confirm", &confirmed["result"]);

    let taken = json!({ "agreement": "rec_other", "commitment_id": "c1", "details": details });
    let refused = hook(h, "reservation.reserve", taken).await;
    assert_shape("reservation_reserve_slot_taken", &refused["error"]);

    let released = hook(h, "reservation.release", key.clone()).await;
    assert_shape("reservation_release", &released["result"]);
    let again = hook(h, "reservation.release", key).await;
    assert_shape("reservation_release_again", &again["result"]);
}

#[tokio::test]
async fn scheduling_shapes() {
    let h = harness().await;

    let ping = rpc(&h, "scheduling.ping", json!({})).await;
    assert_shape("scheduling_ping", &ping);

    let (unconf, inst, conf) = enrol_native_signing(&h, "scheduling").await;
    assert_shape("scheduling_signing_status_unconfigured", &unconf);
    assert_shape("scheduling_install_signing_certificate", &inst);
    assert_shape("scheduling_signing_status", &conf);

    availability_shapes(&h, "lst_shape_listing").await;
    reservation_shapes(&h).await;

    let exp = rpc(&h, "scheduling.export", json!({})).await;
    assert_shape("scheduling_export", &exp);

    let imp = rpc(&h, "scheduling.import", json!({ "bundle": exp })).await;
    assert_shape("scheduling_import", &imp);
}
