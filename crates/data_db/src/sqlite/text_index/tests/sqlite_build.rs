use rusqlite::Connection;

use crate::sqlite::text_index::{TRI_TOKENIZE, WORD_TOKENIZE};

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

    conn.execute(
        &format!("CREATE VIRTUAL TABLE with_prefix USING fts5(f_text, {WORD_TOKENIZE})"),
        [],
    )
    .unwrap();

    let ddl: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'with_prefix'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(ddl.contains("prefix"));

    let no_prefix_tokenize =
        "tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\"";
    conn.execute(
        &format!("CREATE VIRTUAL TABLE without_prefix USING fts5(f_text, {no_prefix_tokenize})"),
        [],
    )
    .unwrap();

    let sample_words = [
        "apple",
        "banana",
        "cherry",
        "date",
        "elderberry",
        "fig",
        "grape",
        "honeydew",
        "kiwi",
        "lemon",
        "mango",
        "nectarine",
        "orange",
        "papaya",
        "quince",
        "raspberry",
        "strawberry",
        "tangerine",
        "ugli",
        "vanilla",
        "watermelon",
        "xigua",
        "yam",
        "zucchini",
    ];

    for word in sample_words {
        conn.execute("INSERT INTO with_prefix(f_text) VALUES (?1)", [word]).unwrap();
        conn.execute("INSERT INTO without_prefix(f_text) VALUES (?1)", [word]).unwrap();
    }

    let size_with_prefix: i64 = conn
        .query_row("SELECT sum(length(block)) FROM with_prefix_data", [], |row| row.get(0))
        .unwrap();
    let size_without_prefix: i64 = conn
        .query_row("SELECT sum(length(block)) FROM without_prefix_data", [], |row| row.get(0))
        .unwrap();

    assert!(size_with_prefix > size_without_prefix);
}

#[test]
fn column_filter_matching() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE VIRTUAL TABLE test_col_filter USING fts5(f_a, f_b)", []).unwrap();

    conn.execute("INSERT INTO test_col_filter(f_a, f_b) VALUES (?1, ?2)", ["abc", "other"])
        .unwrap();
    conn.execute("INSERT INTO test_col_filter(f_a, f_b) VALUES (?1, ?2)", ["other", "abc"])
        .unwrap();

    let mut stmt = conn
        .prepare("SELECT rowid FROM test_col_filter WHERE test_col_filter MATCH 'f_a : \"abc\"'")
        .unwrap();
    let rows: Vec<i64> =
        stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();

    assert_eq!(rows, vec![1]);
}

#[test]
fn pragma_table_info_column_listing() {
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
