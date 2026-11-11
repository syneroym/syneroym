use rusqlite::Connection;

use super::{TRI_TOKENIZE, WORD_TOKENIZE};

#[test]
fn word_table_tokenization_and_indic_marks() {
    let conn = Connection::open_in_memory().unwrap();
    let sql = format!("CREATE VIRTUAL TABLE test_word USING fts5(f_text, {WORD_TOKENIZE})");
    conn.execute(&sql, []).unwrap();

    let rows = ["ek", "ekdam", "nal wala", "नल वाला", "சென்னை வாலா"];
    for text in rows {
        conn.execute("INSERT INTO test_word(f_text) VALUES (?1)", [text]).unwrap();
    }

    let query = |pattern: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT f_text FROM test_word WHERE test_word MATCH ?1 ORDER BY rowid")
            .unwrap();
        let rows = stmt.query_map([pattern], |row| row.get(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };

    assert_eq!(query(r#""e"*"#), vec!["ek", "ekdam"]);
    assert_eq!(query(r#""ek"*"#), vec!["ek", "ekdam"]);
    assert_eq!(query(r#""नल"*"#), vec!["नल वाला"]);
    assert_eq!(query(r#""வா"*"#), vec!["சென்னை வாலா"]);
    assert!(query(r#""ல"*"#).is_empty());
}

#[test]
fn trigram_table_tokenization_and_length_filter() {
    let conn = Connection::open_in_memory().unwrap();
    let sql = format!("CREATE VIRTUAL TABLE test_tri USING fts5(f_text, {TRI_TOKENIZE})");
    conn.execute(&sql, []).unwrap();

    conn.execute("INSERT INTO test_tri(f_text) VALUES (?1)", ["plumber"]).unwrap();

    let query = |pattern: &str| -> Vec<String> {
        let mut stmt = conn.prepare("SELECT f_text FROM test_tri WHERE test_tri MATCH ?1").unwrap();
        let rows = stmt.query_map([pattern], |row| row.get(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };

    assert_eq!(query(r#""umb""#), vec!["plumber"]);
    assert!(query(r#""pl""#).is_empty());
}

#[test]
fn prefix_index_verification() {
    let conn = Connection::open_in_memory().unwrap();
    let sql_prefix = format!("CREATE VIRTUAL TABLE test_word USING fts5(f_text, {WORD_TOKENIZE})");
    conn.execute(&sql_prefix, []).unwrap();

    let master_sql: String = conn
        .query_row("SELECT sql FROM sqlite_master WHERE name = 'test_word'", [], |row| row.get(0))
        .unwrap();
    assert!(master_sql.contains("prefix"));

    conn.execute(
        "CREATE VIRTUAL TABLE test_no_prefix USING fts5(
            f_text,
            tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\"
        )",
        [],
    )
    .unwrap();

    // Insert 500 rows with varying short words to populate FTS index structures
    for i in 0..500 {
        let text = format!("word_{i} ek_{i} ab_{i} c_{i} test_entry_{i}");
        conn.execute("INSERT INTO test_word(f_text) VALUES (?1)", [&text]).unwrap();
        conn.execute("INSERT INTO test_no_prefix(f_text) VALUES (?1)", [&text]).unwrap();
    }

    let size_with_prefix: i64 = conn
        .query_row("SELECT sum(length(block)) FROM test_word_data", [], |row| row.get(0))
        .unwrap();
    let size_without_prefix: i64 = conn
        .query_row("SELECT sum(length(block)) FROM test_no_prefix_data", [], |row| row.get(0))
        .unwrap();

    assert!(
        size_with_prefix > size_without_prefix,
        "expected size with prefix ({size_with_prefix}) to be larger than without \
         ({size_without_prefix})"
    );
}

#[test]
fn column_filter() {
    let conn = Connection::open_in_memory().unwrap();
    let sql = format!("CREATE VIRTUAL TABLE test_cols USING fts5(f_a, f_b, {TRI_TOKENIZE})");
    conn.execute(&sql, []).unwrap();

    conn.execute("INSERT INTO test_cols(rowid, f_a, f_b) VALUES (1, 'abc', 'def')", []).unwrap();
    conn.execute("INSERT INTO test_cols(rowid, f_a, f_b) VALUES (2, 'xyz', 'abc')", []).unwrap();

    let mut stmt =
        conn.prepare("SELECT rowid FROM test_cols WHERE test_cols MATCH 'f_a : \"abc\"'").unwrap();
    let matched_rowids: Vec<i64> =
        stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();

    assert_eq!(matched_rowids, vec![1]);
}

#[test]
fn pragma_table_info() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE VIRTUAL TABLE test_fts USING fts5(f_a, f_b)", []).unwrap();

    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('test_fts')").unwrap();
    let col_names: Vec<String> =
        stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(col_names, vec!["f_a", "f_b"]);

    let mut stmt_missing =
        conn.prepare("SELECT name FROM pragma_table_info('nonexistent_table')").unwrap();
    let missing_cols: Vec<String> =
        stmt_missing.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert!(missing_cols.is_empty());
}

#[test]
fn upsert_returning_rowid_stability() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE test_coll (id TEXT PRIMARY KEY, payload JSON)", []).unwrap();

    let insert_sql = "INSERT INTO test_coll (id, payload) VALUES (?1, ?2) ON CONFLICT(id) DO \
                      UPDATE SET payload = excluded.payload RETURNING rowid";

    let rowid_1: i64 =
        conn.query_row(insert_sql, ["id1", r#"{"title":"initial"}"#], |row| row.get(0)).unwrap();

    let rowid_2: i64 =
        conn.query_row(insert_sql, ["id1", r#"{"title":"updated"}"#], |row| row.get(0)).unwrap();

    assert_eq!(rowid_1, rowid_2);
}

#[test]
fn text_fields_schema_validation() {
    use crate::{
        host_store::{CollectionSchema, DataLayerError, IndexDefinition, IndexType},
        sqlite::schema::do_create_collection,
    };

    let conn = Connection::open_in_memory().unwrap();

    // 9 text fields is refused
    let nine_fields: Vec<String> = (0..9).map(|i| format!("f_{i}")).collect();
    let schema_too_many = CollectionSchema {
        name: "test_coll".to_string(),
        indexes: vec![],
        text_fields: nine_fields,
    };
    match do_create_collection(&conn, &schema_too_many) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "too many text fields (max 8)");
        }
        res => panic!("expected SchemaViolation for too many text fields, got {res:?}"),
    }

    // Case-insensitive duplicate is refused
    let schema_dup = CollectionSchema {
        name: "test_coll".to_string(),
        indexes: vec![],
        text_fields: vec!["title".to_string(), "Title".to_string()],
    };
    match do_create_collection(&conn, &schema_dup) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "text field 'Title' is listed twice");
        }
        res => panic!("expected SchemaViolation for duplicate text field, got {res:?}"),
    }

    // Invalid identifier is refused
    let schema_invalid_id = CollectionSchema {
        name: "test_coll".to_string(),
        indexes: vec![],
        text_fields: vec!["a.b".to_string()],
    };
    match do_create_collection(&conn, &schema_invalid_id) {
        Err(DataLayerError::SchemaViolation(msg)) => {
            assert_eq!(msg, "invalid identifier: a.b");
        }
        res => panic!("expected SchemaViolation for invalid identifier, got {res:?}"),
    }

    // Text field may also be an index field (valid)
    let schema_valid = CollectionSchema {
        name: "test_valid".to_string(),
        indexes: vec![IndexDefinition {
            field_name: "title".to_string(),
            type_: IndexType::String,
        }],
        text_fields: vec!["title".to_string()],
    };
    assert!(do_create_collection(&conn, &schema_valid).is_ok());
}
