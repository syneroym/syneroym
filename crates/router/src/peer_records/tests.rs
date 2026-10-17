use std::time::{Duration, SystemTime, UNIX_EPOCH};

use syneroym_core::dht_registry::{EndpointInfo, EndpointMailbox};
use syneroym_identity::{Identity, substrate};
use syneroym_mailbox_format::MailboxPublicKey;

use super::*;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn info(master: &Identity, mailbox: bool, not_after: u64) -> EndpointInfo {
    let service_id = substrate::derive_did_key(&master.public_key());
    EndpointInfo {
        service_id: service_id.clone(),
        substrate_id: "did:key:zSubstrate".to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after,
        generation: 0,
        mailbox: mailbox
            .then(|| EndpointMailbox { service_id, key: MailboxPublicKey::from_bytes([9; 32]) }),
    }
}

fn record(master: &Identity, mailbox: bool) -> SignedEndpointInfo {
    info(master, mailbox, now_secs() + 3600).sign(master).unwrap()
}

/// The pkarr timestamp has microsecond resolution; a short pause makes the
/// next record certainly newer.
async fn newer_record(master: &Identity, mailbox: bool) -> SignedEndpointInfo {
    tokio::time::sleep(Duration::from_millis(5)).await;
    record(master, mailbox)
}

#[tokio::test]
async fn a_record_with_a_mailbox_is_recalled() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let master = Identity::generate().unwrap();
    let signed = record(&master, true);

    store.remember(&signed).await.unwrap();

    let recalled = store.recall(&signed.info.service_id).await.unwrap();
    assert_eq!(recalled.info.mailbox, signed.info.mailbox);
    assert!(recalled.verify().is_ok());
}

#[tokio::test]
async fn a_record_without_a_mailbox_is_not_stored() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let signed = record(&Identity::generate().unwrap(), false);

    store.remember(&signed).await.unwrap();

    assert!(store.recall(&signed.info.service_id).await.is_none());
}

#[tokio::test]
async fn a_substrate_record_is_ignored() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let master = Identity::generate().unwrap();
    let mut substrate_info = info(&master, true, now_secs() + 3600);
    substrate_info.endpoint_type = EndpointType::Substrate;
    let signed = substrate_info.sign(&master).unwrap();

    store.remember(&signed).await.unwrap();

    assert!(store.recall(&signed.info.service_id).await.is_none());
}

#[tokio::test]
async fn an_older_record_does_not_replace_a_newer_one() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let master = Identity::generate().unwrap();
    let older = record(&master, true);
    let mut newer = newer_record(&master, true).await;
    newer.info.nickname = None;

    store.remember(&newer).await.unwrap();
    store.remember(&older).await.unwrap();

    let recalled = store.recall(&newer.info.service_id).await.unwrap();
    assert_eq!(recalled.pkarr_packet_hex, newer.pkarr_packet_hex);
}

#[tokio::test]
async fn a_newer_record_without_a_mailbox_deletes_the_stored_one() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let master = Identity::generate().unwrap();
    let with = record(&master, true);
    store.remember(&with).await.unwrap();

    let without = newer_record(&master, false).await;
    store.remember(&without).await.unwrap();

    assert!(store.recall(&with.info.service_id).await.is_none());
}

#[tokio::test]
async fn an_older_record_without_a_mailbox_does_not_delete_a_newer_one() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let master = Identity::generate().unwrap();
    let without = record(&master, false);
    let with = newer_record(&master, true).await;
    store.remember(&with).await.unwrap();

    store.remember(&without).await.unwrap();

    assert!(store.recall(&with.info.service_id).await.is_some());
}

#[tokio::test]
async fn a_record_that_does_not_verify_is_not_stored() {
    let store = PeerRecordStore::open_in_memory().unwrap();
    let mut signed = record(&Identity::generate().unwrap(), true);
    signed.info.nickname = Some("changed after signing".to_string());

    assert!(store.remember(&signed).await.is_err());
    assert!(store.recall(&signed.info.service_id).await.is_none());
}

#[tokio::test]
async fn an_expired_record_is_not_recalled_and_its_row_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peer_records.db");
    let master = Identity::generate().unwrap();
    // Valid when stored, `not_after` one second from now.
    let signed = info(&master, true, now_secs() + 1).sign(&master).unwrap();
    let store = PeerRecordStore::open(&path, now_secs()).unwrap();
    store.remember(&signed).await.unwrap();
    assert!(store.recall(&signed.info.service_id).await.is_some());

    tokio::time::sleep(Duration::from_millis(2100)).await;

    assert!(store.recall(&signed.info.service_id).await.is_none());
    let rows: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM peer_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn the_store_survives_reopening_the_file_and_drops_expired_rows_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("peer_records.db");
    let live_master = Identity::generate().unwrap();
    let live = record(&live_master, true);
    let stale_master = Identity::generate().unwrap();
    let stale = info(&stale_master, true, now_secs() + 1).sign(&stale_master).unwrap();
    {
        let store = PeerRecordStore::open(&path, now_secs()).unwrap();
        store.remember(&live).await.unwrap();
        store.remember(&stale).await.unwrap();
    }

    let reopened = PeerRecordStore::open(&path, now_secs() + 10).unwrap();

    assert!(reopened.recall(&live.info.service_id).await.is_some());
    assert!(reopened.recall(&stale.info.service_id).await.is_none());
}
