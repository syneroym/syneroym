#![cfg(feature = "dual_build_fixture")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Proves the `dual_build_fixture` feature's substrate wiring end to end --
//! a real client reaching the linked-in native fixture through the router,
//! over the same `SyneroymClient::request` path any other native or WASM
//! service is reached through. The in-process
//! `crates/app_host_native/tests/dual_build_parity.rs` suite proves the
//! shim itself; this proves the registration.

mod common;

use common::{SubstrateTestContext, alloc_ports};
use serde_json::json;
use syneroym_test_dual_build_fixture::native::FIXTURE_INTERFACE;

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
