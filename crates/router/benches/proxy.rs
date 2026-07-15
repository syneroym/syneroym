#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Universal Proxy same-node latency (M04A Slice A1, F8): the `< 5ms p99`
//! budget is interpreted as in-process local dispatch (`ProxyRouter::invoke`
//! -> registry hit -> native/WASM dispatch -> `Value`), not a loopback QUIC
//! round-trip -- see plan.md F8. Remote-hop latency needs two live nodes and
//! is reported from the cross-node e2e test instead (`coordinator_iroh`'s
//! `test_cross_node_proxy_call`), not benched here.

use std::{fs, sync::Arc};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use dashmap::DashMap;
use serde_json::Value;
use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    dht_registry::RegistryClient,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{IrohHop, ProxyRouter};
use syneroym_rpc::{
    CallOrigin, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, ProxyProtocol, ProxyRequest, RpcResult, ServiceProxy,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::runtime::Builder;

#[derive(Debug, Default)]
struct EchoNativeService;

#[async_trait::async_trait]
impl NativeService for EchoNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: invocation.params })
    }
}

fn base_request(target_service: &str, interface: &str, method: &str) -> ProxyRequest {
    ProxyRequest {
        target_service: target_service.to_string(),
        interface: interface.to_string(),
        method: method.to_string(),
        params: Value::Null,
        caller: CallerContext::service_system("bench-caller"),
        origin: CallOrigin::Native,
        protocol: ProxyProtocol::JsonRpcV1,
        idempotent: false,
        timeout: None,
    }
}

fn bench_proxy_local_native(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    runtime
        .block_on(registry.register(
            "svc-a".to_string(),
            "echo".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
        ))
        .unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    native_dispatch
        .insert("svc-a".to_string(), Arc::new(EchoNativeService) as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        Arc::new(RegistryClient::new(false, None)),
        Arc::downgrade(&native_dispatch),
        std::sync::Weak::new(),
        Arc::new(IrohHop::new(None, RetryPolicy::default())),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );
    let req = base_request("svc-a", "echo", "get");

    c.bench_function("proxy_local_native", |b| {
        b.to_async(&runtime).iter(|| {
            let req = req.clone();
            let router = &router;
            async move {
                black_box(router.invoke(req).await.unwrap());
            }
        });
    });
}

fn bench_proxy_local_wasm(c: &mut Criterion) {
    let component_path = test_constants::greeter_wasm_path();
    let Ok(wasm_bytes) = fs::read(&component_path) else {
        eprintln!(
            "Warning: {} not found, skipping proxy_local_wasm benchmark",
            component_path.display()
        );
        return;
    };

    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let app_sandbox_engine = Arc::new(
        runtime
            .block_on(AppSandboxEngine::init(
                &SubstrateConfig::default(),
                vec![],
                key_store,
                storage_provider,
                blob_provider,
                messaging_broker,
                registry.clone(),
            ))
            .unwrap(),
    );
    app_sandbox_engine.self_weak.set(Arc::downgrade(&app_sandbox_engine)).unwrap();
    app_sandbox_engine.compile_and_cache_wasm("greeter-svc", &wasm_bytes, None).unwrap();

    runtime
        .block_on(registry.register(
            "greeter-svc".to_string(),
            test_constants::GREETER_INTERFACE_NAME.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "greeter-svc".to_string() },
        ))
        .unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        Arc::new(RegistryClient::new(false, None)),
        Arc::downgrade(&native_dispatch),
        Arc::downgrade(&app_sandbox_engine),
        Arc::new(IrohHop::new(None, RetryPolicy::default())),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("greeter-svc", test_constants::GREETER_INTERFACE_NAME, "greet");
    req.params = Value::Array(vec![Value::String("Bencher".to_string())]);

    c.bench_function("proxy_local_wasm", |b| {
        b.to_async(&runtime).iter(|| {
            let req = req.clone();
            let router = &router;
            async move {
                black_box(router.invoke(req).await.unwrap());
            }
        });
    });
}

criterion_group!(benches, bench_proxy_local_native, bench_proxy_local_wasm);
criterion_main!(benches);
