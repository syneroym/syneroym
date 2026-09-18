#![allow(clippy::unwrap_used, clippy::panic)]
use std::{
    fs,
    sync::{Arc, Weak},
};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use serde_json::Value;
use syneroym_core::{local_registry::EndpointRegistry, storage::MockStorage, test_constants};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::CallerContext;
use syneroym_sandbox_wasm::{
    AppSandboxEngine, HostState, MessagingContext, StreamContext,
    conversions::{json_to_wasm_params, val_to_json},
    empty_service_proxy,
};
use test_constants::GREETER_INTERFACE_NAME;
use tokio::runtime::Builder;
use wasmtime::{
    Store,
    component::{Component, Linker, Val, types::ComponentItem},
};

fn test_messaging_context() -> MessagingContext {
    MessagingContext {
        broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        engine: Weak::new(),
    }
}

fn test_streaming_context() -> StreamContext {
    StreamContext {
        registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        engine: Weak::new(),
    }
}

/// Reads the greeter test fixture's compiled WASM bytes, warning and
/// returning `None` if the fixture hasn't been built -- this skips the
/// instantiation benchmarks rather than failing the whole binary.
fn load_greeter_wasm_bytes() -> Option<Vec<u8>> {
    let component_path = test_constants::greeter_wasm_path();
    match fs::read(&component_path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            println!(
                "Warning: syneroym_test_greeter.wasm not found at {}, skipping instantiation \
                 benchmarks",
                component_path.display()
            );
            None
        }
    }
}

/// Builds the `HostState` shared by every benchmark below -- identical
/// arguments except `service_id`, which the store-creation benchmark wraps
/// in `black_box` to keep the compiler from constant-folding it away.
fn bench_host_state(
    service_id: String,
    key_store: &Arc<KeyStore>,
    storage_provider: &Arc<dyn StorageProvider>,
    blob_provider: &Arc<dyn BlobProvider>,
) -> HostState {
    HostState::new(
        service_id,
        None,
        key_store.clone(),
        storage_provider.clone(),
        blob_provider.clone(),
        CallerContext::service_system("test_component"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
}

fn bench_wasm_engine(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let Some(wasm_bytes) = load_greeter_wasm_bytes() else { return };

    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    // The trailing 0s are unused: `None, None` disables pooling entirely.
    let engine = AppSandboxEngine::build_wasm_engine(None, None, 0, 0, 0).unwrap();
    let linker: Linker<HostState> = AppSandboxEngine::build_wasm_linker(&engine).unwrap();
    let component = Component::new(&engine, &wasm_bytes).unwrap();

    // Benchmark 1: Wasm Store & HostState Creation
    c.bench_function("wasm_store_creation", |b| {
        b.iter(|| {
            let host_state = bench_host_state(
                black_box("test_component".to_string()),
                &key_store,
                &storage_provider,
                &blob_provider,
            );
            let _store = Store::new(&engine, host_state);
        });
    });

    // Benchmark 2: Wasm Instantiation (cached component)
    c.bench_function("wasm_cached_instantiation", |b| {
        b.to_async(&runtime).iter(|| async {
            let host_state = bench_host_state(
                "test_component".to_string(),
                &key_store,
                &storage_provider,
                &blob_provider,
            );
            let mut store: Store<HostState> = Store::new(&engine, host_state);
            store.set_fuel(1_000_000).unwrap();
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1_000);
            let _instance = linker.instantiate_async(&mut store, &component).await.unwrap();
        });
    });

    // Extract type info for JSON parameter conversion benchmark
    let host_state = bench_host_state(
        "test_component".to_string(),
        &key_store,
        &storage_provider,
        &blob_provider,
    );
    let mut store: Store<HostState> = Store::new(&engine, host_state);

    store.set_fuel(1_000_000).unwrap();
    store.epoch_deadline_trap();
    store.set_epoch_deadline(1_000);
    let instance = runtime.block_on(linker.instantiate_async(&mut store, &component)).unwrap();

    let interface_name = GREETER_INTERFACE_NAME;
    let method_name = "greet";
    let (_func, _results_len, item) =
        AppSandboxEngine::get_wasm_func(&mut store, &instance, Some(interface_name), method_name)
            .unwrap();

    // Benchmark 3: json_to_wasm_params conversion (named/positional binding)
    let json_params = Value::Array(vec![Value::String("BenchmarkUser".to_string())]);

    c.bench_function("json_to_wasm_params", |b| {
        b.iter(|| {
            let params_iter = match &item {
                ComponentItem::ComponentFunc(f) => f.params(),
                _ => panic!("Expected a function item"),
            };
            let _ = json_to_wasm_params(params_iter, black_box(&json_params)).unwrap();
        });
    });

    // Benchmark 4: WIT -> JSON conversion of a representative record (the
    // typical result hot path -- a `record-read-value`-shaped value with a
    // ~256-byte `list<u8>` payload). Documents the WIT⇄JSON conversion budget.
    let record_val = Val::Record(vec![
        ("id".to_string(), Val::String("user-1234".to_string())),
        ("payload".to_string(), Val::List((0..256u32).map(|i| Val::U8(i as u8)).collect())),
        ("creator-id".to_string(), Val::String("did:key:z6MkExampleCallerIdentity".to_string())),
        ("created-at".to_string(), Val::U64(1_720_000_000_000)),
        ("updated-at".to_string(), Val::U64(1_720_000_000_500)),
    ]);

    c.bench_function("wit_json_roundtrip", |b| {
        b.iter(|| {
            let _ = val_to_json(black_box(&record_val)).unwrap();
        });
    });
}

criterion_group!(benches, bench_wasm_engine);
criterion_main!(benches);
