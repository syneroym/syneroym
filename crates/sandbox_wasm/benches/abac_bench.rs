#![allow(clippy::unwrap_used, clippy::panic)]
//! Slice B4-fdae performance budget (task.md Performance Budgets row 3):
//! the stage-4 ABAC after-step over a candidate batch. Measures
//! `RowAuthorizer::authorize_rows` directly (not `apply_stage4`, whose own
//! overhead is a handful of `Vec` operations, dwarfed by a real WASM call)
//! against the `abac-test` fixture's `allow_all` mode -- the cheapest
//! possible guest logic, isolating the after-step's own fixed cost
//! (component instantiation, WIT value marshalling) from anything a real
//! policy's guest code might add on top.
//!
//! D-B4-1: each call instantiates a fresh, throw-away component instance
//! (no re-entering the live instance a host function is already running
//! inside), so `abac_after_step_0_rows` -- a call with an empty batch, which
//! still pays the full instantiate-and-call cost but has nothing to
//! marshal or iterate -- is this bench's measurement of that fixed
//! per-call floor. The 1/10/100/1000-row benchmarks show how much (or how
//! little) scales on top of it, mirroring Slice B3 Phase 5's own finding
//! that per-call setup dominates over row-count-proportional work.
//!
//! Review finding B4-02/B4-15: the row-count sweep above uses a fixed
//! ~28-byte payload per row, which is the *marshalling cost*'s binding
//! variable (`Val::List(payload.iter().map(Val::U8)...)` expands every
//! payload byte into its own ~40-byte `wasmtime::component::Val`), not row
//! count. `abac_after_step_100_rows_at_*kb` holds row count fixed at 100
//! and sweeps payload size instead, so the headline row-count numbers
//! above aren't read as bounding realistic (kilobyte-scale) records too.
//! This bench measures `RowAuthorizer::authorize_rows` directly, bypassing
//! `apply_stage4`'s `MAX_ABAC_PAYLOAD_BYTES` cap entirely -- the
//! `_at_16kb` point (1.6 MB total) is exactly the measurement that showed
//! the original 16 MiB cap (review residual R4) was too generous and
//! motivated tightening it to 1 MiB, so that point now exceeds what a real
//! read is allowed to reach in production, on purpose: it's here to keep
//! demonstrating the per-byte cost, not to represent an in-budget batch.

use std::{fs, sync::Arc};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{AbacAuthContext, CandidateRow, RowAuthorizer, SessionContext};
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use tokio::runtime::Builder;

const SERVICE_ID: &str = "abac-bench-svc";

fn candidate_row(i: usize) -> CandidateRow {
    candidate_row_with_payload_size(i, br#"{"classification": "public"}"#.len())
}

fn candidate_row_with_payload_size(i: usize, payload_bytes: usize) -> CandidateRow {
    CandidateRow {
        id: format!("row-{i}"),
        payload: vec![b'x'; payload_bytes],
        creator_id: "did:key:zBenchCreator".to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// Deploys the `abac-test` fixture on a fresh, isolated engine (own
/// tempdir, own storage provider -- same isolation `data_layer_bench.rs`'s
/// `fresh_engine` uses). `None` when the WASM artifact hasn't been built.
async fn deployed_engine() -> Option<AppSandboxEngine> {
    let wasm_bytes = fs::read(test_constants::abac_test_wasm_path()).ok()?;

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
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();
    // `temp_dir` must outlive the engine's own use of it; leaked
    // deliberately for the benchmark's process lifetime, same tradeoff
    // `data_layer_bench.rs`'s `fresh_engine` documents for its own callers
    // (there, the caller controls the `TempDir`'s drop point instead).
    std::mem::forget(temp_dir);
    Some(engine)
}

fn bench_stage4_after_step(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let Some(engine) = runtime.block_on(deployed_engine()) else {
        println!(
            "Warning: abac-test WASM artifact not found, skipping stage-4 after-step benchmarks \
             (run `cargo component build --release --target wasm32-wasip2` in \
             test-components/abac-test)"
        );
        return;
    };

    let ctx = AbacAuthContext::build(&SessionContext::default(), "profiles", &["view".to_string()]);

    for batch_size in [0usize, 1, 10, 100, 1000] {
        let rows: Vec<CandidateRow> = (0..batch_size).map(candidate_row).collect();
        c.bench_function(&format!("abac_after_step_{batch_size}_rows"), |b| {
            b.to_async(&runtime).iter(|| async {
                engine.authorize_rows(SERVICE_ID, black_box(&ctx), black_box(&rows)).await.unwrap();
            });
        });
    }

    // Payload-size axis (review finding B4-02/B4-15): row count fixed at
    // 100, payload size swept instead, since B4-02 identified payload
    // bytes -- not row count -- as the actual marshalling-cost driver.
    for payload_kb in [1usize, 16] {
        let rows: Vec<CandidateRow> =
            (0..100).map(|i| candidate_row_with_payload_size(i, payload_kb * 1024)).collect();
        c.bench_function(&format!("abac_after_step_100_rows_at_{payload_kb}kb"), |b| {
            b.to_async(&runtime).iter(|| async {
                engine.authorize_rows(SERVICE_ID, black_box(&ctx), black_box(&rows)).await.unwrap();
            });
        });
    }
}

criterion_group!(benches, bench_stage4_after_step);
criterion_main!(benches);
