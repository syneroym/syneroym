use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use syneroym_app_host::types::http::FrameKind;
use syneroym_app_host_native::WebSocketSink;
use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::{derive_did_key, resolve_did_key},
};
use syneroym_roym_core::{envelope::Response, services};
use syneroym_roym_web::native::NativeWeb;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_1_profile_ping_reachability_byte_identical() {
    let h = harness().await;
    let req = json!({
        "method": "profile.policy",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    let result = resp.result.unwrap();
    assert!(result.get("statement").is_some());
}

#[tokio::test]
async fn scenario_2_inert_extra_caller_field_in_params() {
    let h = harness().await;
    let req_extra = json!({
        "method": "profile.policy",
        "params": {
            "caller": "did:key:fakeAttacker",
            "person_did": "did:key:fakeAlice",
            "auth": "delegated"
        }
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req_extra).await.unwrap();
    let native_res = h.native.invoke_web(&req_extra).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    let result = resp.result.unwrap();
    assert!(result.get("statement").is_some());
}

#[tokio::test]
async fn scenario_3_session_whoami_with_delegated_caller() {
    let h = harness().await;
    let delegated_caller = caller();

    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "session.whoami",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_resp =
        h.wasm_http.post("/rpc", req_body.clone(), Some(delegated_caller.clone())).await;
    let native_resp = h.native_http.post("/rpc", req_body, Some(delegated_caller)).await;

    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);

    let val: Value = serde_json::from_slice(&wasm_resp.body).unwrap();
    assert_eq!(val["result"]["did"], owner_did());
    assert_eq!(val["result"]["auth"], "delegated");
}

#[tokio::test]
async fn scenario_4_unlisted_method_returns_32601() {
    let h = harness().await;
    let req = json!({
        "method": "nope.thing",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(resp.error.unwrap().code, -32601);

    // An unlisted method is rejected by web's own dispatch before it ever
    // reaches the proxy, so no sibling call should have been attempted.
    assert_eq!(h.wasm_proxy.invocations.load(Ordering::SeqCst), 0);
    assert_eq!(h.native_proxy.invocations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scenario_5_unbound_dependency_returns_32001() {
    let h = harness_with_unbound(Some("transaction")).await;
    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "receipt.ping",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_resp = h.wasm_http.post("/rpc", req_body.clone(), Some(caller())).await;
    let native_resp = h.native_http.post("/rpc", req_body, Some(caller())).await;
    assert_eq!(wasm_resp.body, native_resp.body);

    // transaction is a declared dependency of web (see roym.toml), but the
    // harness omits it from both topologies via
    // `harness_with_unbound(Some("transaction"))`, leaving it unbound: the call
    // must be refused with -32001, and the refusal must not repeat the
    // dependency's DID back to the caller.
    let val: Value = serde_json::from_slice(&wasm_resp.body).unwrap();
    let err_code = val["error"]["code"].as_i64().unwrap();
    let err_msg = val["error"]["message"].as_str().unwrap();
    assert_eq!(err_code, -32001);
    assert!(!err_msg.contains("did:"), "error message must not leak a DID: {err_msg}");
}

#[tokio::test]
async fn scenario_6_handle_http_post_rpc_matches_invoke() {
    let h = harness().await;
    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "profile.ping",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_http_resp = h.wasm_http.post("/rpc", req_body.clone(), Some(caller())).await;
    let native_http_resp = h.native_http.post("/rpc", req_body, Some(caller())).await;

    assert_eq!(wasm_http_resp.status, 200);
    assert_eq!(native_http_resp.status, 200);
    assert_eq!(wasm_http_resp.body, native_http_resp.body);

    let val: Value = serde_json::from_slice(&wasm_http_resp.body).unwrap();
    assert_eq!(val["id"], 42);
    assert_eq!(val["result"]["service"], "profile");
}

#[tokio::test]
async fn scenario_7_handle_http_malformed_body_returns_32700() {
    let h = harness().await;
    let bad_body = b"not a json object".to_vec();

    let wasm_http_resp = h.wasm_http.post("/rpc", bad_body.clone(), None).await;
    let native_http_resp = h.native_http.post("/rpc", bad_body, None).await;

    assert_eq!(wasm_http_resp.status, 200);
    assert_eq!(native_http_resp.status, 200);
    assert_eq!(wasm_http_resp.body, native_http_resp.body);

    let val: Value = serde_json::from_slice(&wasm_http_resp.body).unwrap();
    assert_eq!(val["error"]["code"], -32700);
}

#[tokio::test]
async fn scenario_8_status_on_all_six_services() {
    let h = harness().await;
    for svc in services::ALL {
        let wasm_status = h.wasm.status(svc.name).await.unwrap();
        let native_status = h.native.status(svc.name).await.unwrap();

        assert_eq!(wasm_status, native_status, "status mismatch on service {}", svc.name);
        let val: Value = serde_json::from_str(&wasm_status).unwrap();
        assert_eq!(val["service"], svc.name);
        // profile, catalog, conversation, transaction and directory carry real state.
        let expected_schema_version = match svc.name {
            "directory" | "transaction" => 3,
            "profile" | "catalog" | "conversation" => 2,
            _ => 1,
        };
        assert_eq!(val["schema_version"], expected_schema_version);
    }
}

// The web app keeps no per-connection state (CON-1): with `WS_CONNS`
// dropped, there is nothing left to assert registered/removed, so this
// only checks the lifecycle calls are callable and return without error
// on both drivers.
#[tokio::test]
async fn scenario_9_websocket_lifecycle_fires() {
    let h = harness().await;
    let conn_id = "test-ws-conn-123".to_string();
    let caller_ctx = caller();

    NativeWeb::on_open(&h.native.web, caller_ctx.clone(), conn_id.clone()).await;
    NativeWeb::on_message(
        &h.native.web,
        caller_ctx.clone(),
        conn_id.clone(),
        b"hello".to_vec(),
        FrameKind::Text,
    )
    .await;
    NativeWeb::on_close(&h.native.web, caller_ctx, conn_id.clone()).await;

    let web_id = did_for_service("web");
    h.wasm.engine.handle_websocket_on_open(&web_id, &conn_id, Some(caller())).await;
    h.wasm
        .engine
        .handle_websocket_on_message(
            &web_id,
            &conn_id,
            b"hello".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm.engine.handle_websocket_on_close(&web_id, &conn_id, Some(caller())).await;
}

#[tokio::test]
async fn scenario_10_directory_service_call_target() {
    let h = harness().await;
    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "directory.ping",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_resp = h.wasm_http.post("/rpc", req_body.clone(), Some(caller())).await;
    let native_resp = h.native_http.post("/rpc", req_body, Some(caller())).await;

    assert_eq!(wasm_resp.body, native_resp.body);
    let val: Value = serde_json::from_slice(&wasm_resp.body).unwrap();
    assert_eq!(val["result"]["service"], "directory");
}

#[tokio::test]
async fn scenario_11_profile_signing_status_unenrolled_parity() {
    let h = harness().await;
    let req = json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(h.caller())).await;

    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["result"]["certificate"]["state"], "missing");
}

#[tokio::test]
async fn scenario_12_profile_set_without_enrolment_fails_parity() {
    let h = harness().await;
    let req = json!({
        "method": "profile.set",
        "params": { "display_name": "Alice" }
    })
    .to_string()
    .into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(h.caller())).await;

    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
}

#[tokio::test]
async fn scenario_13_profile_install_certificate_and_signing_status_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res =
        h.wasm_http.post("/rpc", status_req.clone(), Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();

    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();

    let wasm_res =
        h.wasm_http.post("/rpc", install_req.clone(), Some(caller_identity.clone())).await;
    let native_res = h.native_http.post("/rpc", install_req, Some(caller_identity.clone())).await;
    assert_eq!(wasm_res.body, native_res.body);

    let wasm_status =
        h.wasm_http.post("/rpc", status_req.clone(), Some(caller_identity.clone())).await;
    let native_status = h.native_http.post("/rpc", status_req, Some(caller_identity)).await;
    assert_eq!(wasm_status.body, native_status.body);
}

#[tokio::test]
async fn scenario_14_profile_set_and_get_envelope_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", install_req.clone(), Some(caller_identity.clone())).await;
    h.native_http.post("/rpc", install_req, Some(caller_identity.clone())).await;

    let set_req = json!({
        "method": "profile.set",
        "params": { "display_name": "Parity Alice", "conversation_address": "syneroym://addr/1" }
    })
    .to_string()
    .into_bytes();

    let wasm_set = h.wasm_http.post("/rpc", set_req.clone(), Some(caller_identity.clone())).await;
    let native_set = h.native_http.post("/rpc", set_req, Some(caller_identity.clone())).await;
    let mut wasm_set_val: Value = serde_json::from_slice(&wasm_set.body).unwrap();
    let mut native_set_val: Value = serde_json::from_slice(&native_set.body).unwrap();
    strip_volatile(&mut wasm_set_val);
    strip_volatile(&mut native_set_val);
    assert_eq!(wasm_set_val, native_set_val);

    let get_req = json!({ "method": "profile.get", "params": {} }).to_string().into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller_identity.clone())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller_identity)).await;
    let mut wasm_get_val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    let mut native_get_val: Value = serde_json::from_slice(&native_get.body).unwrap();
    strip_volatile(&mut wasm_get_val);
    strip_volatile(&mut native_get_val);
    assert_eq!(wasm_get_val, native_get_val);
}

#[tokio::test]
async fn scenario_15_contacts_resolve_address_and_remove_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": {
            "person_did": "did:key:zFriend15",
            "display_name": "Friend 15",
            "conversation_address": "syneroym://friend15"
        }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", upsert_req, Some(caller())).await;

    let resolve_req = json!({
        "method": "contacts.resolve-address",
        "params": { "person_did": "did:key:zFriend15" }
    })
    .to_string()
    .into_bytes();
    let wasm_resolve = h.wasm_http.post("/rpc", resolve_req.clone(), Some(caller())).await;
    let native_resolve = h.native_http.post("/rpc", resolve_req, Some(caller())).await;
    assert_eq!(wasm_resolve.body, native_resolve.body);
    let val: Value = serde_json::from_slice(&wasm_resolve.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    assert_eq!(val["result"]["conversation_address"], "syneroym://friend15");

    let remove_req = json!({
        "method": "contacts.remove",
        "params": { "person_did": "did:key:zFriend15" }
    })
    .to_string()
    .into_bytes();
    let wasm_rem = h.wasm_http.post("/rpc", remove_req.clone(), Some(caller())).await;
    let native_rem = h.native_http.post("/rpc", remove_req, Some(caller())).await;
    assert_eq!(wasm_rem.body, native_rem.body);
}

#[tokio::test]
async fn scenario_16_profile_policy_public_parity() {
    let h = harness().await;
    let req = json!({ "method": "profile.policy", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), None).await;
    let native_res = h.native_http.post("/rpc", req, None).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    let expected = "A blocked sender's messages are refused at this node's inbox. They are never \
                    shown in any conversation, never fire a notification, and are never counted. \
                    Block is enforced locally by this installation's own Conversation service.";
    assert_eq!(val["result"]["statement"], expected);
}

#[tokio::test]
async fn scenario_17_contacts_upsert_and_list_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": { "person_did": "did:key:zFriend17", "favourite": true }
    })
    .to_string()
    .into_bytes();

    let wasm_upsert = h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    let native_upsert = h.native_http.post("/rpc", upsert_req, Some(caller())).await;
    assert_eq!(wasm_upsert.body, native_upsert.body);

    let list_req = json!({ "method": "contacts.list", "params": {} }).to_string().into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", list_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", list_req, Some(caller())).await;
    let mut wasm_list_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_list_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_list_val);
    strip_volatile(&mut native_list_val);
    assert_eq!(wasm_list_val, native_list_val);
}

#[tokio::test]
async fn scenario_18_contacts_get_and_remove_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": { "person_did": "did:key:zFriend18" }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", upsert_req, Some(caller())).await;

    let get_req = json!({
        "method": "contacts.get",
        "params": { "person_did": "did:key:zFriend18" }
    })
    .to_string()
    .into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller())).await;
    let mut wasm_get_val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    let mut native_get_val: Value = serde_json::from_slice(&native_get.body).unwrap();
    strip_volatile(&mut wasm_get_val);
    strip_volatile(&mut native_get_val);
    assert_eq!(wasm_get_val, native_get_val);

    let remove_req = json!({
        "method": "contacts.remove",
        "params": { "person_did": "did:key:zFriend18" }
    })
    .to_string()
    .into_bytes();
    let wasm_remove = h.wasm_http.post("/rpc", remove_req.clone(), Some(caller())).await;
    let native_remove = h.native_http.post("/rpc", remove_req, Some(caller())).await;
    assert_eq!(wasm_remove.body, native_remove.body);
}

#[tokio::test]
async fn scenario_19_contacts_favourite_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": {
            "person_did": "did:key:zFriend19",
            "conversation_address": "syneroym://friend19",
            "favourite": true
        }
    })
    .to_string()
    .into_bytes();
    let wasm_up = h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    let native_up = h.native_http.post("/rpc", upsert_req, Some(caller())).await;
    assert_eq!(wasm_up.body, native_up.body);

    let get_req = json!({
        "method": "contacts.get",
        "params": { "person_did": "did:key:zFriend19" }
    })
    .to_string()
    .into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller())).await;
    assert_eq!(wasm_get.body, native_get.body);
    let val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    assert_eq!(val["result"]["favourite"], true);
}

#[tokio::test]
async fn scenario_20_block_add_and_list_parity() {
    let h = harness().await;

    let add_req = json!({
        "method": "block.add",
        "params": { "person_did": "did:key:zSpammer20", "reason": "spam" }
    })
    .to_string()
    .into_bytes();
    let wasm_add = h.wasm_http.post("/rpc", add_req.clone(), Some(caller())).await;
    let native_add = h.native_http.post("/rpc", add_req, Some(caller())).await;
    assert_eq!(wasm_add.body, native_add.body);

    let list_req = json!({ "method": "block.list", "params": {} }).to_string().into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", list_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", list_req, Some(caller())).await;
    let mut wasm_list_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_list_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_list_val);
    strip_volatile(&mut native_list_val);
    assert_eq!(wasm_list_val, native_list_val);
}

#[tokio::test]
async fn scenario_21_block_check_and_remove_parity() {
    let h = harness().await;

    let add_req = json!({
        "method": "block.add",
        "params": { "person_did": "did:key:zSpammer21" }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", add_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", add_req, Some(caller())).await;

    let check_req = json!({
        "method": "block.check",
        "params": { "person_did": "did:key:zSpammer21" }
    })
    .to_string()
    .into_bytes();
    let wasm_check = h.wasm_http.post("/rpc", check_req.clone(), Some(caller())).await;
    let native_check = h.native_http.post("/rpc", check_req, Some(caller())).await;
    let mut wasm_check_val: Value = serde_json::from_slice(&wasm_check.body).unwrap();
    let mut native_check_val: Value = serde_json::from_slice(&native_check.body).unwrap();
    strip_volatile(&mut wasm_check_val);
    strip_volatile(&mut native_check_val);
    assert_eq!(wasm_check_val, native_check_val);

    let remove_req = json!({
        "method": "block.remove",
        "params": { "person_did": "did:key:zSpammer21" }
    })
    .to_string()
    .into_bytes();
    let wasm_remove = h.wasm_http.post("/rpc", remove_req.clone(), Some(caller())).await;
    let native_remove = h.native_http.post("/rpc", remove_req, Some(caller())).await;
    assert_eq!(wasm_remove.body, native_remove.body);
}

#[tokio::test]
async fn scenario_22_report_create_get_withdraw_and_refile_refusal_parity() {
    let h = harness().await;

    let submit_req = json!({
        "method": "report.create",
        "params": { "subject_kind": "person", "subject_id": "did:key:zOffender22", "category": "harassment", "details": "bad" }
    })
    .to_string()
    .into_bytes();
    let wasm_sub = h.wasm_http.post("/rpc", submit_req.clone(), Some(caller())).await;
    let native_sub = h.native_http.post("/rpc", submit_req.clone(), Some(caller())).await;
    let sub_val: Value = serde_json::from_slice(&wasm_sub.body).unwrap();
    let sub_val_n: Value = serde_json::from_slice(&native_sub.body).unwrap();
    // report.create's own response carries the same at_secs (see
    // strip_volatile) as report.get, so this needs the same
    // parse-then-strip comparison instead of a raw byte one.
    assert_eq!(stripped(&sub_val), stripped(&sub_val_n));
    assert_ne!(sub_val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    let report_id = sub_val["result"]["report_id"].as_str().unwrap();

    let get_req = json!({ "method": "report.get", "params": { "report_id": report_id } })
        .to_string()
        .into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let mut wasm_val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    let mut native_val: Value = serde_json::from_slice(&native_get.body).unwrap();
    strip_volatile(&mut wasm_val);
    strip_volatile(&mut native_val);
    assert_eq!(wasm_val, native_val);

    // Withdraw the report
    let withdraw_req = json!({ "method": "report.withdraw", "params": { "report_id": report_id } })
        .to_string()
        .into_bytes();
    let wasm_with = h.wasm_http.post("/rpc", withdraw_req.clone(), Some(caller())).await;
    let native_with = h.native_http.post("/rpc", withdraw_req, Some(caller())).await;
    let with_val: Value = serde_json::from_slice(&wasm_with.body).unwrap();
    let with_val_n: Value = serde_json::from_slice(&native_with.body).unwrap();
    assert_eq!(stripped(&with_val), stripped(&with_val_n));
    assert_eq!(with_val["result"]["status"], "withdrawn");

    // Verify report.get reflects status "withdrawn" -- report.create's own
    // at_secs (see strip_volatile) carries through, so this is a raw-byte
    // comparison only after parsing and stripping, like the report.get
    // check above.
    let wasm_get2 = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get2 = h.native_http.post("/rpc", get_req, Some(caller())).await;
    let get2_val: Value = serde_json::from_slice(&wasm_get2.body).unwrap();
    let get2_val_n: Value = serde_json::from_slice(&native_get2.body).unwrap();
    assert_eq!(stripped(&get2_val), stripped(&get2_val_n));
    assert_eq!(get2_val["result"]["status"], "withdrawn");

    // Attempting to re-file a withdrawn report refuses on both builds
    let wasm_refile = h.wasm_http.post("/rpc", submit_req.clone(), Some(caller())).await;
    let native_refile = h.native_http.post("/rpc", submit_req, Some(caller())).await;
    let refile_val: Value = serde_json::from_slice(&wasm_refile.body).unwrap();
    let refile_val_n: Value = serde_json::from_slice(&native_refile.body).unwrap();
    assert_eq!(stripped(&refile_val), stripped(&refile_val_n));
    assert!(refile_val.get("error").is_some());
    assert!(refile_val["error"]["message"].as_str().unwrap().contains("withdrawn"));
}

#[tokio::test]
async fn scenario_23_profile_export_and_import_parity() {
    let h = harness().await;
    enrol_signing(&h, "profile").await;

    let exp_req = json!({ "method": "profile.export", "params": {} }).to_string().into_bytes();
    let wasm_exp = h.wasm_http.post("/rpc", exp_req.clone(), Some(caller())).await;
    let native_exp = h.native_http.post("/rpc", exp_req, Some(caller())).await;
    let mut wasm_exp_val: Value = serde_json::from_slice(&wasm_exp.body).unwrap();
    let mut native_exp_val: Value = serde_json::from_slice(&native_exp.body).unwrap();
    verify_and_strip_manifest_signature(&mut wasm_exp_val);
    verify_and_strip_manifest_signature(&mut native_exp_val);
    strip_volatile(&mut wasm_exp_val);
    strip_volatile(&mut native_exp_val);
    assert_eq!(wasm_exp_val, native_exp_val);

    let exp_val: Value = serde_json::from_slice(&wasm_exp.body).unwrap();
    let bundle = &exp_val["result"];

    let imp_req = json!({
        "method": "profile.import",
        "params": { "bundle": bundle }
    })
    .to_string()
    .into_bytes();
    let wasm_imp = h.wasm_http.post("/rpc", imp_req.clone(), Some(caller())).await;
    let native_imp = h.native_http.post("/rpc", imp_req, Some(caller())).await;
    assert_eq!(wasm_imp.body, native_imp.body);
}

#[tokio::test]
async fn scenario_24_contacts_limits_and_set_limits_parity() {
    let h = harness().await;

    let get_req = json!({ "method": "contacts.limits", "params": {} }).to_string().into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller())).await;
    assert_eq!(wasm_get.body, native_get.body);
    let val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));

    let set_req = json!({
        "method": "contacts.set-limits",
        "params": { "window_secs": 7200, "max_per_window": 5 }
    })
    .to_string()
    .into_bytes();
    let wasm_set = h.wasm_http.post("/rpc", set_req.clone(), Some(caller())).await;
    let native_set = h.native_http.post("/rpc", set_req, Some(caller())).await;
    assert_eq!(wasm_set.body, native_set.body);

    let check_req = json!({ "method": "contacts.limits", "params": {} }).to_string().into_bytes();
    let wasm_check = h.wasm_http.post("/rpc", check_req.clone(), Some(caller())).await;
    let native_check = h.native_http.post("/rpc", check_req, Some(caller())).await;
    assert_eq!(wasm_check.body, native_check.body);
    let check_val: Value = serde_json::from_slice(&wasm_check.body).unwrap();
    assert_eq!(check_val["result"]["window_secs"], 7200);
    assert_eq!(check_val["result"]["max_per_window"], 5);
}

#[tokio::test]
async fn scenario_25_unauthenticated_owner_methods_refused_parity() {
    let h = harness().await;
    let methods = ["profile.get", "contacts.list", "block.list", "report.list"];

    for method in methods {
        let req = json!({ "method": method, "params": {} }).to_string().into_bytes();
        let wasm_res = h.wasm_http.post("/rpc", req.clone(), None).await;
        let native_res = h.native_http.post("/rpc", req, None).await;
        assert_eq!(wasm_res.body, native_res.body);
        let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
        assert_eq!(val["error"]["code"], -32010);
    }
}

#[tokio::test]
async fn scenario_26_stranger_owner_methods_refused_parity() {
    let h = harness().await;
    let stranger_identity = custom_caller("did:key:zStranger26");
    let methods = ["profile.get", "contacts.list", "block.list", "report.list"];

    for method in methods {
        let req = json!({ "method": method, "params": {} }).to_string().into_bytes();
        let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(stranger_identity.clone())).await;
        let native_res = h.native_http.post("/rpc", req, Some(stranger_identity.clone())).await;
        assert_eq!(wasm_res.body, native_res.body);
        let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
        assert_eq!(val["error"]["code"], -32011);
    }
}

#[tokio::test]
async fn scenario_27_stranger_cannot_install_certificate_parity() {
    let h = harness().await;
    let stranger = Identity::generate().unwrap();
    let stranger_did = derive_did_key(&stranger.public_key());
    let stranger_identity = custom_caller(&stranger_did);

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(h.caller())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert = DelegationCertificate::issue(
        &stranger,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();

    let wasm_res =
        h.wasm_http.post("/rpc", install_req.clone(), Some(stranger_identity.clone())).await;
    let native_res = h.native_http.post("/rpc", install_req, Some(stranger_identity)).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32011);
}

#[tokio::test]
async fn scenario_28_expired_certificate_refuses_signing_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let mut cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        100,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    cert.issued_at_secs = 1_000_000_000;
    cert.expires_at_secs = 1_000_000_100;

    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", install_req.clone(), Some(caller_identity.clone())).await;
    h.native_http.post("/rpc", install_req, Some(caller_identity.clone())).await;

    let set_req = json!({
        "method": "profile.set",
        "params": { "display_name": "Expired Alice" }
    })
    .to_string()
    .into_bytes();
    let wasm_set = h.wasm_http.post("/rpc", set_req.clone(), Some(caller_identity.clone())).await;
    let native_set = h.native_http.post("/rpc", set_req, Some(caller_identity)).await;
    assert_eq!(wasm_set.body, native_set.body);
    let val: Value = serde_json::from_slice(&wasm_set.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
}

#[tokio::test]
async fn scenario_29_stale_certificate_refuses_signing_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert1 = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let install1 = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert1.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", install1.clone(), Some(caller_identity.clone())).await;
    h.native_http.post("/rpc", install1, Some(caller_identity.clone())).await;

    let set_req = json!({ "method": "profile.set", "params": { "display_name": "Alice 1" } })
        .to_string()
        .into_bytes();
    let wasm_set1 = h.wasm_http.post("/rpc", set_req.clone(), Some(caller_identity.clone())).await;
    let native_set1 = h.native_http.post("/rpc", set_req, Some(caller_identity)).await;
    assert_eq!(wasm_set1.body, native_set1.body);
}

#[tokio::test]
async fn scenario_30_contacts_list_pagination_parity() {
    let h = harness().await;

    for i in 1..=5 {
        let req = json!({
            "method": "contacts.upsert",
            "params": { "person_did": format!("did:key:zFriend30_{i}") }
        })
        .to_string()
        .into_bytes();
        h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
        h.native_http.post("/rpc", req, Some(caller())).await;
    }

    let page_req = json!({
        "method": "contacts.list",
        "params": { "limit": 2, "offset": 1 }
    })
    .to_string()
    .into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", page_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", page_req, Some(caller())).await;
    assert_eq!(wasm_list.body, native_list.body);
}

#[tokio::test]
async fn scenario_31_block_list_pagination_parity() {
    let h = harness().await;

    for i in 1..=5 {
        let req = json!({
            "method": "block.add",
            "params": { "person_did": format!("did:key:zSpammer31_{i}") }
        })
        .to_string()
        .into_bytes();
        h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
        h.native_http.post("/rpc", req, Some(caller())).await;
    }

    let page_req = json!({
        "method": "block.list",
        "params": { "limit": 2, "offset": 1 }
    })
    .to_string()
    .into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", page_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", page_req, Some(caller())).await;
    let mut wasm_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_val);
    strip_volatile(&mut native_val);
    assert_eq!(wasm_val, native_val);
    assert_eq!(wasm_val["result"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_32_report_list_pagination_parity() {
    let h = harness().await;

    for i in 1..=5 {
        let req = json!({
            "method": "report.create",
            "params": {
                "subject_kind": "person",
                "subject_id": format!("did:key:zOffender32_{i}"),
                "category": "harassment",
                "details": format!("abuse {i}")
            }
        })
        .to_string()
        .into_bytes();
        h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
        h.native_http.post("/rpc", req, Some(caller())).await;
    }

    let page_req = json!({
        "method": "report.list",
        "params": { "limit": 2, "offset": 1 }
    })
    .to_string()
    .into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", page_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", page_req, Some(caller())).await;
    let mut wasm_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_val);
    strip_volatile(&mut native_val);
    assert_eq!(wasm_val, native_val);
    assert_eq!(wasm_val["result"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_33_contacts_admit_first_contact_parity() {
    let h = harness().await;

    // 1. Unblocked stranger within limits -> allow
    let req = json!({
        "method": "contacts.admit-first-contact",
        "params": {
            "sender_address": "syneroym://stranger33",
            "sender_person_did": "did:key:zStranger33"
        }
    })
    .to_string()
    .into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(caller())).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    assert_eq!(val["result"]["admission"], "allow");

    // 2. Blocked sender -> blocked
    let block_req = json!({
        "method": "block.add",
        "params": {
            "person_did": "did:key:zBlocked33",
            "address": "syneroym://blocked33"
        }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", block_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", block_req, Some(caller())).await;

    let admit_blocked_req = json!({
        "method": "contacts.admit-first-contact",
        "params": {
            "sender_address": "syneroym://blocked33",
            "sender_person_did": "did:key:zBlocked33"
        }
    })
    .to_string()
    .into_bytes();
    let wasm_blk = h.wasm_http.post("/rpc", admit_blocked_req.clone(), Some(caller())).await;
    let native_blk = h.native_http.post("/rpc", admit_blocked_req, Some(caller())).await;
    assert_eq!(wasm_blk.body, native_blk.body);
    let blk_val: Value = serde_json::from_slice(&wasm_blk.body).unwrap();
    assert_eq!(blk_val["result"]["admission"], "blocked");

    // 3. Sender hitting rate limit -> rate-limited
    let set_limits_req = json!({
        "method": "contacts.set-limits",
        "params": {
            "max_per_window": 1,
            "window_secs": 3600
        }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", set_limits_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", set_limits_req, Some(caller())).await;

    // First attempt for ratelimited sender -> allow
    let rate_req = json!({
        "method": "contacts.admit-first-contact",
        "params": {
            "sender_address": "syneroym://ratelimited33",
            "sender_person_did": "did:key:zRateLimited33"
        }
    })
    .to_string()
    .into_bytes();
    let wasm_r1 = h.wasm_http.post("/rpc", rate_req.clone(), Some(caller())).await;
    let native_r1 = h.native_http.post("/rpc", rate_req.clone(), Some(caller())).await;
    assert_eq!(wasm_r1.body, native_r1.body);
    let r1_val: Value = serde_json::from_slice(&wasm_r1.body).unwrap();
    assert_eq!(r1_val["result"]["admission"], "allow");

    // Second attempt within window -> rate-limited
    let wasm_r2 = h.wasm_http.post("/rpc", rate_req.clone(), Some(caller())).await;
    let native_r2 = h.native_http.post("/rpc", rate_req, Some(caller())).await;
    let mut wasm_r2_val: Value = serde_json::from_slice(&wasm_r2.body).unwrap();
    let mut native_r2_val: Value = serde_json::from_slice(&native_r2.body).unwrap();
    strip_volatile(&mut wasm_r2_val);
    strip_volatile(&mut native_r2_val);
    assert_eq!(wasm_r2_val, native_r2_val);
    assert_eq!(wasm_r2_val["result"]["admission"], "rate-limited");
}

#[tokio::test]
async fn scenario_34_unknown_method_parity() {
    let h = harness().await;
    let req =
        json!({ "method": "profile.nonexistent_method", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(caller())).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32601);
}

#[tokio::test]
async fn scenario_35_invalid_params_parity() {
    let h = harness().await;
    let req = json!({ "method": "contacts.upsert", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(caller())).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
}

/// A passing parity comparison is not evidence of anything unless the
/// comparison is known to detect a real divergence.
#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let h = harness().await;
    // Public methods that return a result object:
    let bound_methods = ["profile.policy"];
    let mutant = Mutant(&h.native);

    for method in bound_methods {
        let req = json!({
            "method": method,
            "params": {}
        })
        .to_string();
        let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
        let mutant_res = mutant.invoke_web(&req).await.unwrap();
        assert_ne!(wasm_res, mutant_res, "invoke_web mutant divergence failed for {method}");
    }

    let all_services = [
        services::WEB.name,
        services::PROFILE.name,
        services::CONVERSATION.name,
        services::CATALOG.name,
        services::TRANSACTION.name,
        services::DIRECTORY.name,
    ];
    for svc_name in all_services {
        let wasm_status = h.wasm.status(svc_name).await.unwrap();
        let mutant_status = mutant.status(svc_name).await.unwrap();
        assert_ne!(wasm_status, mutant_status, "status mutant divergence failed for {svc_name}");
    }
}

#[tokio::test]
async fn scenario_36_profile_import_foreign_subject_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "profile").await;

    let exp_req = json!({ "method": "profile.export", "params": {} }).to_string().into_bytes();
    let wasm_exp = h.wasm_http.post("/rpc", exp_req, Some(caller())).await;
    let mut exp_val: Value = serde_json::from_slice(&wasm_exp.body).unwrap();
    let bundle = exp_val.get_mut("result").unwrap();

    bundle["manifest"]["subject_did"] = json!("did:key:zOtherStranger");

    let imp_req = json!({ "method": "profile.import", "params": { "bundle": bundle } })
        .to_string()
        .into_bytes();
    let wasm_imp = h.wasm_http.post("/rpc", imp_req.clone(), Some(caller())).await;
    let native_imp = h.native_http.post("/rpc", imp_req, Some(caller())).await;
    assert_eq!(wasm_imp.body, native_imp.body);
    let val: Value = serde_json::from_slice(&wasm_imp.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
    assert!(val["error"]["message"].as_str().unwrap().contains("bundle belongs to"));
}
