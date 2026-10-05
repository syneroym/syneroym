//! Deleted text must leave the database files, not only the visible tables.

use std::{fs, path::Path};

use rusqlite::Connection;
use syneroym_rpc::{Admission, DropAnswer};

use super::{store, store_in_dir, store_with_config};
use crate::store::{ConversationConfig, ConversationStore};

/// Unique enough to appear nowhere else: the word and one of its trigrams.
const WORD: &[u8] = b"zqxjvk";
const TRIGRAM: &[u8] = b"qxj";

fn files_contain(dir: &Path, needle: &[u8]) -> bool {
    fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().is_file()).any(|e| {
        let bytes = fs::read(e.path()).unwrap();
        bytes.windows(needle.len()).any(|w| w == needle)
    })
}

fn accepted_message(s: &ConversationStore, conv: &str, id: &str, body: &[u8]) {
    let conn = s.conn().lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    s.insert_incoming_if_absent(
        &tx,
        conv,
        id,
        "did:key:zPeer",
        1_000,
        "text/plain",
        body,
        &[0u8; 64],
        1_000,
        100,
    )
    .unwrap();
    tx.commit().unwrap();
    drop(conn);
    s.apply_admission(id, &Admission::Accept, 1_000).unwrap();
}

#[test]
fn deleted_text_is_gone_from_the_files_after_a_scrub() {
    let (s, dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    accepted_message(&s, &conv, "m:keep", b"an ordinary message that stays");
    accepted_message(&s, &conv, "m:gone", b"a note about zqxjvk to remove");
    assert!(files_contain(&dir, TRIGRAM), "the test needs the text to be on disk first");

    s.delete_message(&conv, "m:gone", 2_000).unwrap();
    assert!(!s.scrub_and_checkpoint(), "nothing holds the log, so one pass finishes");

    assert!(!files_contain(&dir, WORD), "the body must be gone");
    assert!(!files_contain(&dir, TRIGRAM), "the search index must not keep its pieces");
}

#[test]
fn dropped_text_is_gone_from_the_files_after_a_scrub() {
    let (s, dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    accepted_message(&s, &conv, "m:keep", b"an ordinary message that stays");
    accepted_message(&s, &conv, "m:gone", b"a note about zqxjvk to remove");

    let drop = Admission::Drop(DropAnswer { reason: "blocked".into(), report: false });
    s.apply_admission("m:gone", &drop, 2_000).unwrap();
    assert!(!s.scrub_and_checkpoint());

    assert!(!files_contain(&dir, WORD));
    assert!(!files_contain(&dir, TRIGRAM));
}

#[test]
fn a_scrub_asks_for_a_retry_while_a_reader_holds_the_log() {
    let (s, dir) = store_in_dir();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    accepted_message(&s, &conv, "m:1", b"a message with some text");

    // A reader that started before the scrub keeps the log from truncating.
    let reader = Connection::open(dir.join("conversation.db")).unwrap();
    reader.execute_batch("BEGIN; SELECT COUNT(*) FROM messages;").unwrap();
    accepted_message(&s, &conv, "m:2", b"written after the reader began");

    assert!(s.scrub_and_checkpoint(), "a blocked checkpoint must be reported as unfinished");

    reader.execute_batch("COMMIT;").unwrap();
    assert!(!s.scrub_and_checkpoint(), "the next pass finishes once the reader is gone");
}

#[test]
fn dropping_a_row_that_was_never_indexed_keeps_the_index_sound() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    accepted_message(&s, &conv, "m:visible", b"indexed text");
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:held",
            "did:key:zPeer",
            1_001,
            "text/plain",
            b"never indexed",
            &[0u8; 64],
            1_001,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission("m:held", &Admission::Hold("group-hidden".into()), 1_001).unwrap();

    let drop = Admission::Drop(DropAnswer { reason: "expired".into(), report: false });
    s.apply_admission("m:held", &drop, 2_000).unwrap();

    // A 'delete' for text that was never indexed leaves bad entries that
    // surface when the index segments are merged.
    assert!(!s.scrub_and_checkpoint(), "merging the index must not fail");
    let found = s.search("indexed text", None, 10).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "m:visible");
}

#[test]
fn a_burst_of_deletes_is_spread_over_the_scrub_interval() {
    let (s, _dir) = store_with_config(ConversationConfig {
        scrub_min_interval_secs: 3_600,
        ..ConversationConfig::default()
    });
    assert!(s.scrub_due(), "the first scrub after a quiet time is not delayed");

    s.mark_scrubbed();

    assert!(!s.scrub_due(), "a second pass inside the interval waits");
}

#[test]
fn a_zero_interval_never_delays_a_scrub() {
    let (s, _dir) = store_with_config(ConversationConfig {
        scrub_min_interval_secs: 0,
        ..ConversationConfig::default()
    });
    s.mark_scrubbed();

    assert!(s.scrub_due());
}

#[test]
fn a_freshly_opened_store_asks_for_one_scrub() {
    // A delete just before a restart left text in the index; the flag that
    // would have scrubbed it lived only in memory.
    let s = store();

    assert!(s.take_wal_checkpoint_flag(), "the first tick after opening scrubs once");
    assert!(!s.scrub_and_checkpoint(), "with nothing to merge the pass finishes");
}
