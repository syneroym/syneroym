use std::fs;

use tempfile::tempdir;

use crate::{
    record::CheckedRecord,
    store::MailboxStore,
    test_support::{Member, record, sealed_deposit, small_limits},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn the_store_holds_no_message_content_in_clear() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("mailbox.db");

    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    {
        let store = MailboxStore::open(&db_path, small_limits()).unwrap();
        let rec = record(&receiver, NODE_DID);
        let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
        store.register(&receiver.instance(), &checked, now).unwrap();

        let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
        store.deposit(&sender.service_id(), &dep, now).unwrap();

        let conn = store.lock();
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
    }

    for entry in fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        let bytes = fs::read(entry.path()).unwrap();
        let content_str = String::from_utf8_lossy(&bytes);

        assert!(
            !content_str.contains("marker-interface"),
            "database file contained unencrypted interface marker"
        );
        assert!(
            !content_str.contains("marker-method"),
            "database file contained unencrypted method marker"
        );
        assert!(!content_str.contains("test"), "database file contained unencrypted param key");
    }
}
