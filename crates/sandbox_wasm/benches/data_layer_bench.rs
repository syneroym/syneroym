#![allow(clippy::unwrap_used, clippy::panic)]
//! Slice 3A (M03-sss) performance budgets: CRUD/batch operation latency
//! against an encrypted per-service SQLite database, plus WASM lifecycle
//! hook (`init`/`migrate`) timing through a real deployed component.

use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema, Mutation, QueryOptions, RecordWriteValue},
};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use tokio::runtime::Builder;

fn schema(name: &str) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes: vec![] }
}

fn write_value(id: &str, json: &str) -> RecordWriteValue {
    RecordWriteValue { id: id.to_string(), payload: json.as_bytes().to_vec() }
}

fn bench_data_layer_crud(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let temp_dir = tempfile::tempdir().unwrap();
    let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([7u8; 32]).unwrap();

    let store = runtime.block_on(provider.open_service_db("bench-svc", &key_store)).unwrap();
    runtime.block_on(store.create_collection(&schema("bench"))).unwrap();

    // Benchmark: put (single record, encrypted DB)
    c.bench_function("data_layer_put", |b| {
        b.to_async(&runtime).iter(|| async {
            store
                .put("bench", black_box(&write_value("p1", r#"{"age": 30}"#)), "bench-creator")
                .await
                .unwrap();
        });
    });

    // Benchmark: get (single record, warm reader pool)
    runtime
        .block_on(store.put("bench", &write_value("g1", r#"{"age": 30}"#), "bench-creator"))
        .unwrap();
    c.bench_function("data_layer_get", |b| {
        b.to_async(&runtime).iter(|| async {
            let _ = store.get("bench", black_box("g1"), None).await.unwrap();
        });
    });

    // Benchmark: query (100 records, $eq filter)
    for i in 0..100 {
        runtime
            .block_on(store.put(
                "bench",
                &write_value(&format!("q{i}"), r#"{"kind": "target"}"#),
                "c",
            ))
            .unwrap();
    }
    let query_opts = QueryOptions {
        filter: Some(r#"{"kind": "target"}"#.to_string()),
        limit: None,
        cursor: None,
    };
    c.bench_function("data_layer_query_100_eq_filter", |b| {
        b.to_async(&runtime).iter(|| async {
            let _ = store.query("bench", black_box(&query_opts), None).await.unwrap();
        });
    });

    // Benchmark: batch-mutate (50 mutations, single transaction)
    let mutations: Vec<Mutation> =
        (0..50).map(|i| Mutation::Put(write_value(&format!("b{i}"), "{}"))).collect();
    c.bench_function("data_layer_batch_mutate_50", |b| {
        b.to_async(&runtime).iter(|| async {
            store.batch_mutate("bench", black_box(&mutations), "bench-creator").await.unwrap();
        });
    });

    // `provider` caches the `SqliteServiceStore` (and its deadpool reader
    // pool) internally, so it -- not just the local `store` handle -- must be
    // dropped for the pool to actually go away. deadpool's `Pool` spawns a
    // background cleanup task on `Drop` via `tokio::task::spawn_blocking`,
    // which requires an active Tokio runtime context; drop explicitly here,
    // inside `block_on`, rather than implicitly once this function returns
    // to plain sync code (which panics with "no reactor running").
    runtime.block_on(async {
        drop(store);
        drop(provider);
    });
}

/// Builds a fresh, isolated `AppSandboxEngine` (own tempdir, own storage
/// provider) so each benchmark iteration gets a genuinely new service with
/// no prior deployment history.
async fn fresh_engine() -> (tempfile::TempDir, AppSandboxEngine) {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("data"),
        app_data_dir: temp_dir.path().join("user_data"),
        app_cache_dir: temp_dir.path().join("cache"),
        app_log_dir: temp_dir.path().join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let engine = AppSandboxEngine::init(
        &config,
        vec![],
        key_store,
        storage_provider,
        blob_provider,
        Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
    )
    .await
    .unwrap();
    (temp_dir, engine)
}

/// Benchmarks the schema lifecycle hooks (`init`/`migrate`) invoked from
/// `AppSandboxEngine::deploy_wasm`. There is no smaller public entry point
/// that isolates just the hook call from the rest of `deploy_wasm` (fetch,
/// hash-verify, compile, cache), so these measure the full deploy path; the
/// hook invocation itself is a small fraction of that cost.
fn bench_lifecycle_hooks(c: &mut Criterion) {
    let component_path = test_constants::data_layer_test_wasm_path();
    let wasm_bytes = match fs::read(&component_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            println!(
                "Warning: syneroym_test_data_layer.wasm not found at {}, skipping lifecycle hook \
                 benchmarks",
                component_path.display()
            );
            return;
        }
    };

    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let manifest = Arc::new(DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
    });

    // Benchmark: WASM init() hook (first deploy of a fresh service). Uses
    // `iter_custom` (not `iter_batched`) so that per-iteration setup
    // (`fresh_engine`, which transitively opens a deadpool-backed service
    // store) and the timed call both run as plain `.await`s inside the same
    // `block_on`-driven future. deadpool's `Pool` spawns a background
    // cleanup task via `tokio::task::spawn_blocking` on `Drop`, which
    // requires an active Tokio runtime context at drop time -- keeping setup
    // and teardown in the same continuously-polled async block guarantees
    // that (see the explicit end-of-function drop in `bench_data_layer_crud`
    // for the same constraint in a non-WASM context).
    {
        let manifest = manifest.clone();
        c.bench_function("data_layer_wasm_init_hook", |b| {
            b.to_async(&runtime).iter_custom(|iters| {
                let manifest = manifest.clone();
                async move {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        let (temp_dir, engine) = fresh_engine().await;
                        let start = Instant::now();
                        engine.deploy_wasm(black_box("init-bench-svc"), &manifest).await.unwrap();
                        total += start.elapsed();
                        drop(temp_dir);
                    }
                    total
                }
            });
        });
    }

    // Benchmark: WASM migrate() hook (re-deploy of an already-initialized service)
    c.bench_function("data_layer_wasm_migrate_hook", |b| {
        b.to_async(&runtime).iter_custom(|iters| {
            let manifest = manifest.clone();
            async move {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let (temp_dir, engine) = fresh_engine().await;
                    // First deploy runs init(); untimed.
                    engine.deploy_wasm("migrate-bench-svc", &manifest).await.unwrap();
                    let start = Instant::now();
                    // Second deploy of the same service_id runs migrate().
                    engine.deploy_wasm(black_box("migrate-bench-svc"), &manifest).await.unwrap();
                    total += start.elapsed();
                    drop(temp_dir);
                }
                total
            }
        });
    });
}

criterion_group!(benches, bench_data_layer_crud, bench_lifecycle_hooks);
criterion_main!(benches);
