use std::{sync::Arc, time::Duration};

use syneroym_core::dht_registry::RegistryClient;
use syneroym_mailbox_format::Receipt;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    record::CheckedRecord,
    store::MailboxStore,
    test_support::{Member, record, sealed_deposit, small_limits},
    worker::MailboxWorker,
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[tokio::test]
async fn the_worker_sweeps_on_its_tick_and_stops_when_cancelled() {
    let store = Arc::new(MailboxStore::open_in_memory(small_limits()).unwrap());
    let registry = Arc::new(RegistryClient::new(false, None));
    let wake = Arc::new(Notify::new());

    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = syneroym_passport::now_secs();

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 1, now).deposit;
    let dep_id = dep.deposit_id;
    store.deposit(&sender.service_id(), &dep, now).unwrap();

    let worker =
        MailboxWorker::new(store.clone(), registry, wake, Duration::from_millis(500), 3600);

    let cancel = CancellationToken::new();
    let cancel_worker = cancel.clone();
    let handle = tokio::spawn(worker.run(cancel_worker));

    let mut expired = false;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let query_now = syneroym_passport::now_secs();
        let st = store.status(&sender.service_id(), &[dep_id], query_now).unwrap();
        if matches!(st[0].receipt, Receipt::Expired { .. }) {
            expired = true;
            break;
        }
    }
    assert!(expired, "expected worker sweep to expire the message");

    cancel.cancel();
    let join_res = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(join_res.is_ok(), "worker did not stop cleanly on cancellation");
}
