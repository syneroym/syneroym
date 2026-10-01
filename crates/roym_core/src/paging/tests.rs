use serde::Deserialize;
use serde_json::json;
use syneroym_app_host::types::data_layer::QueryResult;
use syneroym_signed_record::RecordDraft;

use super::*;
use crate::signing::tests::TestHost;

pub(crate) fn row(id: &str, payload: &[u8]) -> RecordReadValue {
    RecordReadValue {
        id: id.to_string(),
        payload: payload.to_vec(),
        creator_id: String::new(),
        created_at: 0,
        updated_at: 0,
    }
}

pub(crate) fn page(records: Vec<RecordReadValue>, next_cursor: Option<&str>) -> QueryResult {
    QueryResult { records, next_cursor: next_cursor.map(str::to_string) }
}

fn host_with(pages: Vec<QueryResult>) -> TestHost {
    let host = TestHost::default();
    for p in pages {
        host.push_query_page(p);
    }
    host
}

#[derive(Debug, PartialEq, Deserialize)]
struct Item {
    n: u32,
}

/// One good row, one that is not JSON, one that is JSON but not an
/// `Item`, and a second good row.
fn mixed_page() -> QueryResult {
    page(
        vec![
            row("a", br#"{"n":1}"#),
            row("b", b"not json"),
            row("c", br#"{"m":2}"#),
            row("d", br#"{"n":4}"#),
        ],
        None,
    )
}

#[tokio::test]
async fn walks_every_page_and_hands_each_cursor_back() {
    let host = host_with(vec![
        page(vec![row("a", br#"{"n":1}"#)], Some("c1")),
        page(vec![row("b", br#"{"n":2}"#)], Some("c2")),
        page(vec![row("c", br#"{"n":3}"#)], None),
    ]);
    let filter = Some(r#"{"k":1}"#.to_string());

    let items: Vec<Item> = query_all(&host, "things", filter.clone()).await.unwrap();

    assert_eq!(items, [Item { n: 1 }, Item { n: 2 }, Item { n: 3 }]);
    let queries = host.queries();
    let cursors: Vec<Option<String>> = queries.iter().map(|(_, o)| o.cursor.clone()).collect();
    assert_eq!(cursors, [None, Some("c1".to_string()), Some("c2".to_string())]);
    for (collection, opts) in &queries {
        assert_eq!(collection, "things");
        assert_eq!(opts.limit, Some(PAGE_SIZE));
        assert_eq!(opts.filter, filter);
    }
}

#[tokio::test]
async fn stops_when_the_host_repeats_the_cursor_it_was_given() {
    let host = host_with(vec![
        page(vec![row("a", b"{}")], Some("same")),
        page(vec![row("b", b"{}")], Some("same")),
        page(vec![row("never-read", b"{}")], None),
    ]);

    assert_eq!(count(&host, "things", None).await.unwrap(), 2);
    assert_eq!(host.queries().len(), 2);
}

#[tokio::test]
async fn pages_end_with_none_and_do_not_query_again() {
    let host = host_with(vec![
        page(vec![row("a", b"{}"), row("b", b"{}")], Some("c1")),
        page(vec![row("c", b"{}")], None),
    ]);
    let mut pages = Pages::new(&host, "things", None);

    assert_eq!(pages.next_page().await.unwrap().map(|rows| rows.len()), Some(2));
    assert_eq!(pages.next_page().await.unwrap().map(|rows| rows.len()), Some(1));
    assert!(pages.next_page().await.unwrap().is_none());
    assert!(pages.next_page().await.unwrap().is_none());
    assert_eq!(host.queries().len(), 2);
}

#[tokio::test]
async fn query_all_skips_rows_that_do_not_decode() {
    let host = host_with(vec![mixed_page()]);
    let items: Vec<Item> = query_all(&host, "things", None).await.unwrap();
    assert_eq!(items, [Item { n: 1 }, Item { n: 4 }]);
}

#[tokio::test]
async fn query_all_with_ids_keeps_each_row_id() {
    let host = host_with(vec![mixed_page()]);
    let rows: Vec<(String, Value)> = query_all_with_ids(&host, "things", None).await.unwrap();
    let ids: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["a", "c", "d"]);
    assert_eq!(rows[1].1, json!({ "m": 2 }));
}

#[tokio::test]
async fn count_includes_rows_that_do_not_decode() {
    let host = host_with(vec![mixed_page()]);
    assert_eq!(count(&host, "things", None).await.unwrap(), 4);
}

fn envelope(listing_id: &str, issued_at_secs: u64) -> Vec<u8> {
    let draft = RecordDraft {
        version: 1,
        record_type: "listing".to_string(),
        subject: "did:key:zSubject".to_string(),
        payload: json!({ "listing_id": listing_id }),
        expires_at_secs: None,
        supersedes: None,
    };
    let (env, _) =
        Envelope::unsigned(draft, "did:key:zIssuer".to_string(), None, issued_at_secs).unwrap();
    env.to_json().unwrap().into_bytes()
}

#[tokio::test]
async fn envelope_history_keeps_matches_oldest_first() {
    let newest = envelope("L1", 300);
    let oldest = envelope("L1", 100);
    let host = host_with(vec![page(
        vec![
            row("r3", &newest),
            row("x", &envelope("L2", 50)),
            row("y", b"not an envelope"),
            row("r1", &oldest),
        ],
        None,
    )]);

    let history = envelope_history(&host, "history", "listing_id", "L1").await.unwrap();

    let expected: Vec<String> =
        [oldest, newest].into_iter().map(|b| String::from_utf8(b).unwrap()).collect();
    assert_eq!(history, expected);
    let queries = host.queries();
    assert_eq!(queries[0].1.filter.as_deref(), Some(r#"{"payload.listing_id":"L1"}"#));
}
