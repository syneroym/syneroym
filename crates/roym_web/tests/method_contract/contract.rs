//! Verification of method contracts, dispatch completeness, off-node refusal,
//! and error parameter validation.

use std::path::Path;

use serde_json::{Value, json};
use syneroym_roym_catalog::api as catalog_api;
use syneroym_roym_conversation::api as conversation_api;
use syneroym_roym_core::{method::check::assert_service_table, services};
use syneroym_roym_profile::api as profile_api;
use syneroym_roym_transaction::api as transaction_api;

use super::{
    fixtures::env,
    helpers::{Harness, HttpDriver, harness},
};

#[test]
fn every_profile_method_has_typed_input_and_output() {
    let table = profile_api::method_table();
    assert_eq!(table.len(), 24);
    assert_service_table(
        services::PROFILE,
        &table,
        &[],
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_profile/src"),
    );
}

#[test]
fn every_conversation_method_has_typed_input_and_output() {
    let table = conversation_api::method_table();
    assert_eq!(table.len(), 24);
    assert_service_table(
        services::CONVERSATION,
        &table,
        &[],
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_conversation/src"),
    );
}

#[test]
fn every_catalog_method_has_typed_input_and_output() {
    let table = catalog_api::method_table();
    assert_eq!(table.len(), 17);
    assert_service_table(
        services::CATALOG,
        &table,
        &[],
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_catalog/src"),
    );
}

#[test]
fn every_transaction_method_has_typed_input_and_output() {
    let table = transaction_api::method_table();
    assert_eq!(table.len(), 41);
    assert_service_table(
        services::TRANSACTION,
        &table,
        &[],
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_transaction/src"),
    );
}

#[tokio::test]
async fn every_listed_method_is_dispatched() {
    let h = harness().await;

    let tables = [
        (services::PROFILE, profile_api::method_table()),
        (services::CONVERSATION, conversation_api::method_table()),
        (services::CATALOG, catalog_api::method_table()),
        (services::TRANSACTION, transaction_api::method_table()),
    ];
    for (service, table) in tables {
        for row in table {
            let (wasm_res, native_res) = h.local_invoke(service, &env(&row.name, json!({}))).await;
            assert_ne!(
                wasm_res.get("error").and_then(|e| e.get("code")).and_then(Value::as_i64),
                Some(-32601),
                "{} method {} wasm must not be method-not-found",
                service.name,
                row.name
            );
            assert_ne!(
                native_res.get("error").and_then(|e| e.get("code")).and_then(Value::as_i64),
                Some(-32601),
                "{} method {} native must not be method-not-found",
                service.name,
                row.name
            );
        }
    }

    let (wasm_bad, native_bad) = h
        .local_invoke(services::CONVERSATION, &env("conversation.no-such-method", json!({})))
        .await;
    assert_eq!(wasm_bad["error"]["code"], -32601);
    assert_eq!(native_bad["error"]["code"], -32601);
}

#[tokio::test]
async fn every_listed_method_is_refused_off_the_node_before_parsing() {
    let h = harness().await;

    let tables = [
        (services::PROFILE, profile_api::method_table()),
        (services::CONVERSATION, conversation_api::method_table()),
        (services::CATALOG, catalog_api::method_table()),
        (services::TRANSACTION, transaction_api::method_table()),
    ];
    for (service, table) in tables {
        for row in table {
            let (wasm_res, native_res) =
                h.wire_invoke(service, &env(&row.name, json!({ "zz": 1 }))).await;
            assert_eq!(
                wasm_res["error"]["code"], -32013,
                "{} method {} wasm must refuse off-node with -32013, got {wasm_res}",
                service.name, row.name
            );
            assert_eq!(
                native_res["error"]["code"], -32013,
                "{} method {} native must refuse off-node with -32013, got {native_res}",
                service.name, row.name
            );
        }
    }
}

async fn post_rpc(h: &Harness, method: &str, params: Option<Value>) -> (Value, Value) {
    let body = match params {
        Some(p) => json!({ "method": method, "params": p }).to_string().into_bytes(),
        None => json!({ "method": method }).to_string().into_bytes(),
    };
    let wasm_post = h.wasm_http.post("/rpc", body.clone(), Some(h.caller())).await;
    let native_post = h.native_http.post("/rpc", body, Some(h.caller())).await;
    let wasm_res: Value = serde_json::from_slice(&wasm_post.body).expect("parse wasm rpc response");
    let native_res: Value =
        serde_json::from_slice(&native_post.body).expect("parse native rpc response");
    assert_eq!(wasm_res, native_res, "parity between wasm and native for {method}");
    (wasm_res, native_res)
}

#[tokio::test]
async fn invalid_field_types_and_unknown_fields_are_rejected() {
    let h = harness().await;

    let (w, _) = post_rpc(
        &h,
        "contacts.upsert",
        Some(json!({
            "person_did": "did:key:zContact",
            "conversation_address": "addr1",
            "favourite": "yes",
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "favourite");
    assert!(w["error"]["message"].as_str().unwrap().contains("invalid type"));

    let (w, _) = post_rpc(
        &h,
        "contacts.upsert",
        Some(json!({
            "person_did": "did:key:zContact",
            "conversation_address": "addr1",
            "nick": "friend",
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "nick");

    let (w, _) = post_rpc(
        &h,
        "contacts.upsert",
        Some(json!({
            "person_did": "did:key:zContact",
            "conversation_address": "addr1",
            "profile_envelope": { "issuer": "did:key:zContact" },
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "profile_envelope");

    let (w, _) = post_rpc(
        &h,
        "conversation.send",
        Some(json!({
            "conversation": "c1",
            "body": 5,
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "body");

    let (w, _) = post_rpc(
        &h,
        "conversation.send",
        Some(json!({
            "conversation": "c1",
            "body": "hello",
            "colour": "blue",
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "colour");
}

async fn post_one<D: HttpDriver>(
    driver: &D,
    h: &Harness,
    method: &str,
    params: Option<Value>,
) -> Value {
    let body = match params {
        Some(p) => json!({ "method": method, "params": p }).to_string().into_bytes(),
        None => json!({ "method": method }).to_string().into_bytes(),
    };
    let post = driver.post("/rpc", body, Some(h.caller())).await;
    serde_json::from_slice(&post.body).expect("parse rpc response")
}

async fn assert_group_info_driver<D: HttpDriver>(driver: &D, h: &Harness) {
    let created = post_one(driver, h, "group.create", Some(json!({}))).await;
    let gid = created["result"]["conversation_id"].as_str().unwrap().to_string();

    let g_info1 = post_one(driver, h, "group.info", Some(json!({ "group": &gid }))).await;
    assert!(g_info1.get("result").is_some());
    assert_eq!(g_info1["result"]["conversation_id"], gid);

    let g_info2 = post_one(driver, h, "group.info", Some(json!({ "conversation": &gid }))).await;
    assert!(g_info2.get("result").is_some());
    assert_eq!(g_info2["result"]["conversation_id"], gid);
}

#[tokio::test]
async fn group_reference_accepts_either_alias_and_rejects_both() {
    let h = harness().await;

    // Both keys at once fails during parameter parse before any group lookup,
    // yielding exact identical error on wasm and native.
    let (dup_res, _) =
        post_rpc(&h, "group.info", Some(json!({ "group": "g1", "conversation": "g1" }))).await;
    assert_eq!(dup_res["error"]["code"], -32602);
    assert_eq!(dup_res["error"]["data"]["field"], "conversation");

    // Test that group.info accepts {"group": id} and {"conversation": id} on both
    // wasm and native
    assert_group_info_driver(&h.wasm_http, &h).await;
    assert_group_info_driver(&h.native_http, &h).await;
}

#[tokio::test]
async fn absent_params_is_accepted_for_no_params_method() {
    let h = harness().await;
    let (ping_res, _) = post_rpc(&h, "profile.ping", None).await;
    assert_eq!(ping_res["result"]["service"], "profile");
}

#[tokio::test]
async fn invalid_field_types_and_unknown_fields_in_catalog_and_transaction_are_rejected() {
    let h = harness().await;

    // listing.get with listing_id: 5
    let (w, _) = post_rpc(&h, "listing.get", Some(json!({ "listing_id": 5 }))).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "listing_id");
    assert!(w["error"]["message"].as_str().unwrap().contains("invalid type"));

    // availability.set with a slot having capacity: "two"
    let (w, _) = post_rpc(
        &h,
        "availability.set",
        Some(json!({
            "listing_id": "lst_test",
            "slots": [{
                "start_secs": 100,
                "end_secs": 200,
                "capacity": "two",
            }],
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "slots[0].capacity");

    // availability.set with unknown slot key colour
    let (w, _) = post_rpc(
        &h,
        "availability.set",
        Some(json!({
            "listing_id": "lst_test",
            "slots": [{
                "start_secs": 100,
                "end_secs": 200,
                "capacity": 1,
                "colour": "blue",
            }],
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "slots[0].colour");

    // listing.verify with envelope as an object
    let (w, _) =
        post_rpc(&h, "listing.verify", Some(json!({ "envelope": { "issuer": "x" } }))).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "envelope");

    // request.list with unknown key agreement
    let (w, _) = post_rpc(&h, "request.list", Some(json!({ "agreement": "agr1" }))).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "agreement");

    // booking.list with agreement
    let (w, _) = post_rpc(&h, "booking.list", Some(json!({ "agreement": "agr1" }))).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "agreement");

    // quote.set with terms.quote_expires_at_secs
    let mut q_params = super::fixtures::valid_quote_params("c1", "rec_req");
    q_params["terms"]["quote_expires_at_secs"] = json!(12345);
    let (w, _) = post_rpc(&h, "quote.set", Some(q_params)).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "terms.quote_expires_at_secs");

    // booking.list with state: "nonsense"
    let (w, _) = post_rpc(&h, "booking.list", Some(json!({ "state": "nonsense" }))).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "state");

    // request.list with params absent works
    let (w, _) = post_rpc(&h, "request.list", None).await;
    assert!(w.get("error").is_none(), "request.list with no params must succeed: {w}");
    assert!(w.get("result").is_some());

    // request.set with malformed tagged area block
    let (w, _) = post_rpc(
        &h,
        "request.set",
        Some(json!({
            "conversation": "c1",
            "description": "Need help",
            "data_use_notice": "standard",
            "area": {
                "kind": "circle",
                "lat_e6": 0,
                "lon_e6": 0,
                "radius_m": "x",
            },
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "area");
    assert_eq!(
        w["error"]["message"].as_str().unwrap(),
        "invalid params: area: invalid type: string \"x\", expected u64"
    );
}
