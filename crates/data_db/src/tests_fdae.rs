//! FDAE pushdown-sieve integration tests (M04B Slice B2 Phase 2): real SQL
//! against seeded rows through the `ServiceStore` trait, exercised with a
//! real compiled [`Policy`] and hand-built `SessionContext`s -- asserting row
//! *visibility*, not SQL string shape.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{CompiledSieve, DecisionTrace, Policy, parse_and_validate};
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};
use tempfile::tempdir;

use crate::{
    QueryAuth, ServiceStore, SqliteStorageProvider, StorageProvider,
    host_store::{
        CollectionSchema, DataLayerError, Mutation, PatchMutation, QueryOptions, RecordWriteValue,
        SqlValue,
    },
};

/// `SqlValue` doesn't derive `PartialEq` (only `Clone`/`Debug`/serde) --
/// compare via its already-derived `Serialize` impl, mirroring
/// `tests_crud.rs::rows_as_json`.
fn rows_as_json(rows: &[Vec<SqlValue>]) -> serde_json::Value {
    serde_json::to_value(rows).unwrap()
}

const SERVICE_ID: &str = "svc-fdae-test";

async fn setup_store() -> Box<dyn ServiceStore> {
    let dir = tempdir().unwrap();
    let dir = Box::leak(Box::new(dir));
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());
    provider.open_service_db("fdae-test-svc", &key_store).await.unwrap()
}

fn plain_schema(name: &str) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes: vec![] }
}

fn write_value(id: &str, payload_json: &str) -> RecordWriteValue {
    RecordWriteValue { id: id.to_string(), payload: payload_json.as_bytes().to_vec() }
}

fn resource(collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(SERVICE_ID, SERVICE_ID).0
    ))
}

fn read_cap(collection: &str) -> Capability {
    Capability {
        with: resource(collection),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: None,
    }
}

fn session(subject_did: &str, capabilities: Vec<Capability>) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: None,
        capabilities,
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    }
}

fn session_with_anchor(
    subject_did: &str,
    anchor_did: &str,
    capabilities: Vec<Capability>,
) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: Some(anchor_did.to_string()),
        capabilities,
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    }
}

/// `document` --creator--> `user` (principal_column `did`), `view` permission
/// reachable only via the creator relation.
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

/// Same shape as `single_hop_policy`, but the `view` path terminates in
/// `anchor` rather than `caller`.
fn single_hop_anchor_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "anchor"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// Same shape as `single_hop_policy`, plus a CLS `fields.deny: ["ssn"]` on
/// `view`.
fn cls_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// A `manage` permission covering `data-layer/write`, reachable via the same
/// creator relation -- used to exercise `delete_many`'s D2 write-op binding.
/// A single permission covering both read and write, opted into the
/// stage-4 after-step (ADR-0017 §7, `authorize_rows: true`) -- `data_db`
/// has no WASM engine, so this is exactly the shape
/// `do_aggregate`/`do_delete_many` must deny closed on (the after-step
/// needs materialized candidate rows an aggregate never surfaces, and
/// deletion has no rows left to hand it once the `DELETE` has run). Covers
/// both operations with one policy since both tests only need "some
/// applicable permission opted in", not distinct read/write shapes.
fn abac_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {
                            "allows": ["data-layer/read", "data-layer/write"],
                            "paths": [["creator", "caller"]],
                            "authorize_rows": true
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

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

/// A `paths: []` (public/shared-team) write permission -- every row is
/// reachable, regardless of who created it. Used for D-B5-6: proves an
/// upsert by a teammate who legitimately reaches the row cannot steal its
/// `creator_id` (`ON CONFLICT` no longer refreshes that column).
fn shared_write_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "permissions": {
                        "manage": {"allows": ["data-layer/write"], "paths": []}
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// Same shape as `write_policy`, plus a CLS `fields.deny: ["ssn"]` on
/// `manage` -- D-B5-7's write-side CLS enforcement.
fn cls_write_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {
                            "allows": ["data-layer/write"],
                            "paths": [["creator", "caller"]],
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

fn write_cap(collection: &str) -> Capability {
    Capability {
        with: resource(collection),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    }
}

/// `document.creator` targets a definition (`ghost_user`) whose physical
/// table is never created via `create_collection` -- ADR-0017's 2026-07-20
/// `principal_column` amendment's residual "missing target table" case
/// (§6.6): this must fail closed, not leak.
fn missing_target_table_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "ghost_user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                    }
                },
                "ghost_user": {"table": "ghost_users_never_created", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

async fn seed_creator_docs(store: &dyn ServiceStore) {
    store.create_collection(&plain_schema("users")).await.unwrap();
    store.create_collection(&plain_schema("documents")).await.unwrap();
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
            &write_value("u-bob", &json!({"did": "did:key:bob"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "documents",
            &write_value("doc-1", &json!({"creator_uuid": "u-alice"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "documents",
            &write_value("doc-2", &json!({"creator_uuid": "u-bob"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn mode_b_query_excludes_unreachable_rows_not_error() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    let ids: Vec<_> = outcome.value.records.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec!["doc-1"], "bob's document must be excluded, not erred");
    assert!(outcome.masked_fields.is_empty());
}

/// Slice B3 Phase 4: `QueryAuth.resolved_sieve`, when present, is used
/// as-is and `compile_read` is never consulted -- a caller that already ran
/// `plan_read` + the `resolve_fetches` orchestration + `finalize` (because
/// the policy needed a remote relationship fetch) must not have its result
/// silently re-derived (and potentially narrowed or widened) by the store
/// recompiling from `policy`/`session` on its own. Proven here with a
/// stranger session `single_hop_policy` would otherwise deny outright: with
/// `resolved_sieve` set to an always-true predicate, her query still
/// returns every row.
#[tokio::test]
async fn resolved_sieve_preempts_compile_read_and_is_used_verbatim() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let mallory = session("did:key:mallory", vec![read_cap("documents")]);

    // Baseline: without a pre-resolved sieve, `compile_read` denies mallory
    // outright (she owns neither document).
    let auth_local = QueryAuth {
        policy: &policy,
        session: &mallory,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };
    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let baseline = store.query("documents", &opts, Some(&auth_local)).await.unwrap();
    assert!(baseline.value.records.is_empty(), "compile_read must deny a stranger, as before");

    // With a pre-resolved (always-true) sieve, the same denied session
    // reaches every row -- proving `resolved_sieve` is used verbatim, not
    // merely accepted and then ignored in favor of a fresh `compile_read`.
    let resolved_sieve = CompiledSieve {
        where_clause: "1=1".to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
        trace: DecisionTrace::default(),
        abac_permissions: Vec::new(),
    };
    let auth_resolved = QueryAuth {
        policy: &policy,
        session: &mallory,
        service_id: SERVICE_ID,
        resolved_sieve: Some(resolved_sieve),
    };
    let outcome = store.query("documents", &opts, Some(&auth_resolved)).await.unwrap();
    let mut ids: Vec<_> = outcome.value.records.iter().map(|r| r.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["doc-1".to_string(), "doc-2".to_string()]);
}

/// The anchor terminal reaching real SQL execution end to end, not just the
/// compiled predicate string: a proxying caller (`svc-1`) whose own DID
/// matches nobody's rows still reaches alice's document when anchored to
/// alice, and reaches nothing when anchored to a stranger.
#[tokio::test]
async fn mode_b_query_filters_by_anchor_not_by_the_proxying_caller() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_anchor_policy();
    let opts = QueryOptions { filter: None, limit: None, cursor: None };

    let proxying_for_alice =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &proxying_for_alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    let ids: Vec<_> = outcome.value.records.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec!["doc-1"], "anchored to alice, must reach alice's document");

    let proxying_for_a_stranger =
        session_with_anchor("did:key:svc-1", "did:key:mallory", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &proxying_for_a_stranger,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    assert!(outcome.value.records.is_empty(), "anchored to a stranger, must reach nothing");
}

#[tokio::test]
async fn mode_a_check_access_denies_unreachable_row() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    assert!(
        store
            .check_access("documents", "doc-1", Ability::DATA_LAYER_READ, Some(&auth))
            .await
            .unwrap()
    );
    assert!(
        !store
            .check_access("documents", "doc-2", Ability::DATA_LAYER_READ, Some(&auth))
            .await
            .unwrap()
    );
}

/// The `resolved_sieve` pre-emption applies to Mode A (`check_access`) too,
/// not just Mode B: a resolved sieve is used verbatim instead of the store
/// re-deriving one via `compile_read`.
#[tokio::test]
async fn resolved_sieve_preempts_compile_read_for_check_access_too() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let mallory = session("did:key:mallory", vec![read_cap("documents")]);
    let resolved_sieve = CompiledSieve {
        where_clause: "1=1".to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
        trace: DecisionTrace::default(),
        abac_permissions: Vec::new(),
    };
    let auth = QueryAuth {
        policy: &policy,
        session: &mallory,
        service_id: SERVICE_ID,
        resolved_sieve: Some(resolved_sieve),
    };
    assert!(
        store
            .check_access("documents", "doc-2", Ability::DATA_LAYER_READ, Some(&auth))
            .await
            .unwrap(),
        "a stranger `compile_read` would deny must be admitted when resolved_sieve is \
         verbatim-true"
    );
}

#[tokio::test]
async fn check_access_with_no_auth_is_an_existence_check() {
    // D3: `auth = None` falls back to plain existence, not policy semantics.
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;

    assert!(store.check_access("documents", "doc-1", "data-layer/read", None).await.unwrap());
    assert!(store.check_access("documents", "doc-2", "data-layer/read", None).await.unwrap());
    assert!(
        !store.check_access("documents", "does-not-exist", "data-layer/read", None).await.unwrap()
    );
}

#[tokio::test]
async fn get_of_unreachable_row_returns_none_not_error() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let own = store.get("documents", "doc-1", Some(&auth)).await.unwrap();
    assert!(own.value.is_some());
    let other = store.get("documents", "doc-2", Some(&auth)).await.unwrap();
    assert!(other.value.is_none(), "an existing-but-unreachable row reads as a miss (ADR-0007)");
}

#[tokio::test]
async fn aggregate_is_row_filtered_identically_to_query() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let result = store
        .aggregate("documents", r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#, Some(&auth))
        .await
        .unwrap();
    assert_eq!(
        rows_as_json(&result.rows),
        rows_as_json(&[vec![SqlValue::Integer(1)]]),
        "only alice's own doc-1 is counted"
    );
}

#[tokio::test]
async fn aggregate_denied_when_cls_active() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = cls_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store
        .aggregate("documents", r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

/// ADR-0017 §7: `data_db` has no WASM engine to run the stage-4 after-step,
/// so a sieve whose applicable permission opted in (`abac_permissions`
/// non-empty) must deny the whole aggregate closed -- same category as the
/// CLS denial above, not something the ingress can work around by
/// narrowing its own request.
#[tokio::test]
async fn aggregate_denies_closed_under_a_stage4_policy() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = abac_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store
        .aggregate("documents", r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

#[tokio::test]
async fn masked_fields_exposed_but_rows_unmasked_in_phase_2() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = cls_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    assert_eq!(outcome.masked_fields, vec!["ssn".to_string()]);
    // Phase 2 never strips fields itself (Phase 3 does, host-side) -- the
    // row's payload is untouched even though the mask metadata is exposed.
    assert_eq!(outcome.value.records.len(), 1);
}

/// A CLS-masked field must not be filterable either -- otherwise the Phase-3
/// host-side strip only hides the value from the *output*, while the
/// caller's own filter predicate (which runs in SQL against the raw
/// payload, unaware of `masked_fields`) still turns row presence/absence
/// into a boolean oracle -- and with `$regex`/comparison operators, a full
/// extraction channel, not just a single guess. Surfaced during Slice B2
/// Phase 3 review.
#[tokio::test]
async fn query_filter_referencing_a_cls_masked_field_is_denied() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = cls_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions {
        filter: Some(r#"{"ssn": {"$regex": "1"}}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let err = store.query("documents", &opts, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    // Nested under $and/$or/$not, or as a dotted sub-path, must be caught
    // too -- not just a bare top-level equality filter.
    let opts = QueryOptions {
        filter: Some(r#"{"$and": [{"kind": "report"}, {"ssn.prefix": "1"}]}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let err = store.query("documents", &opts, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

/// The masked-field filter deny must not over-trigger: filtering on a
/// non-masked field while CLS is active for a *different* field must still
/// work normally.
#[tokio::test]
async fn query_filter_on_non_masked_field_still_works_when_cls_active() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = cls_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions {
        filter: Some(r#"{"creator_uuid": "u-alice"}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    assert_eq!(outcome.value.records.len(), 1);
}

#[tokio::test]
async fn delete_many_is_row_filtered_as_a_write_operation() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    // Alice holds only a *read* capability -- `manage` requires
    // data-layer/write, so D2's write-mode compile must deny every row.
    let alice_read_only = session("did:key:alice", vec![read_cap("documents")]);
    let auth_ro = QueryAuth {
        policy: &policy,
        session: &alice_read_only,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };
    let deleted = store.delete_many("documents", None, Some(&auth_ro)).await.unwrap();
    assert_eq!(deleted, 0, "a read-only capability must not satisfy the write-mode sieve");

    // A write capability lets alice delete only her own row.
    let write_cap = Capability {
        with: resource("documents"),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    };
    let alice_write = session("did:key:alice", vec![write_cap]);
    let auth_rw = QueryAuth {
        policy: &policy,
        session: &alice_write,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };
    let deleted = store.delete_many("documents", None, Some(&auth_rw)).await.unwrap();
    assert_eq!(deleted, 1, "only alice's own document is deletable");
    assert!(store.get("documents", "doc-1", None).await.unwrap().value.is_none());
    assert!(store.get("documents", "doc-2", None).await.unwrap().value.is_some());
}

/// ADR-0017 §7: deletion happens inside SQL, so there is no candidate-row
/// batch left to hand the stage-4 after-step once the `DELETE` has run --
/// a sieve whose applicable permission opted in must deny the whole call
/// closed rather than delete unfiltered or delete first and skip the
/// after-step.
#[tokio::test]
async fn delete_many_denies_closed_under_a_stage4_policy() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = abac_policy();
    let write_cap = Capability {
        with: resource("documents"),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![write_cap]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store.delete_many("documents", None, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    assert!(
        store.get("documents", "doc-1", None).await.unwrap().value.is_some(),
        "a denied-closed delete_many must not remove anything"
    );
}

#[tokio::test]
async fn binding_order_sieve_and_filter_and_cursor_with_caveat_where() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("users")).await.unwrap();
    store.create_collection(&plain_schema("documents")).await.unwrap();
    store
        .put(
            "users",
            &write_value("u-alice", &json!({"did": "did:key:alice"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
    for (id, region, kind) in
        [("doc-1", "EU", "report"), ("doc-2", "US", "report"), ("doc-3", "EU", "memo")]
    {
        store
            .put(
                "documents",
                &write_value(
                    id,
                    &json!({"creator_uuid": "u-alice", "region": region, "kind": kind}).to_string(),
                ),
                "svc",
                None,
            )
            .await
            .unwrap();
    }

    let policy = single_hop_policy();
    let cap_with_region_caveat = Capability {
        with: resource("documents"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"where": {"region": "EU"}})),
    };
    let alice = session("did:key:alice", vec![cap_with_region_caveat]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    // Sieve (creator=alice, all 3) ∧ caveat (region=EU, doc-1/doc-3) ∧ the
    // caller's own JSON filter (kind=report, doc-1 only) ∧ cursor pagination.
    let opts = QueryOptions {
        filter: Some(r#"{"kind": "report"}"#.to_string()),
        limit: Some(10),
        cursor: None,
    };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    let ids: Vec<_> = outcome.value.records.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec!["doc-1"]);
}

#[tokio::test]
async fn missing_target_table_fails_closed_not_leak() {
    let store = setup_store().await;
    // Only `documents` is created -- `ghost_users_never_created` never is.
    store.create_collection(&plain_schema("documents")).await.unwrap();
    store
        .put(
            "documents",
            &write_value("doc-1", &json!({"creator_uuid": "u-alice"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();

    let policy = missing_target_table_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let err = store.query("documents", &opts, Some(&auth)).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::CollectionNotFound | DataLayerError::Internal(_)),
        "a missing policy-referenced table must surface as an error, not an empty-but-successful \
         (silently-wrong) result: got {err:?}"
    );

    // Mode A: same missing-table condition must fail closed to `Ok(false)`,
    // never `Ok(true)`.
    assert!(
        !store
            .check_access("documents", "doc-1", Ability::DATA_LAYER_READ, Some(&auth))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn policy_absent_definition_is_unfiltered_when_not_strict() {
    // No `auth` at all preserves today's unfiltered behavior -- covered by
    // `tests_crud.rs`; here we cover the "auth present, but the policy names
    // no definition for this collection" branch instead (`compile_read`'s
    // `Ok(None)` path), which must also be unfiltered, not denied.
    let store = setup_store().await;
    store.create_collection(&plain_schema("unrelated")).await.unwrap();
    store.put("unrelated", &write_value("r1", "{}"), "svc", None).await.unwrap();

    let policy = parse_and_validate(r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("unrelated", &opts, Some(&auth)).await.unwrap();
    assert_eq!(outcome.value.records.len(), 1);
}

/// Regression: a caller respelling the collection's case must not fall
/// through to the "no definition for this collection" unfiltered path.
/// SQLite resolves table names case-insensitively, so `FROM DOCUMENTS` and
/// `FROM documents` hit the exact same physical table -- prior to the fix,
/// `find_definition`'s case-sensitive lookup would miss `single_hop_policy`'s
/// "document" definition for a differently-cased `collection` argument and
/// return `Ok(None)` ("policy is silent, unfiltered"), skipping RLS
/// entirely and skipping the capability check that precedes it too (`Ok(None)`
/// is returned before any capability is even consulted). Mallory holds *no*
/// capabilities at all -- the strongest demonstration that the bypass didn't
/// depend on what she was granted, only on how she spelled the collection.
#[tokio::test]
async fn differently_cased_collection_name_does_not_bypass_the_sieve() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let mallory = session("did:key:mallory", vec![]);
    let auth = QueryAuth {
        policy: &policy,
        session: &mallory,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("DOCUMENTS", &opts, Some(&auth)).await.unwrap();
    assert!(
        outcome.value.records.is_empty(),
        "a differently-cased collection name must still resolve to the 'document' definition and \
         deny an uncapable caller, not silently return every row"
    );
}

/// Plan §11's "adversarial `subject_did`/caveat bound not interpolated
/// (covered in `fdae`; add a data_db end-to-end row)" -- `fdae`'s own unit
/// tests already prove `compile_read` binds these as `?` params; this proves
/// the same holds once `data_db` runs the merged sieve+caveat SQL for real,
/// through both Mode B (`query`) and Mode A (`check_access`).
#[tokio::test]
async fn adversarial_subject_did_and_caveat_value_are_bound_not_interpolated() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();

    // If either the subject_did or the caveat's `where` value were ever
    // string-interpolated instead of bound, `OR '1'='1'` would make every
    // row visible and the embedded `DROP TABLE`/comment would either break
    // the query or actually execute.
    let attacker_cap = Capability {
        with: resource("documents"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"where": {"kind": "x'; DROP TABLE documents; --"}})),
    };
    let attacker = session("attacker' OR '1'='1", vec![attacker_cap]);
    let auth = QueryAuth {
        policy: &policy,
        session: &attacker,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    assert!(
        outcome.value.records.is_empty(),
        "an adversarial subject_did/caveat must never widen visibility via injection"
    );

    // Mode A, same adversarial session: a real, bound `id = ?` AND
    // subject_did predicate must still correctly deny, not error or panic.
    assert!(
        !store
            .check_access("documents", "doc-1", Ability::DATA_LAYER_READ, Some(&auth))
            .await
            .unwrap()
    );

    // The table must still exist and be fully queryable afterwards -- a
    // real injection would have corrupted or dropped it.
    assert!(
        store.check_access("documents", "doc-1", Ability::DATA_LAYER_READ, None).await.unwrap()
    );
}

/// **Known limitation, tracked as D-04-02-g** (surfaced during Slice B2
/// Phase 2 review): `CompiledSieve.where_caveats` is a flat list collected
/// from *every* entitling capability (`crates/fdae/src/compile.rs`'s
/// `entitling_caps`), not associated per-OR-branch. `merge_sieve` ANDs all
/// of them onto the single RLS predicate, so a caller holding a second,
/// narrower-caveated capability on the same resource has their *broader*
/// capability's access narrowed too -- capabilities are meant to be
/// additive, not intersective. This is a `crates/fdae` (Phase 1, already
/// shipped) data-shape issue, not something Phase 2's `merge_sieve` can fix
/// on its own: resolving it needs `CompiledSieve` to carry each caveat
/// alongside the OR-branch it entitles, an ADR-0017-level change. Fails
/// toward *over-restriction*, never a leak -- not a Phase 2 blocker, but
/// pinned here so a future fix has a concrete regression to update (see
/// task.md's Decision Register, D-04-02-g).
#[tokio::test]
async fn two_capabilities_with_conflicting_caveats_currently_narrow_to_zero_rows() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();

    let unrestricted_cap = read_cap("documents");
    let eu_only_cap = Capability {
        with: resource("documents"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"where": {"region": "EU"}})),
    };
    // Alice holds both an unrestricted read grant AND an EU-caveated one on
    // the same resource -- today's (undesired) behavior ANDs both caveats
    // onto the sieve, so even the unrestricted grant's rows are suppressed.
    let alice = session("did:key:alice", vec![unrestricted_cap, eu_only_cap]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    assert!(
        outcome.value.records.is_empty(),
        "D-04-02-g: today, an extra caveated capability narrows an unrestricted one instead of \
         being additive -- alice's unrestricted grant should see doc-1, but the EU caveat (no \
         seeded document carries a matching 'region') ANDs it away. If this assertion starts \
         failing, D-04-02-g has been fixed -- update this test to assert doc-1 IS visible."
    );
}

/// Regression: under `strict: true`, `compile_read`'s deny path used to
/// interpolate the caller-supplied `collection` string verbatim into
/// `path_failed`, which `tracing::info!` then logged -- before
/// `validate_identifier` ever ran (it only ran later, inside `do_query` on
/// the reader-pool thread). A WASM guest passes `collection` straight
/// through from its own `query`/`get` call, so an unvalidated string could
/// carry a newline or ANSI escape into the substrate's operator log,
/// forging log lines. `compile_sieve_for_op` now validates before
/// `compile_read` ever sees the string, so the malformed name never reaches
/// a trace at all -- it fails the call outright instead.
#[tokio::test]
async fn strict_mode_never_logs_an_unvalidated_collection_name() {
    use std::{io, sync::Mutex};

    use tracing_subscriber::prelude::*;

    let store = setup_store().await;
    let policy =
        parse_and_validate(r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let malicious = "evil\nFORGED LOG LINE injected=true";
    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    // `#[tokio::test]` defaults to the current-thread flavor, so a
    // thread-local subscriber guard held across the `.await` below stays
    // valid for the whole call -- no task migration to another OS thread
    // can happen underneath it.
    let guard = tracing::subscriber::set_default(subscriber);
    let result = store.query(malicious, &opts, Some(&auth)).await;
    drop(guard);
    assert!(
        matches!(result, Err(DataLayerError::SchemaViolation(_))),
        "an invalid identifier must be rejected before compiling, not passed through: {result:?}"
    );

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        !logs_content.contains("injected=true") && !logs_content.contains("FORGED LOG LINE"),
        "the unvalidated collection name must never reach a trace log: logs were: {logs_content}"
    );
}

// -- M04B Slice B5-fdae: write-side Tier 3 (Mode-A write authorization) -----

#[tokio::test]
async fn mode_a_write_denies_patch_of_an_unreachable_row() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let bob = session("did:key:bob", vec![write_cap("documents")]);
    let auth =
        QueryAuth { policy: &policy, session: &bob, service_id: SERVICE_ID, resolved_sieve: None };

    let err = store
        .patch("documents", "doc-1", br#"{"nickname": "stolen"}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    let record = store.get("documents", "doc-1", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert!(payload.get("nickname").is_none(), "a denied patch must not be applied");
}

#[tokio::test]
async fn mode_a_write_denies_delete_of_an_unreachable_row() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let bob = session("did:key:bob", vec![write_cap("documents")]);
    let auth =
        QueryAuth { policy: &policy, session: &bob, service_id: SERVICE_ID, resolved_sieve: None };

    let err = store.delete("documents", "doc-1", Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    assert!(store.get("documents", "doc-1", None).await.unwrap().value.is_some());
}

#[tokio::test]
async fn mode_a_write_denies_put_update_of_an_unreachable_row() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let bob = session("did:key:bob", vec![write_cap("documents")]);
    let auth =
        QueryAuth { policy: &policy, session: &bob, service_id: SERVICE_ID, resolved_sieve: None };

    let err = store
        .put(
            "documents",
            &write_value(
                "doc-1",
                &json!({"creator_uuid": "u-alice", "hijacked": true}).to_string(),
            ),
            "did:key:bob",
            Some(&auth),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    let record = store.get("documents", "doc-1", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert!(payload.get("hijacked").is_none(), "a denied update must not be applied");
}

#[tokio::test]
async fn mode_a_write_allows_patch_of_a_reachable_row() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    store.patch("documents", "doc-1", br#"{"nickname": "al"}"#, Some(&auth)).await.unwrap();
    let record = store.get("documents", "doc-1", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(payload["nickname"], "al");
    assert_eq!(payload["creator_uuid"], "u-alice", "sibling fields survive the merge patch");
}

#[tokio::test]
async fn put_create_is_allowed_when_the_new_row_is_reachable() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    store
        .put(
            "documents",
            &write_value("doc-new", &json!({"creator_uuid": "u-alice"}).to_string()),
            "did:key:alice",
            Some(&auth),
        )
        .await
        .unwrap();
    assert!(store.get("documents", "doc-new", None).await.unwrap().value.is_some());
}

#[tokio::test]
async fn put_create_is_denied_when_the_new_row_would_be_unreachable() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store
        .put(
            "documents",
            &write_value("doc-new", &json!({"creator_uuid": "u-bob"}).to_string()),
            "did:key:alice",
            Some(&auth),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    assert!(
        store.get("documents", "doc-new", None).await.unwrap().value.is_none(),
        "a denied create must roll back, not leave the row inserted"
    );
}

#[tokio::test]
async fn patch_is_denied_when_the_post_image_escapes_the_callers_reach() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store
        .patch("documents", "doc-1", br#"{"creator_uuid": "u-bob"}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    let record = store.get("documents", "doc-1", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(
        payload["creator_uuid"], "u-alice",
        "a post-image-denying patch must roll back, not leave the row half-written"
    );
}

#[tokio::test]
async fn batch_mutate_rolls_back_entirely_when_one_mutation_is_unauthorized() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let mutations = vec![
        Mutation::Patch(PatchMutation {
            id: "doc-1".to_string(),
            patch_json: br#"{"nickname":"al"}"#.to_vec(),
        }),
        // Denied: doc-2 belongs to bob, unreachable to alice.
        Mutation::Delete("doc-2".to_string()),
        Mutation::Put(write_value("doc-new", &json!({"creator_uuid": "u-alice"}).to_string())),
    ];
    let err = store
        .batch_mutate("documents", &mutations, "did:key:alice", Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    let doc1 = store.get("documents", "doc-1", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&doc1.payload).unwrap();
    assert!(payload.get("nickname").is_none(), "the earlier patch must not have persisted");
    assert!(
        store.get("documents", "doc-2", None).await.unwrap().value.is_some(),
        "bob's row survives"
    );
    assert!(
        store.get("documents", "doc-new", None).await.unwrap().value.is_none(),
        "the later put must not have persisted"
    );
}

#[tokio::test]
async fn a_read_only_permission_does_not_authorize_a_write() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err =
        store.patch("documents", "doc-1", br#"{"nickname": "al"}"#, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

#[tokio::test]
async fn writes_are_unfiltered_when_no_definition_matches_the_collection() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("unrelated")).await.unwrap();
    let policy = parse_and_validate(r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    store.put("unrelated", &write_value("r1", "{}"), "did:key:alice", Some(&auth)).await.unwrap();
    assert!(store.get("unrelated", "r1", None).await.unwrap().value.is_some());
}

#[tokio::test]
async fn a_stage4_opted_permission_denies_single_row_writes_closed() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = abac_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err =
        store.patch("documents", "doc-1", br#"{"nickname": "al"}"#, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

/// D-B5-3: `CallerContext::service_system`'s empty-capability session
/// (whatever ingress ultimately synthesizes it) can never satisfy any
/// permission, so a policy-covered collection becomes unwritable from that
/// caller. Deliberate -- see the follow-up recorded in the deferred backlog
/// (threading a real principal into the anonymous-connection and
/// proxied-WASM ingresses).
#[tokio::test]
async fn a_system_caller_write_is_denied_under_a_policy() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let system_like = session("system:svc", vec![]);
    let auth = QueryAuth {
        policy: &policy,
        session: &system_like,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store
        .put("documents", &write_value("doc-new", "{}"), "system:svc", Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

#[tokio::test]
async fn delete_of_a_missing_row_denies_under_a_policy_but_stays_idempotent_without_one() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store.delete("documents", "does-not-exist", Some(&auth)).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::PermissionDenied),
        "the pre-image check cannot distinguish absent from present-but-unreachable, and must not \
         try to -- same CLS-masking existence-oracle refusal, one level up"
    );

    store.delete("documents", "does-not-exist", None).await.unwrap();
}

#[tokio::test]
async fn patch_of_a_missing_row_denies_rather_than_reporting_not_found() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err =
        store.patch("documents", "does-not-exist", br#"{"x": 1}"#, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    let err = store.patch("documents", "does-not-exist", br#"{"x": 1}"#, None).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::SchemaViolation(_)),
        "the policy-absent path keeps today's not-found reporting exactly"
    );
}

#[tokio::test]
async fn an_upsert_by_a_teammate_does_not_steal_creator_id() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("documents")).await.unwrap();
    store.put("documents", &write_value("doc-1", "{}"), "did:key:alice", None).await.unwrap();

    let policy = shared_write_policy();
    let bob = session("did:key:bob", vec![write_cap("documents")]);
    let auth =
        QueryAuth { policy: &policy, session: &bob, service_id: SERVICE_ID, resolved_sieve: None };

    // Bob legitimately reaches the row (the policy's `paths: []` is public)
    // and successfully overwrites its payload -- but must not become its
    // `creator_id` (D-B5-6).
    store
        .put(
            "documents",
            &write_value("doc-1", r#"{"edited_by":"bob"}"#),
            "did:key:bob",
            Some(&auth),
        )
        .await
        .unwrap();
    let record = store.get("documents", "doc-1", None).await.unwrap().value.unwrap();
    assert_eq!(record.creator_id, "did:key:alice", "an upsert must not reassign creator_id");
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(payload["edited_by"], "bob", "the payload itself is legitimately updated");
}

#[tokio::test]
async fn a_masked_field_cannot_be_written_on_create() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = cls_write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let err = store
        .put(
            "documents",
            &write_value(
                "doc-new",
                &json!({"creator_uuid": "u-alice", "ssn": "999-99-9999"}).to_string(),
            ),
            "did:key:alice",
            Some(&auth),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    assert!(store.get("documents", "doc-new", None).await.unwrap().value.is_none());
}

#[tokio::test]
async fn a_masked_fields_value_cannot_be_changed_on_update() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("users")).await.unwrap();
    store.create_collection(&plain_schema("documents")).await.unwrap();
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
            "documents",
            &write_value(
                "doc-changed",
                &json!({"creator_uuid": "u-alice", "ssn": "111-11-1111"}).to_string(),
            ),
            "svc",
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "documents",
            &write_value("doc-added", &json!({"creator_uuid": "u-alice"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();

    let policy = cls_write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    // Changing a masked field's existing value is denied, and rolled back.
    let err = store
        .patch("documents", "doc-changed", br#"{"ssn": "222-22-2222"}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    let record = store.get("documents", "doc-changed", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(payload["ssn"], "111-11-1111", "a denied masked-field change must roll back");

    // Adding a masked field that wasn't there before is equally denied.
    let err = store
        .patch("documents", "doc-added", br#"{"ssn": "333-33-3333"}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    let record = store.get("documents", "doc-added", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert!(payload.get("ssn").is_none(), "a denied masked-field addition must roll back");

    // Removing a previously-present masked field is equally denied.
    let err = store
        .patch("documents", "doc-changed", br#"{"ssn": null}"#, Some(&auth))
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    let record = store.get("documents", "doc-changed", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(payload["ssn"], "111-11-1111", "a denied masked-field removal must roll back");
}
