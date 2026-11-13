use std::time::Instant;

use rusqlite::params_from_iter;
use syneroym_rpc::Admission;

use super::{open_at, store, store_in_dir, store_with_config};
use crate::store::{
    ConversationConfig, ConversationStore,
    message::{M_MESSAGE_COLUMNS, MESSAGE_COLUMNS},
    search::search_query,
    text_index,
};

fn send_msg(s: &ConversationStore, conv: &str, id: &str, body: &[u8]) {
    s.insert_outgoing_and_enqueue(
        conv,
        id,
        "did:key:zMe",
        1_000,
        "text/plain",
        &[],
        body,
        &[0u8; 64],
        "did:key:zPeer",
        1_000,
        false,
    )
    .unwrap();
}

#[test]
fn column_list_matches_row_to_message() {
    let plain: Vec<_> = MESSAGE_COLUMNS.split(',').map(str::trim).collect();
    let prefixed: Vec<_> = M_MESSAGE_COLUMNS.split(',').map(str::trim).collect();

    assert_eq!(plain.len(), prefixed.len());
    for (p, m) in plain.iter().zip(prefixed.iter()) {
        assert_eq!(*m, format!("m.{p}"));
    }
}

#[test]
fn sync_twice_keeps_one_row() {
    let s = store();
    let conn = s.conn().lock().unwrap();

    text_index::sync(&conn, 42, "hello world").unwrap();
    text_index::sync(&conn, 42, "hello world").unwrap();

    let tri_count: i64 = conn
        .query_row("SELECT count(*) FROM messages_tri WHERE rowid = 42", [], |r| r.get(0))
        .unwrap();
    let word_count: i64 = conn
        .query_row("SELECT count(*) FROM messages_word WHERE rowid = 42", [], |r| r.get(0))
        .unwrap();

    assert_eq!(tri_count, 1);
    assert_eq!(word_count, 1);
}

#[test]
fn remove_without_row_is_ok() {
    let s = store();
    let conn = s.conn().lock().unwrap();
    assert!(text_index::remove(&conn, 999_999).is_ok());
}

#[test]
fn non_utf8_body_is_not_indexed() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();

    s.insert_outgoing_and_enqueue(
        &conv,
        "m:invalid_utf8",
        "did:key:zMe",
        1_000,
        "text/plain",
        &[],
        b"\xff\xfe\xfd",
        &[0u8; 64],
        "did:key:zPeer",
        1_000,
        false,
    )
    .unwrap();

    let conn = s.conn().lock().unwrap();
    let count: i64 = conn.query_row("SELECT count(*) FROM messages_tri", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn short_terms_use_the_word_table_and_never_scan() {
    let (s, _dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();

    send_msg(&s, &conv, "m:1", b"ek do teen");
    send_msg(&s, &conv, "m:2", b"hello");
    send_msg(&s, &conv, "m:3", "एक बात".as_bytes());

    // "ek" is short (2 letters); matches "ek do teen" and "एक बात" (ek is prefix of
    // ek)
    let hits = s.search("ek", Some(&conv), 10).unwrap();
    let mut ids: Vec<_> = hits.into_iter().map(|m| m.id).collect();
    ids.sort();
    assert_eq!(ids, vec!["m:1", "m:3"]);

    // "lo" is short (2 letters); not a prefix of hello, so finds nothing
    let hits_lo = s.search("lo", Some(&conv), 10).unwrap();
    assert_eq!(hits_lo.len(), 0);

    // Populate up to 150 messages to check query plan with and without ANALYZE
    for i in 4..=150 {
        send_msg(&s, &conv, &format!("m:{i}"), format!("padding message {i}").as_bytes());
    }

    let conn = s.conn().lock().unwrap();
    let check_plan = |label: &str| {
        // The three shapes of the statement `search` runs: word only, trigram
        // only, and both.
        let shapes = [
            (None, Some("ek*".to_string())),
            (Some("\"hello\"".to_string()), None),
            (Some("\"hello\"".to_string()), Some("ek*".to_string())),
        ];
        for (tri, word) in shapes {
            let (sql, values) = search_query(tri.clone(), word.clone(), Some(&conv), 10);
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan: Vec<String> = stmt
                .query_map(params_from_iter(values), |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let full = plan.join("\n");
            assert!(
                !full.lines().any(|l| l.contains("SCAN m ") || l.ends_with("SCAN m")),
                "[{label} {tri:?} {word:?}] plan must never scan m table, got:\n{full}"
            );
            assert!(
                full.contains("SEARCH m USING INTEGER PRIMARY KEY"),
                "[{label} {tri:?} {word:?}] plan must use rowid search, got:\n{full}"
            );
        }
    };

    check_plan("without ANALYZE");
    conn.execute("ANALYZE", []).unwrap();
    check_plan("with ANALYZE");
}

#[test]
fn latin_query_finds_devanagari_message_and_reverse() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();

    send_msg(&s, &conv, "m:deva", "नल वाला".as_bytes());
    send_msg(&s, &conv, "m:latn", b"nal wala");

    let hits_latn = s.search("nal wala", Some(&conv), 10).unwrap();
    assert_eq!(hits_latn.len(), 2);

    let hits_deva = s.search("नल वाला", Some(&conv), 10).unwrap();
    assert_eq!(hits_deva.len(), 2);
}

#[test]
fn old_messages_fts_database_is_rebuilt_on_open() {
    let (s, dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();

    send_msg(&s, &conv, "m:1", b"hello world");
    send_msg(&s, &conv, "m:2", b"second message");

    // Simulate old database state: create messages_fts virtual table and drop
    // text_index tables
    {
        let conn = s.conn().lock().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE messages_fts USING fts5(
                body,
                content='messages',
                content_rowid='rowid',
                tokenize = 'trigram case_sensitive 0'
            );
            DROP TABLE messages_tri;
            DROP TABLE messages_word;
            DROP TABLE text_index_version;",
        )
        .unwrap();
    }
    drop(s);

    let s = open_at(&dir, ConversationConfig::default());
    let hits = s.search("hello", None, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m:1");

    let conn = s.conn().lock().unwrap();
    let old_fts: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             'messages_fts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!old_fts, "messages_fts table must be dropped after rebuild");
}

#[test]
fn key_version_change_rebuilds() {
    let (s, dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();

    send_msg(&s, &conv, "m:1", b"persisted search key message");

    {
        let conn = s.conn().lock().unwrap();
        conn.execute("UPDATE text_index_version SET key_version = 1 WHERE id = 1", []).unwrap();
    }
    drop(s);

    let reopened = open_at(&dir, ConversationConfig::default());
    let hits = reopened.search("persisted", None, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m:1");
}

#[test]
fn rebuild_indexes_only_visible_messages() {
    let (s, dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    // 1. Outgoing message (accepted)
    send_msg(&s, &conv, "m:out", b"outgoing message body");

    // 2. Accepted incoming message
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:inc_acc",
            "did:key:zPeer",
            now,
            "text/plain",
            &[],
            b"accepted incoming body",
            &[0u8; 64],
            now,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission("m:inc_acc", &Admission::Accept, now).unwrap();

    // 3. Held incoming message
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:inc_held",
            "did:key:zPeer",
            now + 1,
            "text/plain",
            &[],
            b"held incoming body",
            &[0u8; 64],
            now + 1,
            101,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission("m:inc_held", &Admission::Hold("test".into()), now + 1).unwrap();

    // 4. Undecided incoming message
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:inc_undecided",
            "did:key:zPeer",
            now + 2,
            "text/plain",
            &[],
            b"undecided incoming body",
            &[0u8; 64],
            now + 2,
            102,
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // Set key version to 1 to force rebuild on next open
    {
        let conn = s.conn().lock().unwrap();
        conn.execute("UPDATE text_index_version SET key_version = 1 WHERE id = 1", []).unwrap();
    }
    drop(s);

    let reopened = open_at(&dir, ConversationConfig::default());
    let conn = reopened.conn().lock().unwrap();
    let rowids: Vec<i64> = conn
        .prepare("SELECT rowid FROM messages_tri ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // Only outgoing (rowid 1) and accepted incoming (rowid 2) must be indexed.
    assert_eq!(rowids.len(), 2);
}

#[test]
#[ignore = "Performance measurement for pull request text"]
fn measure_rebuild_of_10000_messages_on_open() {
    let config = ConversationConfig {
        max_pending_per_conversation: 20_000,
        max_messages_per_conversation: 20_000,
        ..ConversationConfig::default()
    };
    let (s, dir) = store_with_config(config);
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    for i in 0..10_000 {
        send_msg(&s, &conv, &format!("m:{i}"), format!("message {i} नल वाला plumber").as_bytes());
    }
    {
        let conn = s.conn().lock().unwrap();
        conn.execute("UPDATE text_index_version SET key_version = 1 WHERE id = 1", []).unwrap();
    }
    drop(s);

    let start = Instant::now();
    let reopened = open_at(&dir, ConversationConfig::default());
    let elapsed = start.elapsed();
    assert_eq!(reopened.search("plumber", None, 500).unwrap().len(), 500);
    println!("rebuild of 10000 messages on open took: {elapsed:?}");
}
