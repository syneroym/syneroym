//! FDAE pushdown-sieve integration tests (M04B Slice B2 Phase 2): real SQL
//! against seeded rows through the `ServiceStore` trait, exercised with a
//! real compiled [`Policy`] and hand-built `SessionContext`s -- asserting row
//! *visibility*, not SQL string shape.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Policy, parse_and_validate};
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};
use tempfile::tempdir;

use crate::{
    QueryAuth, ServiceStore, SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema, DataLayerError, QueryOptions, RecordWriteValue, SqlValue},
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
        .put("users", &write_value("u-alice", &json!({"did": "did:key:alice"}).to_string()), "svc")
        .await
        .unwrap();
    store
        .put("users", &write_value("u-bob", &json!({"did": "did:key:bob"}).to_string()), "svc")
        .await
        .unwrap();
    store
        .put(
            "documents",
            &write_value("doc-1", &json!({"creator_uuid": "u-alice"}).to_string()),
            "svc",
        )
        .await
        .unwrap();
    store
        .put(
            "documents",
            &write_value("doc-2", &json!({"creator_uuid": "u-bob"}).to_string()),
            "svc",
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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let outcome = store.query("documents", &opts, Some(&auth)).await.unwrap();
    let ids: Vec<_> = outcome.value.records.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec!["doc-1"], "bob's document must be excluded, not erred");
    assert!(outcome.masked_fields.is_empty());
}

#[tokio::test]
async fn mode_a_check_access_denies_unreachable_row() {
    let store = setup_store().await;
    seed_creator_docs(store.as_ref()).await;
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth_ro = QueryAuth { policy: &policy, session: &alice_read_only, service_id: SERVICE_ID };
    let deleted = store.delete_many("documents", None, Some(&auth_ro)).await.unwrap();
    assert_eq!(deleted, 0, "a read-only capability must not satisfy the write-mode sieve");

    // A write capability lets alice delete only her own row.
    let write_cap = Capability {
        with: resource("documents"),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    };
    let alice_write = session("did:key:alice", vec![write_cap]);
    let auth_rw = QueryAuth { policy: &policy, session: &alice_write, service_id: SERVICE_ID };
    let deleted = store.delete_many("documents", None, Some(&auth_rw)).await.unwrap();
    assert_eq!(deleted, 1, "only alice's own document is deletable");
    assert!(store.get("documents", "doc-1", None).await.unwrap().value.is_none());
    assert!(store.get("documents", "doc-2", None).await.unwrap().value.is_some());
}

#[tokio::test]
async fn binding_order_sieve_and_filter_and_cursor_with_caveat_where() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("users")).await.unwrap();
    store.create_collection(&plain_schema("documents")).await.unwrap();
    store
        .put("users", &write_value("u-alice", &json!({"did": "did:key:alice"}).to_string()), "svc")
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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
        )
        .await
        .unwrap();

    let policy = missing_target_table_policy();
    let alice = session("did:key:alice", vec![read_cap("documents")]);
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    store.put("unrelated", &write_value("r1", "{}"), "svc").await.unwrap();

    let policy = parse_and_validate(r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &mallory, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &attacker, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
    let auth = QueryAuth { policy: &policy, session: &alice, service_id: SERVICE_ID };

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
