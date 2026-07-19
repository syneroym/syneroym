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
use hkdf::Hkdf;
use sha2::Sha256;
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
    key_store.inject_kek([7u8; 32]).unwrap();

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
                key_store.inject_kek([1u8; 32]).unwrap();

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
    key_store.inject_kek([7u8; 32]).unwrap();

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

/// Isolated cost of the HKDF-SHA256 derivation `resolve_dek` now performs
/// on every call (M04A Slice B6, `derive_instance_kek` in
/// `syneroym_data_keystore::key_store`) -- same construction (no salt, a
/// `"syneroym:kek:v1:{scope}"` info string, 32-byte OKM), duplicated here
/// rather than exposing the private helper, so
/// `bench_service_db_open_with_per_instance_kek` below has a baseline to
/// compare its end-to-end numbers against.
fn bench_hkdf_derive_in_isolation(c: &mut Criterion) {
    let master = [6u8; 32];
    c.bench_function("hkdf_derive_instance_kek", |b| {
        b.iter(|| {
            let hk = Hkdf::<Sha256>::new(None, &master);
            let info = format!("syneroym:kek:v1:{}", black_box("bench-svc"));
            let mut okm = [0u8; 32];
            hk.expand(info.as_bytes(), &mut okm).unwrap();
            black_box(okm);
        });
    });
}

/// Benchmark: `open_service_db` end-to-end with per-instance KEK
/// derivation (M04A Slice B6): HKDF-derive, AES-GCM DEK generate-or-load,
/// and the SQLCipher `PRAGMA key` open. Two shapes, per the task.md
/// perf-budget row ("Service DB open with per-app KEK"): a first open (DEK
/// generated) and a warm re-open (DEK loaded). The warm case uses a fresh
/// `SqliteStorageProvider` instance over the same `db_dir` so the
/// in-memory `service_stores` cache is empty and `open_service_db` is
/// forced through the real `resolve_dek` load path and a fresh SQLCipher
/// open, not the cache shortcut.
fn bench_service_db_open_with_per_instance_kek(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("service_db_open_per_instance_kek");

    group.bench_function("first_open_generate", |b| {
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total = Duration::ZERO;
            for i in 0..iters {
                let temp_dir = tempfile::tempdir().unwrap();
                let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
                let key_store = Arc::new(KeyStore::new());
                key_store.inject_kek([3u8; 32]).unwrap();
                let service_id = format!("first-open-svc-{i}");

                let start = Instant::now();
                let store =
                    provider.open_service_db(black_box(&service_id), &key_store).await.unwrap();
                total += start.elapsed();

                drop(store);
                drop(provider);
            }
            total
        });
    });

    group.bench_function("warm_reopen_load", |b| {
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let temp_dir = tempfile::tempdir().unwrap();
                let key_store = Arc::new(KeyStore::new());
                key_store.inject_kek([4u8; 32]).unwrap();

                // Prime: generates and persists the DEK via a first
                // provider instance (its service_stores cache is dropped
                // with it, so it cannot shortcut the timed reopen below).
                {
                    let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
                    let store = provider.open_service_db("warm-svc", &key_store).await.unwrap();
                    drop(store);
                }

                let provider2 = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
                let start = Instant::now();
                let store =
                    provider2.open_service_db(black_box("warm-svc"), &key_store).await.unwrap();
                total += start.elapsed();

                drop(store);
                drop(provider2);
            }
            total
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_vault_reveal,
    bench_config_get,
    bench_kek_rotation,
    bench_sqlcipher_overhead,
    bench_hkdf_derive_in_isolation,
    bench_service_db_open_with_per_instance_kek
);
criterion_main!(benches);
