use rusqlite::Connection;
use serde_json::json;

use crate::{
    host_store::{DataLayerError, Mutation, RecordWriteValue},
    sqlite::{
        mutation::{
            do_authorized_delete, do_authorized_patch, do_authorized_put, do_batch_mutate,
            do_create, do_delete_many,
        },
        schema::do_create_collection,
        text_index::{remove_rows, sync_row},
    },
    test_support::schema_with,
};

#[test]
fn writes_patch_checks_row_existence_before_parsing_patch() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let err =
        do_authorized_patch(&mut conn, "items", "missing_id", b"invalid json", None).unwrap_err();
    match err {
        DataLayerError::SchemaViolation(msg) => {
            assert!(
                msg.contains("record not found for patch"),
                "expected 'record not found for patch', got {msg}"
            );
        }
        other => panic!("expected SchemaViolation with record not found, got {other:?}"),
    }

    let coll_err =
        do_authorized_patch(&mut conn, "missing_coll", "1", b"invalid json", None).unwrap_err();
    assert!(
        matches!(coll_err, DataLayerError::CollectionNotFound),
        "expected CollectionNotFound, got {coll_err:?}"
    );
}

#[test]
fn writes_put_patch_delete_lifecycle() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let val1 =
        RecordWriteValue { id: "1".to_string(), payload: br#"{"title": "apple orange"}"#.to_vec() };
    do_authorized_put(&mut conn, "items", &val1, "user", None).unwrap();

    let match_apple: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_items_word WHERE _text_items_word MATCH 'apple'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(match_apple, 1);

    let val2 =
        RecordWriteValue { id: "1".to_string(), payload: br#"{"title": "banana grape"}"#.to_vec() };
    do_authorized_put(&mut conn, "items", &val2, "user", None).unwrap();

    let match_apple_after: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_items_word WHERE _text_items_word MATCH 'apple'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(match_apple_after, 0);

    let match_banana: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_items_word WHERE _text_items_word MATCH 'banana'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(match_banana, 1);

    do_authorized_patch(&mut conn, "items", "1", br#"{"title": null}"#, None).unwrap();

    let match_banana_after: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_items_word WHERE _text_items_word MATCH 'banana'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(match_banana_after, 0);

    do_authorized_delete(&mut conn, "items", "1", None).unwrap();

    let tri_rows: i64 =
        conn.query_row("SELECT count(*) FROM _text_items_tri", [], |row| row.get(0)).unwrap();
    assert_eq!(tri_rows, 0);

    let word_rows: i64 =
        conn.query_row("SELECT count(*) FROM _text_items_word", [], |row| row.get(0)).unwrap();
    assert_eq!(word_rows, 0);
}

#[test]
fn writes_patch_untouched_non_string_succeeds_but_patching_non_string_refused() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema_plain = schema_with("items", vec![], vec![]);
    do_create_collection(&mut conn, &schema_plain).unwrap();

    conn.execute(
        "INSERT INTO items (id, payload, creator_id, created_at, updated_at) VALUES ('1', \
         '{\"title\": 42, \"summary\": \"old summary\"}', 'user', 0, 0)",
        [],
    )
    .unwrap();

    let schema_indexed =
        schema_with("items", vec![], vec!["title".to_string(), "summary".to_string()]);
    do_create_collection(&mut conn, &schema_indexed).unwrap();

    let patch_ok = br#"{"summary": "new summary"}"#;
    assert!(do_authorized_patch(&mut conn, "items", "1", patch_ok, None).is_ok());

    let patch_bad = br#"{"title": 99}"#;
    match do_authorized_patch(&mut conn, "items", "1", patch_bad, None) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "text field 'title' must be a string");
        }
        res => panic!("expected SchemaViolation, got {res:?}"),
    }
}

#[test]
fn writes_sync_row_accepts_nul_bytes() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let payload = json!({"title": "hello\0world"});
    assert!(sync_row(&conn, "items", &["title".to_string()], 1, &payload).is_ok());
}

#[test]
fn writes_missing_text_table_maps_to_internal_error() {
    let conn = Connection::open_in_memory().unwrap();
    let payload = json!({"title": "test"});

    match sync_row(&conn, "missing_coll", &["title".to_string()], 1, &payload) {
        Err(DataLayerError::Internal(msg)) => {
            assert_eq!(msg, "text index of 'missing_coll' is missing");
        }
        res => panic!("expected Internal error for missing table in sync_row, got {res:?}"),
    }

    match remove_rows(&conn, "missing_coll", &[1]) {
        Err(DataLayerError::Internal(msg)) => {
            assert_eq!(msg, "text index of 'missing_coll' is missing");
        }
        res => panic!("expected Internal error for missing table in remove_rows, got {res:?}"),
    }
}

#[test]
fn writes_delete_many_removes_from_collection_and_fts_tables() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let r1 = RecordWriteValue {
        id: "1".to_string(),
        payload: br#"{"title": "apple one", "status": "active"}"#.to_vec(),
    };
    let r2 = RecordWriteValue {
        id: "2".to_string(),
        payload: br#"{"title": "apple two", "status": "inactive"}"#.to_vec(),
    };
    let r3 = RecordWriteValue {
        id: "3".to_string(),
        payload: br#"{"title": "banana three", "status": "active"}"#.to_vec(),
    };

    do_authorized_put(&mut conn, "items", &r1, "user", None).unwrap();
    do_authorized_put(&mut conn, "items", &r2, "user", None).unwrap();
    do_authorized_put(&mut conn, "items", &r3, "user", None).unwrap();

    let deleted =
        do_delete_many(&mut conn, "items", Some(r#"{"status": "active"}"#), None).unwrap();
    assert_eq!(deleted, 2);

    let remaining_coll: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM items ORDER BY id").unwrap();
        stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    assert_eq!(remaining_coll, vec!["2"]);

    let remaining_word: i64 =
        conn.query_row("SELECT count(*) FROM _text_items_word", [], |row| row.get(0)).unwrap();
    assert_eq!(remaining_word, 1);

    let remaining_tri: i64 =
        conn.query_row("SELECT count(*) FROM _text_items_tri", [], |row| row.get(0)).unwrap();
    assert_eq!(remaining_tri, 1);
}

#[test]
fn writes_batch_mutate_and_create_rollback() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let mutations = vec![
        Mutation::Put(RecordWriteValue {
            id: "1".to_string(),
            payload: br#"{"title": "first"}"#.to_vec(),
        }),
        Mutation::Put(RecordWriteValue {
            id: "2".to_string(),
            payload: br#"{"title": 12345}"#.to_vec(),
        }),
    ];

    let res = do_batch_mutate(&mut conn, "items", &mutations, "user", None);
    assert!(res.is_err());

    let count_coll: i64 =
        conn.query_row("SELECT count(*) FROM items", [], |row| row.get(0)).unwrap();
    assert_eq!(count_coll, 0);

    let count_word: i64 =
        conn.query_row("SELECT count(*) FROM _text_items_word", [], |row| row.get(0)).unwrap();
    assert_eq!(count_word, 0);

    let val1 =
        RecordWriteValue { id: "1".to_string(), payload: br#"{"title": "existing"}"#.to_vec() };
    do_authorized_put(&mut conn, "items", &val1, "user", None).unwrap();

    let create_vals = vec![
        RecordWriteValue { id: "2".to_string(), payload: br#"{"title": "new item"}"#.to_vec() },
        RecordWriteValue { id: "1".to_string(), payload: br#"{"title": "duplicate id"}"#.to_vec() },
    ];

    let conflict = do_create(&mut conn, "items", &create_vals, "user", None).unwrap();
    assert_eq!(conflict, Some("1".to_string()));

    let count_coll_after: i64 =
        conn.query_row("SELECT count(*) FROM items", [], |row| row.get(0)).unwrap();
    assert_eq!(count_coll_after, 1);

    let count_word_after: i64 =
        conn.query_row("SELECT count(*) FROM _text_items_word", [], |row| row.get(0)).unwrap();
    assert_eq!(count_word_after, 1);
}

#[test]
fn writes_put_non_string_refused() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let val_num = RecordWriteValue { id: "1".to_string(), payload: br#"{"title": 5}"#.to_vec() };
    match do_authorized_put(&mut conn, "items", &val_num, "user", None) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "text field 'title' must be a string");
        }
        res => panic!("expected SchemaViolation, got {res:?}"),
    }

    let count: i64 = conn.query_row("SELECT count(*) FROM items", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn writes_collection_without_text_fields_stores_and_deletes_normally() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec![]);
    do_create_collection(&mut conn, &schema).unwrap();

    let val = RecordWriteValue { id: "1".to_string(), payload: br#"{"title": 12345}"#.to_vec() };
    do_authorized_put(&mut conn, "items", &val, "user", None).unwrap();
    do_authorized_patch(&mut conn, "items", "1", br#"{"title": 67890}"#, None).unwrap();
    do_authorized_delete(&mut conn, "items", "1", None).unwrap();

    let index_table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_index')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!index_table_exists);

    let tri_table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_items_tri')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!tri_table_exists);
}
