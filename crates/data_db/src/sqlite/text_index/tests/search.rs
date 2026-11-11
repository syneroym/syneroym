use std::{sync::Arc, time::Instant};

use rusqlite::{Connection, types::Value as SqlValue};
use serde_json::Map;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{CompiledSieve, DecisionTrace};
use syneroym_ucan::SessionContext;

use crate::{
    QueryAuth, ServiceStore, SqliteStorageProvider, StorageProvider,
    filter::compile_filter_in,
    host_store::{DataLayerError, Mutation, QueryOptions, RecordWriteValue},
    sqlite::{
        query::query_sql,
        schema::do_create_collection,
        text_index::{fields, sync_row},
    },
    test_support::schema_with,
    text_match::TextFields,
};

async fn setup_store() -> Box<dyn ServiceStore> {
    let dir = tempfile::tempdir().unwrap();
    let dir = Box::leak(Box::new(dir));
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());
    provider.open_service_db("test-svc", &key_store).await.unwrap()
}

#[tokio::test]
async fn search_finds_devanagari_and_tamil_words() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let val1 = RecordWriteValue {
        id: "1".to_string(),
        payload: "{\"title\": \"नल वाला\"}".as_bytes().to_vec(),
    };
    let val2 = RecordWriteValue {
        id: "2".to_string(),
        payload: "{\"title\": \"சென்னை வாலா\"}".as_bytes().to_vec(),
    };
    store.put("items", &val1, "user", None).await.unwrap();
    store.put("items", &val2, "user", None).await.unwrap();

    let opts1 = QueryOptions {
        filter: Some("{\"title\": {\"$search\": \"नल\"}}".to_string()),
        cursor: None,
        limit: None,
    };
    let res1 = store.query("items", &opts1, None).await.unwrap();
    assert_eq!(res1.value.records.len(), 1);
    assert_eq!(res1.value.records[0].id, "1");

    let opts2 = QueryOptions {
        filter: Some("{\"title\": {\"$search\": \"வாலா\"}}".to_string()),
        cursor: None,
        limit: None,
    };
    let res2 = store.query("items", &opts2, None).await.unwrap();
    assert_eq!(res2.value.records.len(), 1);
    assert_eq!(res2.value.records[0].id, "2");
}

#[tokio::test]
async fn search_matches_nfc_equivalent_forms() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    // Store with precomposed U+0958
    let val1 = RecordWriteValue {
        id: "1".to_string(),
        payload: "{\"title\": \"\u{0958}िला\"}".as_bytes().to_vec(),
    };
    store.put("items", &val1, "user", None).await.unwrap();

    // Search with decomposed U+0915 U+093C
    let opts1 = QueryOptions {
        filter: Some("{\"title\": {\"$search\": \"\u{0915}\u{093C}िला\"}}".to_string()),
        cursor: None,
        limit: None,
    };
    let res1 = store.query("items", &opts1, None).await.unwrap();
    assert_eq!(res1.value.records.len(), 1);
    assert_eq!(res1.value.records[0].id, "1");

    // Store with decomposed U+0915 U+093C
    let val2 = RecordWriteValue {
        id: "2".to_string(),
        payload: "{\"title\": \"\u{0915}\u{093C}लम\"}".as_bytes().to_vec(),
    };
    store.put("items", &val2, "user", None).await.unwrap();

    // Search with precomposed U+0958
    let opts2 = QueryOptions {
        filter: Some("{\"title\": {\"$search\": \"\u{0958}लम\"}}".to_string()),
        cursor: None,
        limit: None,
    };
    let res2 = store.query("items", &opts2, None).await.unwrap();
    assert_eq!(res2.value.records.len(), 1);
    assert_eq!(res2.value.records[0].id, "2");
}

#[tokio::test]
async fn search_case_insensitive_and_substring() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let val = RecordWriteValue {
        id: "1".to_string(),
        payload: br#"{"title": "Chennai plumber"}"#.to_vec(),
    };
    store.put("items", &val, "user", None).await.unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"title": {"$search": "PLUMB"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let res = store.query("items", &opts, None).await.unwrap();
    assert_eq!(res.value.records.len(), 1);
    assert_eq!(res.value.records[0].id, "1");
}

#[tokio::test]
async fn short_terms_match_words_and_word_prefixes() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let rows = [
        ("1", r#"{"title": "ek do teen"}"#),
        ("2", r#"{"title": "ekdam"}"#),
        ("3", r#"{"title": "plumber"}"#),
    ];
    for (id, payload) in rows {
        let val = RecordWriteValue { id: id.to_string(), payload: payload.as_bytes().to_vec() };
        store.put("items", &val, "user", None).await.unwrap();
    }

    // "ek" matches "ek do teen" and "ekdam"
    let opts_ek = QueryOptions {
        filter: Some(r#"{"title": {"$search": "ek"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let res_ek = store.query("items", &opts_ek, None).await.unwrap();
    let ids: Vec<String> = res_ek.value.records.into_iter().map(|r| r.id).collect();
    assert_eq!(ids, vec!["1", "2"]);

    // "lu" does not match "plumber" (interior substring not matched by short term)
    let opts_lu = QueryOptions {
        filter: Some(r#"{"title": {"$search": "lu"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let res_lu = store.query("items", &opts_lu, None).await.unwrap();
    assert!(res_lu.value.records.is_empty());

    // "ek teen" requires both words
    let opts_both = QueryOptions {
        filter: Some(r#"{"title": {"$search": "ek teen"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let res_both = store.query("items", &opts_both, None).await.unwrap();
    assert_eq!(res_both.value.records.len(), 1);
    assert_eq!(res_both.value.records[0].id, "1");
}

#[tokio::test]
async fn search_term_cleaning_finds_words() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let val = RecordWriteValue {
        id: "1".to_string(),
        payload: br#"{"title": "Chennai plumber"}"#.to_vec(),
    };
    store.put("items", &val, "user", None).await.unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"title": {"$search": "plumber, chennai"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let res = store.query("items", &opts, None).await.unwrap();
    assert_eq!(res.value.records.len(), 1);
    assert_eq!(res.value.records[0].id, "1");
}

#[tokio::test]
async fn search_does_not_match_other_field() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string(), "summary".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let val = RecordWriteValue {
        id: "1".to_string(),
        payload: br#"{"title": "first", "summary": "secret plumber"}"#.to_vec(),
    };
    store.put("items", &val, "user", None).await.unwrap();

    let opts = QueryOptions {
        filter: Some(r#"{"title": {"$search": "plumber"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let res = store.query("items", &opts, None).await.unwrap();
    assert!(res.value.records.is_empty());
}

#[tokio::test]
async fn search_and_composition_and_equality_filter() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let rows = [
        ("1", r#"{"title": "ek do teen", "status": "active"}"#),
        ("2", r#"{"title": "ek do", "status": "active"}"#),
        ("3", r#"{"title": "ek do teen", "status": "inactive"}"#),
    ];
    for (id, payload) in rows {
        let val = RecordWriteValue { id: id.to_string(), payload: payload.as_bytes().to_vec() };
        store.put("items", &val, "user", None).await.unwrap();
    }

    // $and of two searches
    let opts_and = QueryOptions {
        filter: Some(
            r#"{"$and": [{"title": {"$search": "ek"}}, {"title": {"$search": "teen"}}]}"#
                .to_string(),
        ),
        cursor: None,
        limit: None,
    };
    let res_and = store.query("items", &opts_and, None).await.unwrap();
    let ids: Vec<String> = res_and.value.records.into_iter().map(|r| r.id).collect();
    assert_eq!(ids, vec!["1", "3"]);

    // $search next to equality filter
    let opts_eq = QueryOptions {
        filter: Some(
            r#"{"$and": [{"title": {"$search": "teen"}}, {"status": "active"}]}"#.to_string(),
        ),
        cursor: None,
        limit: None,
    };
    let res_eq = store.query("items", &opts_eq, None).await.unwrap();
    assert_eq!(res_eq.value.records.len(), 1);
    assert_eq!(res_eq.value.records[0].id, "1");
}

#[tokio::test]
async fn search_paging_cursor() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    for i in 1..=25 {
        let id = format!("item_{i:02}");
        let payload = format!("{{\"title\": \"match plumber item {i}\"}}");
        let val = RecordWriteValue { id, payload: payload.into_bytes() };
        store.put("items", &val, "user", None).await.unwrap();
    }

    let filter = Some(r#"{"title": {"$search": "plumber"}}"#.to_string());
    let mut all_ids = Vec::new();
    let mut cursor = None;

    loop {
        let opts = QueryOptions { filter: filter.clone(), cursor, limit: Some(10) };
        let res = store.query("items", &opts, None).await.unwrap();
        all_ids.extend(res.value.records.into_iter().map(|r| r.id));
        cursor = res.value.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(all_ids.len(), 25);
    let mut sorted = all_ids.clone();
    sorted.sort();
    assert_eq!(all_ids, sorted);
}

#[tokio::test]
async fn search_cls_masked_field_refused() {
    let store = setup_store().await;

    let schema = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema).await.unwrap();

    let resolved_sieve = CompiledSieve {
        where_clause: "1=1".to_string(),
        params: Vec::new(),
        masked_fields: vec!["title".to_string()],
        where_caveats: Vec::new(),
        trace: DecisionTrace::default(),
        abac_permissions: Vec::new(),
    };
    let policy =
        syneroym_fdae::parse_and_validate(r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let session = SessionContext {
        subject_did: "did:key:alice".to_string(),
        anchor_did: None,
        capabilities: Vec::new(),
        claims: Map::new(),
        verified_at_secs: 0,
    };
    let auth = QueryAuth {
        policy: &policy,
        session: &session,
        service_id: "test-svc",
        resolved_sieve: Some(resolved_sieve),
    };

    let opts = QueryOptions {
        filter: Some(r#"{"title": {"$search": "test"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let err = store.query("items", &opts, Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

#[tokio::test]
async fn search_on_undeclared_field_or_collection_without_index_refused() {
    let store = setup_store().await;

    let schema_with_idx = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema_with_idx).await.unwrap();

    let schema_no_idx = schema_with("plain", vec![], vec![]);
    store.create_collection(&schema_no_idx).await.unwrap();

    let opts_undeclared = QueryOptions {
        filter: Some(r#"{"summary": {"$search": "x"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let err1 = store.query("items", &opts_undeclared, None).await.unwrap_err();
    match err1 {
        DataLayerError::SchemaViolation(msg) => {
            assert_eq!(msg, "'summary' is not a text field of collection 'items'");
        }
        res => panic!("expected SchemaViolation, got {res:?}"),
    }

    let opts_no_idx = QueryOptions {
        filter: Some(r#"{"title": {"$search": "x"}}"#.to_string()),
        cursor: None,
        limit: None,
    };
    let err2 = store.query("plain", &opts_no_idx, None).await.unwrap_err();
    match err2 {
        DataLayerError::SchemaViolation(msg) => {
            assert_eq!(msg, "'title' is not a text field of collection 'plain'");
        }
        res => panic!("expected SchemaViolation, got {res:?}"),
    }
}

#[test]
fn search_plan_never_scans_the_collection() {
    let mut conn = Connection::open_in_memory().unwrap();
    let schema = schema_with("c", vec![], vec!["title".to_string()]);
    do_create_collection(&mut conn, &schema).unwrap();

    let tf = fields(&conn, "c").unwrap();
    for i in 1..=150 {
        let payload = format!("{{\"title\": \"ek plumber {i}\", \"status\": \"active\"}}");
        conn.execute(
            "INSERT INTO c (id, payload, creator_id, created_at, updated_at) VALUES (?1, ?2, \
             'user', 0, 0)",
            [&format!("c{i:03}"), &payload],
        )
        .unwrap();
        let rowid = conn.last_insert_rowid();
        let val: serde_json::Value = serde_json::from_str(&payload).unwrap();
        sync_row(&conn, "c", &tf, rowid, &val).unwrap();
    }

    let text_fields = ["title".to_string()];
    let text = TextFields { collection: "c", fields: &text_fields };

    type PlanCase<'a> = (Option<&'a str>, Option<(String, Vec<SqlValue>)>, Option<&'a str>);
    let cases: Vec<PlanCase<'_>> = vec![
        // Long term
        (Some(r#"{"title": {"$search": "plumber"}}"#), None, None),
        // Short term
        (Some(r#"{"title": {"$search": "ek"}}"#), None, None),
        // Both
        (Some(r#"{"title": {"$search": "ek plumber"}}"#), None, None),
        // Both with cursor
        (Some(r#"{"title": {"$search": "ek plumber"}}"#), None, Some("c010")),
        // $and of two searches
        (
            Some(r#"{"$and": [{"title": {"$search": "ek"}}, {"title": {"$search": "plumber"}}]}"#),
            None,
            None,
        ),
        // Search next to equality and sieve
        (
            Some(r#"{"$and": [{"title": {"$search": "plumber"}}, {"status": "active"}]}"#),
            Some(("creator_id = ?".to_string(), vec![SqlValue::Text("user".into())])),
            None,
        ),
    ];

    for analyze in [false, true] {
        if analyze {
            conn.execute("ANALYZE", []).unwrap();
        }
        for (filter_json, sieve, cursor) in &cases {
            let compiled = compile_filter_in(*filter_json, Some(&text)).unwrap();
            let (sql, bound_params) = query_sql("c", sieve.clone(), compiled.as_ref(), *cursor, 10);

            let mut explain_stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan_rows = explain_stmt
                .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap();

            let mut lines = Vec::new();
            for r in plan_rows {
                lines.push(r.unwrap());
            }

            let has_fts_scan = lines.iter().any(|l| {
                l.contains("SCAN _text_c_tri VIRTUAL TABLE")
                    || l.contains("SCAN _text_c_word VIRTUAL TABLE")
            });
            assert!(has_fts_scan, "expected FTS scan in plan (analyze={analyze}): {lines:?}");

            for line in &lines {
                assert!(
                    !line.starts_with("SCAN c ")
                        && !line.starts_with("SCAN c USING")
                        && line != "SCAN c",
                    "plan scanned collection table (analyze={analyze}): {lines:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn measure_rebuild_and_query_10000_rows() {
    let store = setup_store().await;
    let schema_plain = schema_with("items", vec![], vec![]);
    store.create_collection(&schema_plain).await.unwrap();

    // Insert 10,000 rows in batches using batch_mutate (max batch size is 200)
    for chunk in 0..50 {
        let mut ops = Vec::with_capacity(200);
        for i in 0..200 {
            let id = format!("doc_{:05}", chunk * 200 + i);
            let payload = format!(
                "{{\"title\": \"Chennai plumber {}\", \"status\": \"active\"}}",
                chunk * 200 + i
            );
            ops.push(Mutation::Put(RecordWriteValue { id, payload: payload.into_bytes() }));
        }
        store.batch_mutate("items", &ops, "user", None).await.unwrap();
    }

    // Measure rebuild: call create_collection with text_fields to declare and
    // backfill
    let rebuild_start = Instant::now();
    let schema_text = schema_with("items", vec![], vec!["title".to_string()]);
    store.create_collection(&schema_text).await.unwrap();
    let rebuild_duration = rebuild_start.elapsed();

    // Measure query on 10,000 rows
    let query_start = Instant::now();
    let res = store
        .query(
            "items",
            &QueryOptions {
                filter: Some(r#"{"title": {"$search": "Chennai plumber 5000"}}"#.to_string()),
                cursor: None,
                limit: Some(10),
            },
            None,
        )
        .await
        .unwrap();
    let query_duration = query_start.elapsed();

    assert_eq!(res.value.records.len(), 1);
    eprintln!(
        "MEASURE_RESULT: 10,000 rows rebuild={:?}, query={:?}",
        rebuild_duration, query_duration
    );
}

#[tokio::test]
async fn measure_put_plain_collection() {
    let store = setup_store().await;
    let schema_plain = schema_with("plain", vec![], vec![]);
    store.create_collection(&schema_plain).await.unwrap();

    let start = Instant::now();
    for i in 0..1000 {
        let id = format!("doc_{i:04}");
        let payload = format!("{{\"field\": \"value {i}\"}}");
        store
            .put(
                "plain",
                &RecordWriteValue { id: id.clone(), payload: payload.into_bytes() },
                "user",
                None,
            )
            .await
            .unwrap();
    }
    let duration = start.elapsed();
    eprintln!("MEASURE_RESULT: 1,000 puts on plain collection: {:?}", duration);
}
