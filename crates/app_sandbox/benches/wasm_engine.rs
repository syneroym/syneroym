#![allow(clippy::unwrap_used, clippy::panic)]
use std::{fs, sync::Arc};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_app_sandbox::{AppSandboxEngine, HostState, conversions::json_to_wasm_params};
use syneroym_core::test_constants;
use syneroym_data_layer::SqliteStorageProvider;
use syneroym_key_store::KeyStore;
use test_constants::GREETER_INTERFACE_NAME;
use tokio::runtime::Builder;
use wasmtime::{
    Store,
    component::{Component, Linker, types::ComponentItem},
};

fn bench_wasm_engine(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let component_path = test_constants::greeter_wasm_path();
    let wasm_bytes = match fs::read(&component_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            println!(
                "Warning: syneroym_test_greeter.wasm not found at {}, skipping instantiation \
                 benchmarks",
                component_path.display()
            );
            return;
        }
    };

    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn syneroym_data_layer::StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    let engine = AppSandboxEngine::build_wasm_engine(None, None).unwrap();
    let linker: Linker<HostState> = AppSandboxEngine::build_wasm_linker(&engine).unwrap();
    let component = Component::new(&engine, &wasm_bytes).unwrap();

    // Benchmark 1: Wasm Store & HostState Creation
    c.bench_function("wasm_store_creation", |b| {
        b.iter(|| {
            let host_state = HostState::new(
                black_box("test_component".to_string()),
                None,
                key_store.clone(),
                storage_provider.clone(),
                false,
                0,
            );
            let _store = Store::new(&engine, host_state);
        });
    });

    // Benchmark 2: Wasm Instantiation (cached component)
    c.bench_function("wasm_cached_instantiation", |b| {
        b.to_async(&runtime).iter(|| async {
            let host_state = HostState::new(
                "test_component".to_string(),
                None,
                key_store.clone(),
                storage_provider.clone(),
                false,
                0,
            );
            let mut store: Store<HostState> = Store::new(&engine, host_state);
            store.set_fuel(1_000_000).unwrap();
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1_000);
            let _instance = linker.instantiate_async(&mut store, &component).await.unwrap();
        });
    });

    // Extract type info for JSON parameter conversion benchmark
    let host_state = HostState::new(
        "test_component".to_string(),
        None,
        key_store.clone(),
        storage_provider.clone(),
        false,
        0,
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

    // Benchmark 3: json_to_wasm_params conversion
    let json_params = vec![serde_json::Value::String("BenchmarkUser".to_string())];

    c.bench_function("json_to_wasm_params", |b| {
        b.iter(|| {
            let params_iter = match &item {
                ComponentItem::ComponentFunc(f) => f.params(),
                _ => panic!("Expected a function item"),
            };
            let _ = json_to_wasm_params(params_iter, black_box(json_params.clone())).unwrap();
        });
    });
}

criterion_group!(benches, bench_wasm_engine);
criterion_main!(benches);
