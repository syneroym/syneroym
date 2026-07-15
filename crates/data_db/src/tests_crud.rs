//! CRUD, query-filter, batch, and DDL-gating tests for Slice 3A, exercised
//! end-to-end against a real (unencrypted, for test speed) SQLite-backed
//! `ServiceStore`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::Value;
use syneroym_data_keystore::KeyStore;
use tempfile::tempdir;

use crate::{
    ServiceStore, SqliteStorageProvider, StorageProvider,
    host_store::{
        CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
        QueryOptions, RecordWriteValue, SqlValue,
    },
    sqlite::{MAX_BATCH_SIZE, MAX_QUERY_PAGE_SIZE},
};

async fn setup_store() -> Box<dyn ServiceStore> {
    let dir = tempdir().unwrap();
    // Leak the tempdir so it outlives the store for the duration of the test
    // process; test isolation is still per-test via the unique tempdir path.
    let dir = Box::leak(Box::new(dir));
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());
    provider.open_service_db("crud-test-svc", &key_store).await.unwrap()
}

fn schema(name: &str) -> CollectionSchema {
    CollectionSchema {
        name: name.to_string(),
        indexes: vec![IndexDefinition { field_name: "age".to_string(), type_: IndexType::Numeric }],
    }
}

fn write_value(id: &str, json: &str) -> RecordWriteValue {
    RecordWriteValue { id: id.to_string(), payload: json.as_bytes().to_vec() }
}

#[tokio::test]
async fn test_put_get_patch_correctness() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();

    store
        .put("people", &write_value("p1", r#"{"name": "alice", "age": 30}"#), "creator-1")
        .await
        .unwrap();
    let got = store.get("people", "p1").await.unwrap().unwrap();
    assert_eq!(got.id, "p1");
    assert_eq!(got.creator_id, "creator-1");
    let payload: Value = serde_json::from_slice(&got.payload).unwrap();
    assert_eq!(payload["name"], "alice");
    assert_eq!(payload["age"], 30);

    store.patch("people", "p1", br#"{"age": 31, "nickname": "al"}"#).await.unwrap();
    let patched = store.get("people", "p1").await.unwrap().unwrap();
    let payload: Value = serde_json::from_slice(&patched.payload).unwrap();
    assert_eq!(payload["name"], "alice");
    assert_eq!(payload["age"], 31);
    assert_eq!(payload["nickname"], "al");
    assert_eq!(patched.created_at, got.created_at, "created_at must be immutable across updates");
    assert!(patched.updated_at >= got.updated_at);
}

#[tokio::test]
async fn test_get_returns_ok_none_for_missing_record() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    assert!(store.get("people", "does-not-exist").await.unwrap().is_none());
}

#[tokio::test]
async fn test_query_operators_end_to_end() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", r#"{"name": "alice", "age": 30}"#), "c").await.unwrap();
    store.put("people", &write_value("p2", r#"{"name": "bob", "age": 17}"#), "c").await.unwrap();
    store.put("people", &write_value("p3", r#"{"name": "carol", "age": 45}"#), "c").await.unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"age": {"$gt": 18}}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let result = store.query("people", &opts).await.unwrap();
    let mut ids: Vec<_> = result.records.iter().map(|r| r.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["p1", "p3"]);

    let opts_in = QueryOptions {
        filter: Some(r#"{"name": {"$in": ["bob", "carol"]}}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let result_in = store.query("people", &opts_in).await.unwrap();
    let mut ids_in: Vec<_> = result_in.records.iter().map(|r| r.id.clone()).collect();
    ids_in.sort();
    assert_eq!(ids_in, vec!["p2", "p3"]);

    let opts_and = QueryOptions {
        filter: Some(r#"{"$and": [{"age": {"$gt": 18}}, {"name": "alice"}]}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let result_and = store.query("people", &opts_and).await.unwrap();
    assert_eq!(result_and.records.len(), 1);
    assert_eq!(result_and.records[0].id, "p1");
}

#[tokio::test]
async fn test_query_dot_notation() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store
        .put("people", &write_value("p1", r#"{"address": {"city": "London"}}"#), "c")
        .await
        .unwrap();
    store
        .put("people", &write_value("p2", r#"{"address": {"city": "Paris"}}"#), "c")
        .await
        .unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"address.city": "London"}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let result = store.query("people", &opts).await.unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].id, "p1");
}

#[tokio::test]
async fn test_query_empty_list_when_no_match() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", r#"{"name": "alice"}"#), "c").await.unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"name": "nobody"}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let result = store.query("people", &opts).await.unwrap();
    assert!(result.records.is_empty());
    assert_eq!(result.next_cursor, None);
}

#[tokio::test]
async fn test_query_cursor_pagination_disjoint_pages() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    for i in 0..10 {
        store.put("people", &write_value(&format!("p{i:02}"), "{}"), "c").await.unwrap();
    }

    let page1 = store
        .query("people", &QueryOptions { filter: None, limit: Some(4), cursor: None })
        .await
        .unwrap();
    assert_eq!(page1.records.len(), 4);
    let cursor = page1.next_cursor.clone().expect("expected a next cursor for page 1");

    let page2 = store
        .query("people", &QueryOptions { filter: None, limit: Some(4), cursor: Some(cursor) })
        .await
        .unwrap();
    assert_eq!(page2.records.len(), 4);

    let page1_ids: Vec<_> = page1.records.iter().map(|r| r.id.clone()).collect();
    let page2_ids: Vec<_> = page2.records.iter().map(|r| r.id.clone()).collect();
    assert!(page1_ids.iter().all(|id| !page2_ids.contains(id)), "pages must be disjoint");
}

#[tokio::test]
async fn test_batch_mutate_rolls_back_all_on_one_failure() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("existing", "{}"), "c").await.unwrap();

    // The Patch targets an id that doesn't exist, which fails -- the earlier
    // Put in the same batch must not persist either.
    let mutations = vec![
        Mutation::Put(write_value("new-1", "{}")),
        Mutation::Patch(PatchMutation {
            id: "does-not-exist".to_string(),
            patch_json: b"{}".to_vec(),
        }),
    ];
    let err = store.batch_mutate("people", &mutations, "c").await.unwrap_err();
    assert!(matches!(err, DataLayerError::SchemaViolation(_)));
    assert!(
        store.get("people", "new-1").await.unwrap().is_none(),
        "partial batch must not persist"
    );
}

#[tokio::test]
async fn test_batch_mutate_exceeding_max_size_rejected() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    let mutations: Vec<_> = (0..(MAX_BATCH_SIZE + 1))
        .map(|i| Mutation::Put(write_value(&format!("p{i}"), "{}")))
        .collect();
    let err = store.batch_mutate("people", &mutations, "c").await.unwrap_err();
    assert!(matches!(err, DataLayerError::SchemaViolation(_)));
}

#[tokio::test]
async fn test_delete_many_returns_affected_row_count() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", r#"{"age": 10}"#), "c").await.unwrap();
    store.put("people", &write_value("p2", r#"{"age": 20}"#), "c").await.unwrap();
    store.put("people", &write_value("p3", r#"{"age": 30}"#), "c").await.unwrap();

    let deleted = store.delete_many("people", Some(r#"{"age": {"$gte": 20}}"#)).await.unwrap();
    assert_eq!(deleted, 2);
    assert!(store.get("people", "p1").await.unwrap().is_some());
    assert!(store.get("people", "p2").await.unwrap().is_none());
}

#[tokio::test]
async fn test_delete_missing_record_is_idempotent() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.delete("people", "does-not-exist").await.unwrap();
}

#[tokio::test]
async fn test_unsupported_operator_returns_schema_violation() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    let opts = QueryOptions {
        filter: Some(r#"{"name": {"$lookup": 1}}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let err = store.query("people", &opts).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::SchemaViolation(msg) if msg.contains("unsupported operator"))
    );
}

#[tokio::test]
async fn test_sql_injection_via_filter_value_is_safely_bound() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", r#"{"name": "alice"}"#), "c").await.unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"name": "'; DROP TABLE people; --"}"#.to_string()),
        limit: None,
        cursor: None,
    };
    let result = store.query("people", &opts).await.unwrap();
    assert!(result.records.is_empty());

    // The table must still exist and be queryable afterwards.
    assert!(store.get("people", "p1").await.unwrap().is_some());
}

#[tokio::test]
async fn test_filter_nested_over_10_levels_rejected() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    let mut json = "1".to_string();
    for i in 0..12 {
        json = format!(r#"{{"f{i}": {json}}}"#);
    }
    let opts = QueryOptions { filter: Some(json), limit: None, cursor: None };
    let err = store.query("people", &opts).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::SchemaViolation(msg) if msg.contains("too deeply nested"))
    );
}

#[tokio::test]
async fn test_updated_at_is_host_injected_discarding_guest_value() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", "{}"), "c").await.unwrap();
    let before = store.get("people", "p1").await.unwrap().unwrap();

    // The guest embeds an `updated_at` key inside the merge-patch payload
    // itself -- this must have no effect on the host-injected `updated-at`
    // column, which is unconditionally recomputed from the host clock.
    store.patch("people", "p1", br#"{"updated_at": 1}"#).await.unwrap();
    let after = store.get("people", "p1").await.unwrap().unwrap();
    assert!(after.updated_at >= before.updated_at);
    assert_ne!(after.updated_at, 1);
}

#[tokio::test]
async fn test_execute_ddl_succeeds_and_is_queryable() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.execute_ddl("ALTER TABLE people ADD COLUMN nickname TEXT").await.unwrap();
    store.put("people", &write_value("p1", "{}"), "c").await.unwrap();
    assert!(store.get("people", "p1").await.unwrap().is_some());
}

#[tokio::test]
async fn test_execute_ddl_invalid_syntax_rejected_before_mutation() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    let err = store.execute_ddl("NOT VALID SQL").await.unwrap_err();
    assert!(matches!(err, DataLayerError::Internal(_)));
}

#[tokio::test]
async fn test_creator_id_is_always_host_supplied() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", "{}"), "the-deploying-service-id").await.unwrap();
    let got = store.get("people", "p1").await.unwrap().unwrap();
    assert_eq!(got.creator_id, "the-deploying-service-id");
}

#[tokio::test]
async fn test_query_missing_collection_is_an_error_not_empty_list() {
    let store = setup_store().await;
    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let err = store.query("never_created", &opts).await.unwrap_err();
    assert!(matches!(err, DataLayerError::CollectionNotFound));
}

// -- query-raw (Slice B5, ADR-0011) --------------------------------------

async fn seeded_people_store() -> Box<dyn ServiceStore> {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    store.put("people", &write_value("p1", r#"{"name": "alice", "age": 30}"#), "c").await.unwrap();
    store.put("people", &write_value("p2", r#"{"name": "bob", "age": 17}"#), "c").await.unwrap();
    store.put("people", &write_value("p3", r#"{"name": "carol", "age": 45}"#), "c").await.unwrap();
    store
}

/// `SqlValue` doesn't derive `PartialEq` (only `Clone`/`Debug`/serde); compare
/// via its already-derived `Serialize` impl instead of matching every arm by
/// hand.
fn rows_as_json(rows: &[Vec<SqlValue>]) -> Value {
    serde_json::to_value(rows).unwrap()
}

#[tokio::test]
async fn test_query_raw_projects_arbitrary_columns() {
    let store = seeded_people_store().await;
    let result = store
        .query_raw(
            "SELECT json_extract(payload,'$.name') AS name, json_extract(payload,'$.age') AS age \
             FROM people ORDER BY name",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(result.columns, vec!["name".to_string(), "age".to_string()]);
    assert_eq!(
        rows_as_json(&result.rows),
        rows_as_json(&[
            vec![SqlValue::Text("alice".to_string()), SqlValue::Integer(30)],
            vec![SqlValue::Text("bob".to_string()), SqlValue::Integer(17)],
            vec![SqlValue::Text("carol".to_string()), SqlValue::Integer(45)],
        ])
    );
}

#[tokio::test]
async fn test_query_raw_aggregation() {
    let store = seeded_people_store().await;
    let result = store.query_raw("SELECT count(*) AS n FROM people", &[]).await.unwrap();
    assert_eq!(result.columns, vec!["n".to_string()]);
    assert_eq!(rows_as_json(&result.rows), rows_as_json(&[vec![SqlValue::Integer(3)]]));
}

#[tokio::test]
async fn test_query_raw_binds_params_no_injection() {
    let store = seeded_people_store().await;
    let result = store
        .query_raw(
            "SELECT id FROM people WHERE json_extract(payload,'$.name') = ?",
            &[SqlValue::Text("x'; DROP TABLE people; --".to_string())],
        )
        .await
        .unwrap();
    assert!(result.rows.is_empty());

    // The table must still exist and be fully queryable afterwards.
    let count = store.query_raw("SELECT count(*) AS n FROM people", &[]).await.unwrap();
    assert_eq!(rows_as_json(&count.rows), rows_as_json(&[vec![SqlValue::Integer(3)]]));
}

#[tokio::test]
async fn test_query_raw_rejects_write_statements() {
    let store = seeded_people_store().await;
    for sql in [
        r#"INSERT INTO people (id, payload, creator_id, created_at, updated_at) VALUES ('x', '{}', 'c', 0, 0)"#,
        "UPDATE people SET payload = '{}' WHERE id = 'p1'",
        "DELETE FROM people",
        "DROP TABLE people",
        "CREATE TABLE t (id TEXT)",
    ] {
        let err = store.query_raw(sql, &[]).await.unwrap_err();
        assert!(
            matches!(err, DataLayerError::PermissionDenied),
            "expected permission-denied for {sql:?}, got {err:?}"
        );
    }

    // None of the rejected statements executed.
    let count = store.query_raw("SELECT count(*) AS n FROM people", &[]).await.unwrap();
    assert_eq!(rows_as_json(&count.rows), rows_as_json(&[vec![SqlValue::Integer(3)]]));
}

#[tokio::test]
async fn test_query_raw_blob_column_is_schema_violation() {
    let store = seeded_people_store().await;
    let err = store.query_raw("SELECT x'00'", &[]).await.unwrap_err();
    assert!(matches!(err, DataLayerError::SchemaViolation(_)));
}

#[tokio::test]
async fn test_query_raw_malformed_sql_is_schema_violation() {
    let store = seeded_people_store().await;
    let err = store.query_raw("SELECT nope FROM", &[]).await.unwrap_err();
    assert!(matches!(err, DataLayerError::SchemaViolation(_)));
}

#[tokio::test]
async fn test_query_raw_exceeding_page_cap_is_quota_exceeded() {
    let store = setup_store().await;
    store.create_collection(&schema("people")).await.unwrap();
    for i in 0..(MAX_QUERY_PAGE_SIZE + 1) {
        store.put("people", &write_value(&format!("p{i:05}"), "{}"), "c").await.unwrap();
    }
    let err = store.query_raw("SELECT id FROM people", &[]).await.unwrap_err();
    assert!(matches!(err, DataLayerError::QuotaExceeded));
}
