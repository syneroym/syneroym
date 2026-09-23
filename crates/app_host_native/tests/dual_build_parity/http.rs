use super::helpers::*;

#[tokio::test]
async fn both_builds_answer_an_http_request_identically() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/echo", Some(caller())).await;
    let native_resp = h.native_http.get("/echo", Some(caller())).await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);
    assert_eq!(wasm_resp.headers, native_resp.headers);
}

/// A guest HTTP request is router ingress, so `invocation.caller()` must
/// report the request's real origin and never `internal` -- on **both**
/// builds. The WASM engine sets `InstanceOptions::from_wire()`
/// unconditionally for every guest HTTP request; the native shim's
/// `HttpSink` / `WebSocketSink` are built from `host_for_wire`, matching
/// it. Reverting either -- the engine's `from_wire()` to `default()`, or
/// the shim's `http_host_for` back to `host_for` -- turns one of these
/// answers into `internal` and fails this test.
///
/// No caller -> `anonymous`; a verified delegated caller -> `verified`.
#[tokio::test]
async fn a_guest_http_request_reports_the_same_wire_origin_on_both_builds() {
    let h = harness().await;

    let wasm_anon = h.wasm_http.get("/origin", None).await;
    let native_anon = h.native_http.get("/origin", None).await;
    assert_eq!(wasm_anon.status, 200);
    assert_eq!(native_anon.status, 200);
    assert_eq!(
        String::from_utf8_lossy(&wasm_anon.body),
        "anonymous",
        "a guest HTTP call with no caller must observe `anonymous`, never `internal`"
    );
    assert_eq!(
        wasm_anon.body, native_anon.body,
        "the native shim's HTTP sink must report the same origin as the WASM engine"
    );

    let wasm_verified = h.wasm_http.get("/origin", Some(caller())).await;
    let native_verified = h.native_http.get("/origin", Some(caller())).await;
    assert_eq!(String::from_utf8_lossy(&wasm_verified.body), "verified");
    assert_eq!(wasm_verified.body, native_verified.body);
}

#[tokio::test]
async fn both_builds_persist_host_state_from_an_http_request_identically() {
    let h = harness().await;
    let wasm_resp = h
        .wasm_http
        .post("/store?item1", b"{\"data\":\"stored-via-http\"}".to_vec(), Some(caller()))
        .await;
    let native_resp = h
        .native_http
        .post("/store?item1", b"{\"data\":\"stored-via-http\"}".to_vec(), Some(caller()))
        .await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, b"stored");
    assert_eq!(native_resp.body, b"stored");

    // Read back the persisted state via run() on both builds
    let wasm_res = h.wasm.run(r#"{"op":"read-http-store"}"#).await.unwrap();
    let native_res = h.native.run(r#"{"op":"read-http-store"}"#).await.unwrap();
    assert_eq!(wasm_res, native_res);

    let parsed: Value = serde_json::from_str(&wasm_res).unwrap();
    let entries = parsed["ok"]["entries"].as_array().expect("entries array in http store");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "item1");
    assert_eq!(entries[0]["payload"], "{\"data\":\"stored-via-http\"}");
}

#[tokio::test]
async fn both_builds_render_the_same_caller_for_a_delegated_request() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/whoami", Some(caller())).await;
    let native_resp = h.native_http.get("/whoami", Some(caller())).await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);
}

#[tokio::test]
async fn both_builds_substitute_the_service_itself_for_an_anonymous_public_request() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/whoami", None).await;
    let native_resp = h.native_http.get("/whoami", None).await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);
    assert_eq!(String::from_utf8_lossy(&wasm_resp.body), "anonymous");
}

#[tokio::test]
async fn a_guest_rejection_is_an_ok_with_a_4xx_on_both_builds() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/reject", Some(caller())).await;
    let native_resp = h.native_http.get("/reject", Some(caller())).await;
    assert_eq!(wasm_resp.status, 403);
    assert_eq!(native_resp.status, 403);
    assert_eq!(wasm_resp.body, native_resp.body);
}

#[tokio::test]
async fn a_handler_failure_is_an_err_on_both_builds() {
    let h = harness().await;
    let req = HttpRequest {
        method: "GET".to_string(),
        path: "/fail".to_string(),
        query: String::new(),
        route: "/fail".to_string(),
        path_params: vec![],
        headers: vec![],
        body: vec![],
        caller: Some(CallerIdentity {
            did: caller().caller_did,
            auth: CallerAuth::Delegated,
            app_instance: None,
        }),
    };
    let wasm_res = h.wasm_engine.handle_guest_http_request(SERVICE_ID, &req, Some(caller())).await;
    assert!(matches!(wasm_res, Ok(GuestHttpOutcome::Failed(_)) | Err(_)));
    let native_res = h.native_http.adapter.handle_request(req, Some(caller())).await;
    assert!(native_res.is_err());
}

#[tokio::test]
async fn both_builds_deliver_websocket_frames_to_the_app() {
    let h = harness().await;
    // WASM
    h.wasm_engine.handle_websocket_on_open(SERVICE_ID, "ws-c1", Some(caller())).await;
    h.wasm_engine
        .handle_websocket_on_message(
            SERVICE_ID,
            "ws-c1",
            b"frame1".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm_engine
        .handle_websocket_on_message(
            SERVICE_ID,
            "ws-c1",
            b"frame1".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm_engine
        .handle_websocket_on_message(
            SERVICE_ID,
            "ws-c1",
            b"frame2".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm_engine.handle_websocket_on_close(SERVICE_ID, "ws-c1", Some(caller())).await;
    let wasm_log = h.wasm.run(r#"{"op":"read-ws-log"}"#).await.unwrap();

    // Native
    h.native_http.adapter.on_websocket_open("ws-c1".to_string(), Some(caller())).await;
    h.native_http
        .adapter
        .on_websocket_message(
            "ws-c1".to_string(),
            b"frame1".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.native_http
        .adapter
        .on_websocket_message(
            "ws-c1".to_string(),
            b"frame1".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.native_http
        .adapter
        .on_websocket_message(
            "ws-c1".to_string(),
            b"frame2".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.native_http.adapter.on_websocket_close("ws-c1".to_string(), Some(caller())).await;
    let native_log = h.native.run(r#"{"op":"read-ws-log"}"#).await.unwrap();

    assert_eq!(wasm_log, native_log);
    let parsed: serde_json::Value = serde_json::from_str(&wasm_log).expect("parse wasm_log");
    let events = parsed["ok"]["events"].as_array().expect("events array");
    assert_eq!(events.len(), 5); // open + 3 messages + close
}

#[tokio::test]
async fn both_builds_push_a_frame_to_a_live_connection() {
    let h = harness().await;
    let mut rx_wasm = h.wasm_ws_senders.register(SERVICE_ID, "live-conn");
    let mut rx_native = h.native_ws_senders.register(SERVICE_ID, "live-conn");

    let wasm_res =
        h.wasm.run(r#"{"op":"ws-send","conn":"live-conn","body":"msg-to-live"}"#).await.unwrap();
    let native_res =
        h.native.run(r#"{"op":"ws-send","conn":"live-conn","body":"msg-to-live"}"#).await.unwrap();
    assert_eq!(wasm_res, native_res);

    let msg_wasm = rx_wasm.recv().await.unwrap();
    let msg_native = rx_native.recv().await.unwrap();
    assert_eq!(msg_wasm, msg_native);
}
