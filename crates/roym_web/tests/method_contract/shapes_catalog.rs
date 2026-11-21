use serde_json::json;

use super::{
    fixtures::full_listing_params,
    helpers::{Harness, harness},
    shapes::{assert_shape, enrol_native_signing, rpc},
};

async fn catalog_admin_shapes(h: &Harness) {
    let ping = rpc(h, "listing.ping", json!({})).await;
    assert_shape("listing_ping", &ping);

    let lim = rpc(h, "listing.limits", json!({})).await;
    assert_shape("listing_limits", &lim);

    let set_lim =
        rpc(h, "listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 10 })).await;
    assert_shape("listing_set_limits", &set_lim);

    let (unconf, inst, conf) = enrol_native_signing(h, "catalog").await;
    assert_shape("catalog_signing_status_unconfigured", &unconf);
    assert_shape("catalog_install_signing_certificate", &inst);
    assert_shape("catalog_signing_status", &conf);
}

async fn listing_shapes(h: &Harness) {
    let list_empty = rpc(h, "listing.list", json!({})).await;
    assert_shape("listing_list_empty", &list_empty);

    let get_missing = rpc(h, "listing.get", json!({ "listing_id": "nonexistent" })).await;
    assert_shape("listing_get_missing", &get_missing);

    let p1 = full_listing_params("lawn-mowing", "Lawn Mowing");
    let set_v1 = rpc(h, "listing.set", p1).await;
    assert_shape("listing_set_new", &set_v1);
    let listing_id = set_v1["listing_id"].as_str().unwrap().to_string();

    let p2 = full_listing_params("lawn-mowing", "Lawn Mowing Professional");
    let set_v2 = rpc(h, "listing.set", p2).await;
    assert_shape("listing_set_second_version", &set_v2);

    let get_found = rpc(h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_shape("listing_get_found", &get_found);
    let env_str = get_found["envelope"].as_str().unwrap().to_string();

    let hist = rpc(h, "listing.history", json!({ "listing_id": listing_id })).await;
    assert_shape("listing_history", &hist);

    let ver_good = rpc(h, "listing.verify", json!({ "envelope": env_str })).await;
    assert_shape("listing_verify_good", &ver_good);

    let mut tampered = env_str.clone();
    if let Some(pos) = tampered.find("Lawn Mowing") {
        tampered.replace_range(pos..pos + 4, "Tree");
    }
    let ver_tampered = rpc(h, "listing.verify", json!({ "envelope": tampered })).await;
    assert_shape("listing_verify_tampered", &ver_tampered);

    let p_active = full_listing_params("snow-clearing", "Snow Clearing");
    rpc(h, "listing.set", p_active).await;

    let withdraw = rpc(h, "listing.withdraw", json!({ "listing_id": listing_id })).await;
    assert_shape("listing_withdraw", &withdraw);

    let list_rows = rpc(h, "listing.list", json!({})).await;
    assert_shape("listing_list_with_rows", &list_rows);
}

#[tokio::test]
async fn catalog_shapes() {
    let h = harness().await;
    catalog_admin_shapes(&h).await;
    listing_shapes(&h).await;

    let exp = rpc(&h, "catalog.export", json!({})).await;
    assert_shape("catalog_export", &exp);

    let imp = rpc(&h, "catalog.import", json!({ "bundle": exp })).await;
    assert_shape("catalog_import", &imp);
}
