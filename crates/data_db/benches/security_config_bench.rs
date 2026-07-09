#![allow(clippy::unwrap_used, clippy::panic)]
//! M03-sss performance budgets not covered by
//! `crates/sandbox_wasm/benches/data_layer_bench.rs`: `vault/reveal`,
//! `config/get`, KEK rotation (100 DEKs), and the SQLCipher-vs-plaintext A/B
//! overhead comparison for `put`/`get`.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema, RecordWriteValue},
};
use syneroym_data_keystore::KeyStore;
use tokio::runtime::Builder;

fn schema(name: &str) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes: vec![] }
}

fn write_value(id: &str, json: &str) -> RecordWriteValue {
    RecordWriteValue { id: id.to_string(), payload: json.as_bytes().to_vec() }
}

/// Benchmark: `vault/reveal` (single secret, warm reader pool).
fn bench_vault_reveal(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([7u8; 32], None).unwrap();

    let store = runtime.block_on(provider.open_service_db("vault-bench-svc", &key_store)).unwrap();
    runtime.block_on(store.write_secret("api_key", b"super-secret-value")).unwrap();

    c.bench_function("vault_reveal", |b| {
        b.to_async(&runtime).iter(|| async {
            let _ = store.reveal_secret(black_box("api_key")).await.unwrap();
        });
    });

    runtime.block_on(async {
        drop(store);
        drop(provider);
    });
}

/// Benchmark: `config/get` (cache-warm, pinned generation) —
/// `StorageProvider::get_latest_config_generation`, the call
/// `build_store_and_instantiate` makes on every WASM invocation to resolve
/// the active generation (`crates/sandbox_wasm/src/engine.rs`).
fn bench_config_get(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();

    runtime
        .block_on(provider.save_config_generation("config-bench-svc", r#"{"db_name":"profiles"}"#))
        .unwrap();

    c.bench_function("config_get", |b| {
        b.to_async(&runtime).iter(|| async {
            let _ =
                provider.get_latest_config_generation(black_box("config-bench-svc")).await.unwrap();
        });
    });

    runtime.block_on(async {
        drop(provider);
    });
}

/// Benchmark: KEK rotation re-encrypting 100 service DEKs in a single
/// `substrate.db` transaction.
fn bench_kek_rotation(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    c.bench_function("kek_rotation_100_deks", |b| {
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let temp_dir = tempfile::tempdir().unwrap();
                let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
                let key_store = Arc::new(KeyStore::new());
                key_store.inject_kek([1u8; 32], None).unwrap();

                // Generating a DEK happens as a side effect of the first
                // `open_service_db` call for a given service_id.
                for i in 0..100 {
                    let store = provider
                        .open_service_db(&format!("rotate-svc-{i}"), &key_store)
                        .await
                        .unwrap();
                    drop(store);
                }

                let start = Instant::now();
                provider.rotate_kek(&key_store, black_box([2u8; 32])).await.unwrap();
                total += start.elapsed();

                drop(provider);
            }
            total
        });
    });
}

/// A/B: SQLCipher overhead vs. plaintext for `put`/`get`. Reports both as
/// named `criterion` benchmarks (`sqlcipher_overhead/put_encrypted` vs.
/// `sqlcipher_overhead/put_plaintext`, same for `get`) rather than computing
/// a ratio in-process, so the existing `criterion` HTML report / Statistics
/// comparison is the source of truth for the < 10% budget rather than a
/// hand-rolled percentage calculation here.
fn bench_sqlcipher_overhead(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([7u8; 32], None).unwrap();

    let mut group = c.benchmark_group("sqlcipher_overhead");

    for (label, encrypted) in [("encrypted", true), ("plaintext", false)] {
        let temp_dir = tempfile::tempdir().unwrap();
        let provider = SqliteStorageProvider::new(temp_dir.path(), encrypted).unwrap();
        let store =
            runtime.block_on(provider.open_service_db("overhead-bench-svc", &key_store)).unwrap();
        runtime.block_on(store.create_collection(&schema("bench"))).unwrap();

        group.bench_function(format!("put_{label}"), |b| {
            b.to_async(&runtime).iter(|| async {
                store
                    .put("bench", black_box(&write_value("p1", r#"{"age": 30}"#)), "bench-creator")
                    .await
                    .unwrap();
            });
        });

        runtime
            .block_on(store.put("bench", &write_value("g1", r#"{"age": 30}"#), "bench-creator"))
            .unwrap();
        group.bench_function(format!("get_{label}"), |b| {
            b.to_async(&runtime).iter(|| async {
                let _ = store.get("bench", black_box("g1")).await.unwrap();
            });
        });

        runtime.block_on(async {
            drop(store);
            drop(provider);
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_vault_reveal,
    bench_config_get,
    bench_kek_rotation,
    bench_sqlcipher_overhead
);
criterion_main!(benches);
