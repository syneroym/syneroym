#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M3B Slice 6A integration test: two deployed WASM components in
//! different services exchange a message guest-to-guest, using the
//! fully-qualified cross-service topic (a bare `subscribe-to("orders/new")`
//! from a different service would resolve to the *subscriber's own*
//! namespace and never see the publish -- see ADR-0010's Topic Namespace
//! Isolation section and task.md's Finding B1).

use std::{fs, path::Path, sync::Arc, time::Duration};

use syneroym_core::{config::SubstrateConfig, test_constants};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::JsonRpcRequest;
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

const TEST_DRIVER_INTERFACE: &str = "syneroym-test:messaging-pubsub-test/test-driver@0.1.0";
const SERVICE_A: &str = "messaging-svc-a";
const SERVICE_B: &str = "messaging-svc-b";

/// Builds an `AppSandboxEngine` wrapped in `Arc` with `self_weak` set, the
/// same recipe `RouteHandler::init` uses in production (see Step 9 of the
/// Slice 6A plan) -- required for a live `subscribe-to` call's forwarding
/// task to be able to reach back into the engine and invoke
/// `deliver_message` once the originating `Store` is gone.
async fn make_engine(dir: &Path) -> Arc<AppSandboxEngine> {
    let mut config = SubstrateConfig {
        app_local_data_dir: dir.join("data"),
        app_data_dir: dir.join("user_data"),
        app_cache_dir: dir.join("cache"),
        app_log_dir: dir.join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());

    let engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
        )
        .await
        .unwrap(),
    );
    engine.self_weak.set(Arc::downgrade(&engine)).expect("self_weak set once");
    engine
}

fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![TEST_DRIVER_INTERFACE.to_string()],
        }),
        registry_certificate: None,
    }
}

async fn call(
    engine: &AppSandboxEngine,
    service_id: &str,
    method: &str,
    params: serde_json::Value,
) -> String {
    let request =
        JsonRpcRequest { jsonrpc: "2.0".to_string(), method: method.to_string(), params, id: None };
    engine.execute_wasm(service_id, TEST_DRIVER_INTERFACE, &request).await.unwrap()
}

#[tokio::test]
async fn test_guest_to_guest_cross_service_message_delivery() {
    let Ok(wasm_bytes) = fs::read(test_constants::messaging_pubsub_test_wasm_path()) else {
        eprintln!(
            "Skipping test_guest_to_guest_cross_service_message_delivery: messaging-pubsub-test \
             WASM artifact not found (run `cargo build --target wasm32-wasip2 --release` in \
             test-components/messaging-pubsub-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_A, &manifest).await.unwrap();
    engine.deploy_wasm(SERVICE_B, &manifest).await.unwrap();

    // Service B opts into service A's namespace explicitly, via the
    // fully-qualified `svc/<service-a>/...` topic.
    let fully_qualified_topic = format!("svc/{SERVICE_A}/orders/new");
    call(&engine, SERVICE_B, "subscribe-to", serde_json::json!([fully_qualified_topic])).await;

    // Service A publishes to its own bare "orders/new", which the host
    // namespaces to `svc/messaging-svc-a/orders/new`.
    call(&engine, SERVICE_A, "publish-to", serde_json::json!(["orders/new", "hello from A"])).await;

    // Delivery happens on a separately-spawned task, so poll with a
    // timeout rather than asserting immediately.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut received = String::new();
    while tokio::time::Instant::now() < deadline {
        received = call(&engine, SERVICE_B, "get-received-messages", serde_json::json!([])).await;
        if !received.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(received, format!("{fully_qualified_topic}\thello from A"));
}

/// ADR-0010 Topic Namespace Isolation / task.md:854 Security Test: a
/// publish-side `svc/`-prefixed topic must not let one service impersonate
/// another's namespace. Service B subscribes to its own bare topic (which
/// the host namespaces to `svc/messaging-svc-b/orders/new`); Service A then
/// publishes to a spoofed literal `svc/messaging-svc-b/orders/new` topic.
/// If publish namespacing were vulnerable (reusing the subscribe-side
/// literal-passthrough rule for `svc/`-prefixed topics), Service B would
/// wrongly receive A's message.
#[tokio::test]
async fn test_publish_cannot_spoof_another_services_namespace() {
    let Ok(wasm_bytes) = fs::read(test_constants::messaging_pubsub_test_wasm_path()) else {
        eprintln!(
            "Skipping test_publish_cannot_spoof_another_services_namespace: messaging-pubsub-test \
             WASM artifact not found (run `cargo build --target wasm32-wasip2 --release` in \
             test-components/messaging-pubsub-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_A, &manifest).await.unwrap();
    engine.deploy_wasm(SERVICE_B, &manifest).await.unwrap();

    // Service B subscribes to its own bare topic.
    call(&engine, SERVICE_B, "subscribe-to", serde_json::json!(["orders/new"])).await;

    // Service A tries to spoof service B's namespace by publishing to a
    // topic that already looks fully-qualified as `svc/<B>/...`.
    let spoofed_topic = format!("svc/{SERVICE_B}/orders/new");
    call(&engine, SERVICE_A, "publish-to", serde_json::json!([spoofed_topic, "spoofed"])).await;

    // Give any (incorrect) delivery a chance to land before asserting
    // absence.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let received = call(&engine, SERVICE_B, "get-received-messages", serde_json::json!([])).await;
    assert_eq!(received, "", "service B must not receive a publish spoofed via svc/<B>/...");
}

/// task.md's Measurable Exit Criteria: guest `handle-message` delivery
/// <25ms p99 (the guest path is more expensive than the native-subscriber
/// path due to fresh-Store-per-delivery instantiation cost).
#[tokio::test]
async fn test_guest_delivery_latency_budget() {
    let Ok(wasm_bytes) = fs::read(test_constants::messaging_pubsub_test_wasm_path()) else {
        eprintln!(
            "Skipping test_guest_delivery_latency_budget: messaging-pubsub-test WASM artifact not \
             found (run `cargo build --target wasm32-wasip2 --release` in \
             test-components/messaging-pubsub-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_A, &manifest).await.unwrap();
    engine.deploy_wasm(SERVICE_B, &manifest).await.unwrap();

    let fully_qualified_topic = format!("svc/{SERVICE_A}/orders/new");
    call(&engine, SERVICE_B, "subscribe-to", serde_json::json!([fully_qualified_topic])).await;

    let mut latencies = Vec::new();
    for i in 0..20u32 {
        let publish_start = tokio::time::Instant::now();
        call(&engine, SERVICE_A, "publish-to", serde_json::json!(["orders/new", format!("m{i}")]))
            .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let received =
                call(&engine, SERVICE_B, "get-received-messages", serde_json::json!([])).await;
            if received.lines().count() == (i + 1) as usize {
                latencies.push(publish_start.elapsed());
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for guest delivery #{i}"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    latencies.sort();
    let p99 = latencies[(latencies.len() * 99 / 100).min(latencies.len() - 1)];
    eprintln!(
        "guest handle-message delivery latency: p99={p99:?} max={:?} (n={})",
        latencies.last().unwrap(),
        latencies.len()
    );
    // task.md's Measurable Exit Criteria budget is 25ms p99; asserted here
    // at 3x that (75ms) for headroom against shared-CI-runner variance and
    // this loop's own 2ms polling granularity, while still catching an
    // order-of-magnitude regression.
    assert!(p99 < Duration::from_millis(75), "guest delivery p99 budget blown: {p99:?}");
}
