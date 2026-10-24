use serde_json::json;

use crate::curated_list::{CuratedList, ListEntry, MailboxEntry, check_entries, rank};

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";
const DID_C: &str = "did:key:h7wy7c8txi9eay56m9m53wco4nrstgt6hxbxp1snrdu5c8r69nre6ara";

fn entry(service_id: &str) -> MailboxEntry {
    MailboxEntry { service_id: service_id.to_string(), operator: "op".to_string() }
}

#[test]
fn a_mailbox_entry_takes_a_node_did_and_writes_its_canonical_form() {
    let mut e = entry(DID_A);

    let id = e.check().expect("a node DID is accepted");

    assert_eq!(id, DID_A);
    assert_eq!(e.service_id, DID_A);
}

#[test]
fn a_mailbox_entry_that_is_not_a_node_did_is_refused() {
    for bad in ["", "did:web:example.com", "did:key:not-z32"] {
        assert!(entry(bad).check().is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn two_mailbox_entries_with_one_did_are_a_duplicate() {
    let mut entries = vec![entry(DID_A), entry(DID_B), entry(DID_A)];

    let err = check_entries(&mut entries).unwrap_err().to_string();

    assert!(err.contains("roles.community_registry.mailboxes[2]"), "{err}");
    assert!(err.contains("duplicate"), "{err}");
}

#[test]
fn a_mailbox_list_uses_its_own_json_key() {
    let list = CuratedList { max_age_secs: 60, entries: vec![entry(DID_A)] };

    let value = serde_json::to_value(&list).unwrap();

    assert_eq!(
        value,
        json!({ "max_age_secs": 60, "mailboxes": [{ "service_id": DID_A, "operator": "op" }] })
    );
    let back: CuratedList<MailboxEntry> = serde_json::from_value(value).unwrap();
    assert_eq!(back, list);
}

#[test]
fn the_mailbox_ranking_is_fixed() {
    let ranked = rank("inst-1/chat", vec![entry(DID_A), entry(DID_B), entry(DID_C)]);

    let ids: Vec<_> = ranked.iter().map(|e| e.service_id.as_str()).collect();

    assert_eq!(ids, [DID_A, DID_C, DID_B]);
}

#[test]
fn the_mailbox_list_takes_no_filter() {
    assert!(MailboxEntry::filter_pairs(&Default::default()).is_empty());
    assert!(entry(DID_A).matches(&Default::default()));
}
