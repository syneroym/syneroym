use serde_json::json;

use super::*;
use crate::{
    MAILBOX_USE, RELAY_HOME,
    test_support::{limits, new_did},
};

const NOW: u64 = 1_000_000;
const DB: &str = "leases.db";

fn id(n: u8) -> PassportId {
    hex::encode([n; 32]).parse().unwrap()
}

fn lease(passport: u8, subject: &str, registered_at: u64, lease_ends_at: u64) -> Lease {
    Lease {
        passport_id: id(passport),
        owner_did: "did:key:howner".to_string(),
        subject: subject.to_string(),
        lease_ends_at,
        registered_at,
        limits: Map::new(),
    }
}

#[test]
fn leases_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let subject = new_did();
    let mut granted = lease(1, &subject, NOW, NOW + 100);
    granted.limits = limits(&[("max_bytes", json!(5))]);
    {
        let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
        store.grant(granted.clone(), None, NOW).unwrap();
    }
    let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW + 1).unwrap();
    assert!(store.is_admitted(&subject, NOW + 1));
    assert_eq!(store.lease_for(&subject, NOW + 1), Some(granted));
}

#[test]
fn lease_for_picks_the_newest_registration() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let s = new_did();
    store.grant(lease(1, &s, NOW, NOW + 1_000), None, NOW).unwrap();
    store.grant(lease(2, &s, NOW + 10, NOW + 500), None, NOW + 10).unwrap();
    assert_eq!(store.lease_for(&s, NOW + 10).unwrap().passport_id, id(2));

    store.grant(lease(1, &s, NOW + 20, NOW + 1_000), None, NOW + 20).unwrap();
    assert_eq!(store.lease_for(&s, NOW + 20).unwrap().passport_id, id(1));

    store.deny(&id(1), NOW + 20).unwrap();
    assert_eq!(store.lease_for(&s, NOW + 20).unwrap().passport_id, id(2));

    let t = new_did();
    store.grant(lease(3, &t, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(4, &t, NOW, NOW + 200), None, NOW).unwrap();
    assert_eq!(store.lease_for(&t, NOW).unwrap().passport_id, id(4), "the later end wins a tie");

    let u = new_did();
    store.grant(lease(6, &u, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(5, &u, NOW, NOW + 100), None, NOW).unwrap();
    assert_eq!(store.lease_for(&u, NOW).unwrap().passport_id, id(5), "then the smaller id");
}

#[test]
fn lease_for_ignores_an_ended_lease() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let s = new_did();
    store.grant(lease(1, &s, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(2, &s, NOW + 10, NOW + 50), None, NOW + 10).unwrap();
    assert_eq!(store.lease_for(&s, NOW + 50).unwrap().passport_id, id(1));
    assert_eq!(store.lease_for(&s, NOW + 100), None);
}

#[test]
fn an_ended_lease_does_not_admit() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let s = new_did();
    store.grant(lease(1, &s, NOW, NOW + 100), None, NOW).unwrap();
    assert!(store.is_admitted(&s, NOW + 99));
    assert!(!store.is_admitted(&s, NOW + 100));
}

#[test]
fn ended_leases_are_deleted_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (new_did(), new_did());
    {
        let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
        store.grant(lease(1, &a, NOW, NOW + 10), None, NOW).unwrap();
        store.grant(lease(1, &b, NOW, NOW + 100), None, NOW).unwrap();
    }
    LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW + 10).unwrap();
    let conn = Connection::open(dir.path().join(DB)).unwrap();
    let rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM passport_leases", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn deny_ends_leases_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (new_did(), new_did());
    {
        let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
        store.grant(lease(1, &a, NOW, NOW + 100), None, NOW).unwrap();
        store.grant(lease(1, &b, NOW, NOW + 100), None, NOW).unwrap();
        assert_eq!(store.deny(&id(1), NOW).unwrap(), 2);
        assert!(!store.is_admitted(&a, NOW));
        assert!(!store.is_admitted(&b, NOW));
        assert!(store.is_denied(&id(1)));
    }
    let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
    assert!(store.is_denied(&id(1)));
    assert!(!store.is_admitted(&a, NOW));
    assert_eq!(store.grant(lease(1, &a, NOW, NOW + 100), None, NOW), Err(RegisterError::Denied));
}

#[test]
fn deny_in_advance_refuses_a_later_grant() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    assert_eq!(store.deny(&id(7), NOW).unwrap(), 0);
    assert_eq!(
        store.grant(lease(7, &new_did(), NOW, NOW + 100), None, NOW),
        Err(RegisterError::Denied)
    );
}

#[test]
fn the_limit_counts_distinct_running_subjects() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let (a, b, c) = (new_did(), new_did(), new_did());
    store.grant(lease(1, &a, NOW, NOW + 1_000), Some(2), NOW).unwrap();
    store.grant(lease(1, &b, NOW, NOW + 10), Some(2), NOW).unwrap();
    assert_eq!(
        store.grant(lease(1, &c, NOW, NOW + 1_000), Some(2), NOW),
        Err(RegisterError::LimitReached { limit: 2 })
    );
    store.grant(lease(1, &a, NOW + 1, NOW + 1_000), Some(2), NOW + 1).unwrap();
    store.grant(lease(1, &c, NOW + 10, NOW + 1_000), Some(2), NOW + 10).unwrap();
    assert_eq!(
        store.grant(lease(2, &b, NOW + 10, NOW + 1_000), Some(0), NOW + 10),
        Err(RegisterError::LimitReached { limit: 0 }),
        "a limit of 0 admits nobody"
    );
}

#[test]
fn a_renewal_replaces_the_lease_end() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let s = new_did();
    store.grant(lease(1, &s, NOW, NOW + 1_000), None, NOW).unwrap();
    store.grant(lease(1, &s, NOW + 5, NOW + 100), None, NOW + 5).unwrap();
    let current = store.lease_for(&s, NOW + 5).unwrap();
    assert_eq!((current.registered_at, current.lease_ends_at), (NOW + 5, NOW + 100));
    assert_eq!(store.running_leases(NOW + 5).len(), 1);
}

#[test]
fn running_leases_lists_only_running_ones_in_order() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let (a, b) = (new_did(), new_did());
    let (first, second) = if a < b { (a, b) } else { (b, a) };
    store.grant(lease(2, &second, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(2, &first, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(1, &second, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(3, &first, NOW, NOW + 10), None, NOW).unwrap();
    let running: Vec<(PassportId, String)> =
        store.running_leases(NOW + 10).into_iter().map(|l| (l.passport_id, l.subject)).collect();
    assert_eq!(running, vec![(id(1), second.clone()), (id(2), first), (id(2), second)]);
}

#[test]
fn a_subject_with_two_passports_stays_admitted_when_one_is_denied() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let s = new_did();
    store.grant(lease(1, &s, NOW, NOW + 100), None, NOW).unwrap();
    store.grant(lease(2, &s, NOW, NOW + 100), None, NOW).unwrap();
    store.deny(&id(1), NOW).unwrap();
    assert!(store.is_admitted(&s, NOW));
    assert_eq!(store.lease_for(&s, NOW).unwrap().passport_id, id(2));
}

#[test]
fn open_refuses_a_path_in_the_db_name() {
    let dir = tempfile::tempdir().unwrap();
    for bad in ["a/b.db", "a\\b.db", "../b.db"] {
        assert!(
            LeaseStore::open(dir.path(), bad, &RELAY_HOME, NOW).is_err(),
            "{bad} must be refused"
        );
    }
}

#[test]
fn a_file_made_for_one_use_is_refused_for_another() {
    let dir = tempfile::tempdir().unwrap();
    LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
    let error = LeaseStore::open(dir.path(), DB, &MAILBOX_USE, NOW).unwrap_err();
    assert!(error.to_string().contains("relay/home"), "{error}");
    let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
    assert_eq!(store.ability(), RELAY_HOME.ability);
}

/// The limit is atomic because `grant` holds the connection from the count to
/// the write. Threads that grant at once must not pass the limit together.
#[test]
fn concurrent_grants_never_pass_the_limit() {
    const LIMIT: u64 = 3;
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (store, subject) = (store.clone(), new_did());
                scope.spawn(move || {
                    store.grant(lease(1, &subject, NOW, NOW + 100), Some(LIMIT), NOW)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 3);
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| *e == RegisterError::LimitReached { limit: LIMIT })
    );
    assert_eq!(store.running_leases(NOW).len(), 3);
}

/// Grants and a deny of one passport run at once. Whatever the order, no
/// lease of the denied passport is left, in memory or in the file.
#[test]
fn a_deny_racing_grants_leaves_no_lease_behind() {
    let dir = tempfile::tempdir().unwrap();
    let store = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..16 {
            let (store, subject) = (store.clone(), new_did());
            scope.spawn(move || {
                let _ = store.grant(lease(1, &subject, NOW, NOW + 100), None, NOW);
            });
        }
        scope.spawn(|| {
            store.deny(&id(1), NOW).unwrap();
        });
    });
    assert!(store.is_denied(&id(1)));
    assert!(store.running_leases(NOW).is_empty());
    drop(store);
    let reopened = LeaseStore::open(dir.path(), DB, &RELAY_HOME, NOW).unwrap();
    assert!(reopened.is_denied(&id(1)));
    assert!(reopened.running_leases(NOW).is_empty());
}

#[test]
fn a_failed_write_is_a_store_error_and_leaves_memory_unchanged() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let (kept, lost) = (new_did(), new_did());
    store.grant(lease(1, &kept, NOW, NOW + 100), None, NOW).unwrap();
    store.conn().execute_batch("DROP TABLE passport_leases").unwrap();

    let renewal = lease(1, &kept, NOW + 5, NOW + 500);
    assert!(matches!(store.grant(renewal, None, NOW + 5), Err(RegisterError::Store(_))));
    assert!(matches!(
        store.grant(lease(1, &lost, NOW + 5, NOW + 500), None, NOW + 5),
        Err(RegisterError::Store(_))
    ));
    assert!(!store.is_admitted(&lost, NOW + 5));
    let current = store.lease_for(&kept, NOW + 5).unwrap();
    assert_eq!((current.registered_at, current.lease_ends_at), (NOW, NOW + 100));
}

#[test]
fn a_failed_deny_leaves_memory_unchanged() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let s = new_did();
    store.grant(lease(1, &s, NOW, NOW + 100), None, NOW).unwrap();
    store.conn().execute_batch("DROP TABLE passport_denied").unwrap();

    assert!(store.deny(&id(1), NOW).is_err());
    assert!(!store.is_denied(&id(1)));
    assert!(store.is_admitted(&s, NOW));
}
