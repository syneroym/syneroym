#![allow(clippy::unwrap_used, clippy::panic)]
//! FDAE pushdown query performance budget (M04B, task.md's Performance
//! Budgets table): "100 records, single-hop ReBAC" `query` (Mode B) end to
//! end through the real `ServiceStore` -- policy compilation + the merged
//! `WHERE EXISTS` SQL, executed against real SQLite -- must stay under
//! 25 ms p99 (M3A's unauthenticated 20 ms baseline + 5 ms for policy
//! compilation). `criterion`'s own report (mean/p99 in
//! `target/criterion/fdae_pushdown_query/single_hop_100_records/report/
//! index.html`, or `cargo xtask perf-summary`'s `PERF_SUMMARY.md` append) is
//! the source of truth for the budget, per this workspace's convention
//! (`security_config_bench.rs`'s SQLCipher A/B comparison uses the same
//! approach) -- not a hand-rolled in-process assertion.

use std::sync::Arc;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use serde_json::json;
use syneroym_data_db::{
    QueryAuth, ServiceStore, SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema, Mutation, PatchMutation, QueryOptions, RecordWriteValue},
};
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Policy, parse_and_validate};
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};
use tokio::runtime::Builder;

const SERVICE_ID: &str = "fdae-bench-svc";
const RECORD_COUNT: usize = 100;

fn schema(name: &str) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes: vec![] }
}

fn write_value(id: &str, json: &str) -> RecordWriteValue {
    RecordWriteValue { id: id.to_string(), payload: json.as_bytes().to_vec() }
}

/// `document` --creator--> `user` (principal_column `did`); `view` reachable
/// only via the creator relation -- the plan's "single-hop ReBAC" shape.
fn single_hop_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

fn resource(collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(SERVICE_ID, SERVICE_ID).0
    ))
}

fn session(subject_did: &str) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: None,
        // Scoped to the physical collection name ("documents", the string
        // `store.query` is actually called with below) -- `compile_read`
        // builds the resource it checks capabilities against from the
        // query's own `collection` argument, not the policy's definition
        // key ("document").
        capabilities: vec![Capability {
            with: resource("documents"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    }
}

/// Seeds `RECORD_COUNT` documents, half created by `did:key:alice` (visible
/// to the bench caller) and half by a stranger (excluded by the sieve) -- so
/// the benched query does real row-pruning work, not a no-op filter.
async fn seed(store: &dyn ServiceStore) {
    store.create_collection(&schema("users")).await.unwrap();
    store.create_collection(&schema("documents")).await.unwrap();
    store
        .put(
            "users",
            &write_value("u-alice", &json!({"did": "did:key:alice"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "users",
            &write_value("u-mallory", &json!({"did": "did:key:mallory"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
    for i in 0..RECORD_COUNT {
        let creator = if i % 2 == 0 { "u-alice" } else { "u-mallory" };
        store
            .put(
                "documents",
                &write_value(
                    &format!("doc-{i}"),
                    &json!({"creator_uuid": creator, "n": i}).to_string(),
                ),
                "svc",
                None,
            )
            .await
            .unwrap();
    }
}

fn bench_fdae_pushdown_query(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([9u8; 32]).unwrap();

    let store = runtime.block_on(provider.open_service_db(SERVICE_ID, &key_store)).unwrap();
    runtime.block_on(seed(store.as_ref()));

    let policy = single_hop_policy();
    let alice = session("did:key:alice");
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };
    let opts = QueryOptions { filter: None, limit: None, cursor: None };

    let mut group = c.benchmark_group("fdae_pushdown_query");
    group.bench_function("single_hop_100_records", |b| {
        b.to_async(&runtime).iter(|| async {
            let outcome =
                store.query("documents", black_box(&opts), Some(black_box(&auth))).await.unwrap();
            assert_eq!(outcome.value.records.len(), RECORD_COUNT / 2);
        });
    });
    group.finish();

    runtime.block_on(async {
        drop(store);
        drop(provider);
    });
}

/// A `manage` permission covering `data-layer/write` via the same creator
/// relation -- M04B Slice B5-fdae's write-side sieve.
fn write_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {"allows": ["data-layer/write"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

fn write_session(subject_did: &str) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: None,
        capabilities: vec![Capability {
            with: resource("documents"),
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        }],
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    }
}

/// M04B Slice B5-fdae: the per-mutation `EXISTS` check (2 per row for
/// `patch`/`put`-update, 1 for `delete`) an authorized write pays that an
/// unauthorized (`auth: None`) one does not -- claim to establish per
/// task.md's Performance Budgets table: it must not dominate write latency.
/// `patch` alone and a 50-mutation `batch_mutate` (all of alice's own,
/// even-numbered seeded rows), each against the unauthorized baseline.
fn bench_fdae_authorized_writes(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let provider = SqliteStorageProvider::new(temp_dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([9u8; 32]).unwrap();

    let store = runtime.block_on(provider.open_service_db(SERVICE_ID, &key_store)).unwrap();
    runtime.block_on(seed(store.as_ref()));

    let policy = write_policy();
    let alice = write_session("did:key:alice");
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    // Every even-numbered seeded doc belongs to alice (`seed`'s own
    // `i % 2 == 0` rule) -- 50 of the 100 records.
    let batch: Vec<Mutation> = (0..RECORD_COUNT / 2)
        .map(|i| {
            Mutation::Patch(PatchMutation {
                id: format!("doc-{}", i * 2),
                patch_json: br#"{"touched":true}"#.to_vec(),
            })
        })
        .collect();

    let mut group = c.benchmark_group("fdae_authorized_write");
    group.bench_function("patch_unauthorized_baseline", |b| {
        b.to_async(&runtime).iter(|| async {
            store
                .patch("documents", black_box("doc-0"), black_box(br#"{"n":1}"#), None)
                .await
                .unwrap();
        });
    });
    group.bench_function("patch_authorized", |b| {
        b.to_async(&runtime).iter(|| async {
            store
                .patch(
                    "documents",
                    black_box("doc-0"),
                    black_box(br#"{"n":1}"#),
                    Some(black_box(&auth)),
                )
                .await
                .unwrap();
        });
    });
    group.bench_function("batch_mutate_50_unauthorized_baseline", |b| {
        b.to_async(&runtime).iter(|| async {
            store.batch_mutate("documents", black_box(&batch), "svc", None).await.unwrap();
        });
    });
    group.bench_function("batch_mutate_50_authorized", |b| {
        b.to_async(&runtime).iter(|| async {
            store
                .batch_mutate(
                    "documents",
                    black_box(&batch),
                    "did:key:alice",
                    Some(black_box(&auth)),
                )
                .await
                .unwrap();
        });
    });
    group.finish();

    runtime.block_on(async {
        drop(store);
        drop(provider);
    });
}

criterion_group!(benches, bench_fdae_pushdown_query, bench_fdae_authorized_writes);
criterion_main!(benches);
