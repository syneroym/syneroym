use rusqlite::Connection;

use crate::{
    host_store::{CollectionSchema, DataLayerError, IndexDefinition, IndexType, RecordWriteValue},
    sqlite::{
        mutation::do_authorized_put,
        query::do_list_collections,
        schema::{do_create_collection, do_drop_collection},
        text_index::fields,
    },
    test_support::schema_with,
};

#[test]
fn declare_creates_tables_and_excludes_from_list_collections() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let tri_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_items_tri')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tri_exists);

    let word_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_items_word')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(word_exists);

    let key_ver: u32 = conn
        .query_row("SELECT key_version FROM _text_index WHERE collection = 'items'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(key_ver, 1);

    let collections = do_list_collections(&mut conn).unwrap();
    assert_eq!(collections, vec!["items"]);
}

#[test]
fn declare_backfills_pre_existing_rows() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema_plain = schema_with("items", vec![], vec![]);
    do_create_collection(&mut conn, &schema_plain).unwrap();

    conn.execute(
        "INSERT INTO items (id, payload, creator_id, created_at, updated_at) VALUES (?1, ?2, \
         'user', 0, 0)",
        ["1", r#"{"title": "hello world"}"#],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items (id, payload, creator_id, created_at, updated_at) VALUES (?1, ?2, \
         'user', 0, 0)",
        ["2", r#"{"title": "other content"}"#],
    )
    .unwrap();

    let schema_indexed = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema_indexed).unwrap();

    let tri_matches: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_items_tri WHERE _text_items_tri MATCH 'world'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tri_matches, 1);

    let word_matches: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_items_word WHERE _text_items_word MATCH 'hello'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(word_matches, 1);
}

#[test]
fn declare_idempotent_no_rebuild_if_same_fields() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    conn.execute("INSERT INTO _text_items_word (rowid, f_title) VALUES (999, 'marker')", [])
        .unwrap();

    do_create_collection(&mut conn, &schema).unwrap();

    let marker_count: i64 = conn
        .query_row("SELECT count(*) FROM _text_items_word WHERE rowid = 999", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker_count, 1);
}

#[test]
fn declare_field_order_does_not_rebuild() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema1 = schema_with("items", vec![], vec!["title".to_string(), "summary".to_string()]);
    do_create_collection(&mut conn, &schema1).unwrap();

    conn.execute(
        "INSERT INTO _text_items_word (rowid, f_title, f_summary) VALUES (999, 'm1', 'm2')",
        [],
    )
    .unwrap();

    let schema2 = schema_with("items", vec![], vec!["summary".to_string(), "title".to_string()]);
    do_create_collection(&mut conn, &schema2).unwrap();

    let marker_count: i64 = conn
        .query_row("SELECT count(*) FROM _text_items_word WHERE rowid = 999", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker_count, 1);
}

#[test]
fn declare_case_difference_rebuilds() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema1 = schema_with("items", vec![], vec!["Title".to_string()]);
    do_create_collection(&mut conn, &schema1).unwrap();

    conn.execute("INSERT INTO _text_items_word (rowid, f_Title) VALUES (999, 'marker')", [])
        .unwrap();

    let schema2 = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema2).unwrap();

    let marker_count: i64 = conn
        .query_row("SELECT count(*) FROM _text_items_word WHERE rowid = 999", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker_count, 0);

    let cols = fields(&conn, "items").unwrap();
    assert_eq!(cols, vec!["title"]);
}

#[test]
fn declare_shares_text_index_row_case_insensitively() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("Foo", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let coll_in_index: String =
        conn.query_row("SELECT collection FROM _text_index", [], |row| row.get(0)).unwrap();
    assert_eq!(coll_in_index, "foo");

    let count: i64 =
        conn.query_row("SELECT count(*) FROM _text_index", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn declare_empty_text_fields_keeps_existing_text_index() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    conn.execute("INSERT INTO _text_items_word (rowid, f_title) VALUES (999, 'marker')", [])
        .unwrap();

    let schema_empty = schema_with("items", vec![], vec![]);
    do_create_collection(&mut conn, &schema_empty).unwrap();

    let marker_count: i64 = conn
        .query_row("SELECT count(*) FROM _text_items_word WHERE rowid = 999", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker_count, 1);

    let cols = fields(&conn, "items").unwrap();
    assert_eq!(cols, vec!["title"]);
}

#[test]
fn declare_adding_field_rebuilds() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema1 = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema1).unwrap();

    let schema2 = schema_with("items", vec![], vec!["title".to_string(), "summary".to_string()]);
    do_create_collection(&mut conn, &schema2).unwrap();

    let cols = fields(&conn, "items").unwrap();
    assert_eq!(cols, vec!["title", "summary"]);
}

#[test]
fn declare_key_version_mismatch_rebuilds() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    conn.execute("INSERT INTO _text_items_word (rowid, f_title) VALUES (999, 'marker')", [])
        .unwrap();
    conn.execute("UPDATE _text_index SET key_version = 0 WHERE collection = 'items'", []).unwrap();

    do_create_collection(&mut conn, &schema).unwrap();

    let marker_count: i64 = conn
        .query_row("SELECT count(*) FROM _text_items_word WHERE rowid = 999", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker_count, 0);

    let key_ver: u32 = conn
        .query_row("SELECT key_version FROM _text_index WHERE collection = 'items'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(key_ver, 1);
}

#[test]
fn declare_missing_tri_table_rebuilds() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    conn.execute("DROP TABLE _text_items_tri", []).unwrap();

    do_create_collection(&mut conn, &schema).unwrap();

    let tri_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_items_tri')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tri_exists);
}

#[test]
fn declare_backfill_skips_non_string_values() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema_plain = schema_with("items", vec![], vec![]);
    do_create_collection(&mut conn, &schema_plain).unwrap();

    conn.execute(
        "INSERT INTO items (id, payload, creator_id, created_at, updated_at) VALUES ('1', \
         '{\"title\": 12345}', 'user', 0, 0)",
        [],
    )
    .unwrap();

    let schema_indexed = schema_with("items", vec![], vec!["title".to_string()]);
    assert!(do_create_collection(&mut conn, &schema_indexed).is_ok());

    let f_val: Option<String> = conn
        .query_row("SELECT f_title FROM _text_items_word WHERE rowid = 1", [], |row| row.get(0))
        .unwrap();
    assert!(f_val.is_none());
}

#[test]
fn declare_refusals() {
    let mut conn = Connection::open_in_memory().unwrap();

    let schema_text_coll = schema_with("_text_x", vec![], vec!["title".to_string()]);
    match do_create_collection(&mut conn, &schema_text_coll) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "invalid identifier: _text_x");
        }
        res => panic!("expected SchemaViolation for _text_x, got {res:?}"),
    }

    let schema_text_caps = schema_with("_TEXT_x", vec![], vec!["title".to_string()]);
    match do_create_collection(&mut conn, &schema_text_caps) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "invalid identifier: _TEXT_x");
        }
        res => panic!("expected SchemaViolation for _TEXT_x, got {res:?}"),
    }

    let nine_fields: Vec<String> = (0..9).map(|i| format!("f_{i}")).collect();
    let schema_too_many = schema_with("items", vec![], nine_fields);
    match do_create_collection(&mut conn, &schema_too_many) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "too many text fields (max 8)");
        }
        res => panic!("expected SchemaViolation for too many text fields, got {res:?}"),
    }

    let schema_dup = schema_with("items", vec![], vec!["title".to_string(), "Title".to_string()]);
    match do_create_collection(&mut conn, &schema_dup) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "text field 'Title' is listed twice");
        }
        res => panic!("expected SchemaViolation for duplicate text field, got {res:?}"),
    }

    let schema_invalid_id = schema_with("items", vec![], vec!["a.b".to_string()]);
    match do_create_collection(&mut conn, &schema_invalid_id) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "invalid identifier: a.b");
        }
        res => panic!("expected SchemaViolation for invalid identifier, got {res:?}"),
    }

    let schema_valid = CollectionSchema {
        name: "test_valid".to_string(),
        indexes: vec![IndexDefinition {
            field_name: "title".to_string(),
            type_: IndexType::String,
        }],
        text_fields: vec!["title".to_string()],
    };
    assert!(do_create_collection(&mut conn, &schema_valid).is_ok());
}

#[test]
fn declare_drop_collection_cleans_up() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    do_drop_collection(&mut conn, "items").unwrap();

    let tri_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_items_tri')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!tri_exists);

    let word_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_items_word')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!word_exists);

    let index_row_count: i64 = conn
        .query_row("SELECT count(*) FROM _text_index WHERE collection = 'items'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(index_row_count, 0);

    let schema_again = schema_with("items", vec![], vec![]);
    assert!(do_create_collection(&mut conn, &schema_again).is_ok());
}

#[test]
fn declare_drop_collection_without_text_fields_leaves_no_text_index_table() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("items", vec![], vec![]);
    do_create_collection(&mut conn, &schema).unwrap();

    do_drop_collection(&mut conn, "items").unwrap();

    let index_table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_index')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!index_table_exists);
}

#[test]
fn declare_two_collections_foo_and_foo_data_keep_own_rows() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema_foo = schema_with("foo", vec![], vec!["title".to_string()]);
    let schema_data = schema_with("foo_data", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema_foo).unwrap();
    do_create_collection(&mut conn, &schema_data).unwrap();

    let val_foo =
        RecordWriteValue { id: "1".to_string(), payload: br#"{"title": "hello"}"#.to_vec() };
    let val_data =
        RecordWriteValue { id: "2".to_string(), payload: br#"{"title": "world"}"#.to_vec() };
    do_authorized_put(&mut conn, "foo", &val_foo, "user", None).unwrap();
    do_authorized_put(&mut conn, "foo_data", &val_data, "user", None).unwrap();

    let foo_match: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_foo_tri WHERE _text_foo_tri MATCH 'hello'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(foo_match, 1);

    let data_match: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_foo_data_tri WHERE _text_foo_data_tri MATCH 'world'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(data_match, 1);

    let foo_wrong: i64 = conn
        .query_row(
            "SELECT count(*) FROM _text_foo_tri WHERE _text_foo_tri MATCH 'world'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(foo_wrong, 0);
}
