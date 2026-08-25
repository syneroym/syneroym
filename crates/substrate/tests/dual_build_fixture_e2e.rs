#![cfg(feature = "dual_build_fixture")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Proves the `dual_build_fixture` feature's substrate wiring end to end --
//! a real client reaching the linked-in native fixture through the router,
//! over the same `SyneroymClient::request` path any other native or WASM
//! service is reached through. The in-process
//! `crates/app_host_native/tests/dual_build_parity.rs` suite proves the
//! shim itself; this proves the registration.

mod common;

use std::time::Duration;

use common::{SubstrateTestContext, alloc_ports};
use serde_json::json;
use syneroym_sdk::TransportConnection;
use syneroym_test_dual_build_fixture::native::FIXTURE_INTERFACE;
use tokio::{io::AsyncWriteExt, time};

#[tokio::test]
async fn a_client_reaches_the_linked_native_fixture_through_the_router() {
    let [iroh_port, registry_port, gateway_port] = alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;
    // The fixture's own `data-layer` calls (`create-collection`/`put`/
    // `query`) go through the same `SqliteStorageProvider` every deployed
    // service shares, which fails closed with no KEK injected -- same as
    // every other e2e test that touches data-layer through a real substrate.
    ctx.substrate_client.inject_kek("40".repeat(32)).await.expect("inject_kek failed");

    let response = ctx
        .substrate_client
        .request(FIXTURE_INTERFACE, "run", json!([r#"{"op":"store-messages","count":3}"#]))
        .await
        .expect("request to the linked-in native fixture");

    let payload: String =
        serde_json::from_value(response.result).expect("fixture's payload is a JSON string");
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).expect("fixture's payload is itself JSON");
    assert_eq!(parsed["ok"]["written"], 3);
    assert_eq!(parsed["ok"]["read"], 3);

    ctx.teardown().await;
}

#[tokio::test]
async fn an_http_request_reaches_the_linked_native_fixture_through_the_router() {
    let [iroh_port, registry_port, gateway_port] = alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;
    ctx.substrate_client.inject_kek("40".repeat(32)).await.expect("inject_kek failed");

    let node_did = ctx.substrate_client.service_id().to_string();
    let TransportConnection::Iroh { conn, .. } =
        ctx.substrate_client.connection().expect("iroh connection");
    let (mut send, mut recv) = conn.open_bi().await.unwrap();

    let preamble = format!("http://http-native|{node_did}\n");
    send.write_all(preamble.as_bytes()).await.unwrap();
    let http_req = "GET /whoami HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    send.write_all(http_req.as_bytes()).await.unwrap();
    send.shutdown().await.unwrap();

    let buf = recv.read_to_end(64 * 1024).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    assert!(
        resp.contains("200 OK") || resp.contains("HTTP/1.1 200"),
        "unexpected HTTP response: {resp}"
    );
    assert!(resp.contains("anonymous"), "unexpected body: {resp}");

    ctx.teardown().await;
}

#[tokio::test]
async fn a_cross_service_call_by_did_is_gated_on_native_build() {
    let [iroh_port, registry_port, gateway_port] = alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;
    ctx.substrate_client.inject_kek("40".repeat(32)).await.expect("inject_kek failed");

    let target_did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
    let response = ctx
        .substrate_client
        .request(
            FIXTURE_INTERFACE,
            "run",
            json!([serde_json::to_string(&json!({
                "op": "proxy-call-cross-service-native",
                "target": target_did,
                "interface": "data-layer",
                "method": "query",
                "params": "{}",
            }))
            .unwrap()]),
        )
        .await
        .expect("request to native fixture");

    let payload: String = serde_json::from_value(response.result).expect("fixture payload");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("fixture payload JSON");
    let error_str = parsed["ok"]["error"].as_str().expect("expected error field in ok payload");
    assert!(
        error_str.contains("PermissionDenied")
            || error_str.contains("permission-denied")
            || error_str.contains("Permission denied"),
        "expected permission denied error for ungranted cross-service call, got: {error_str}"
    );

    ctx.teardown().await;
}

#[tokio::test]
async fn a_websocket_reaches_the_linked_native_fixture_through_the_router() {
    let [iroh_port, registry_port, gateway_port] = alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;
    ctx.substrate_client.inject_kek("40".repeat(32)).await.expect("inject_kek failed");

    let node_did = ctx.substrate_client.service_id().to_string();
    let TransportConnection::Iroh { conn, .. } =
        ctx.substrate_client.connection().expect("iroh connection");

    // 1. Non-public /ws route without caller must fail with 401 Unauthorized
    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let preamble = format!("http://http-native|{node_did}\n");
        send.write_all(preamble.as_bytes()).await.unwrap();
        let ws_upgrade = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: \
                          websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: \
                          dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        send.write_all(ws_upgrade.as_bytes()).await.unwrap();
        send.shutdown().await.unwrap();

        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            match recv.read(&mut chunk).await {
                Ok(Some(n)) if n > 0 => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                _ => break,
            }
        }
        let resp = String::from_utf8_lossy(&buf);
        assert!(
            resp.contains("401 Unauthorized") || resp.contains("HTTP/1.1 401"),
            "expected 401 on non-public /ws, got: {resp}"
        );
    }

    // 2. Public /ws-public route upgrades successfully to 101 Switching Protocols
    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let preamble = format!("http://http-native|{node_did}\n");
        send.write_all(preamble.as_bytes()).await.unwrap();
        let ws_upgrade = "GET /ws-public HTTP/1.1\r\nHost: localhost\r\nUpgrade: \
                          websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: \
                          dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        send.write_all(ws_upgrade.as_bytes()).await.unwrap();

        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            match recv.read(&mut chunk).await {
                Ok(Some(n)) if n > 0 => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                _ => break,
            }
        }
        let resp = String::from_utf8_lossy(&buf);
        assert!(
            resp.contains("101 Switching Protocols") || resp.contains("HTTP/1.1 101"),
            "unexpected upgrade response: {resp}"
        );
    }

    // 3. Verify on_open reached the native fixture by querying the stored WS log
    time::sleep(Duration::from_millis(50)).await;
    let response = ctx
        .substrate_client
        .request(
            "dual-build-fixture",
            "run",
            serde_json::json!([serde_json::to_string(&serde_json::json!({
                "op": "read-ws-log",
            }))
            .unwrap()]),
        )
        .await
        .expect("request to native fixture");

    let payload: String = serde_json::from_value(response.result).expect("fixture payload");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("fixture payload JSON");
    let events = parsed["ok"]["events"].as_array().expect("events array in WS log");
    assert!(
        events.iter().any(|e| e["event"] == "open"),
        "expected on_open event in native fixture WS log: {parsed:?}"
    );

    ctx.teardown().await;
}
