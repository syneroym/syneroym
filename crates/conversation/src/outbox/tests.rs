use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use syneroym_rpc::{
    ConversationDeliveryState, ConversationMessage, ConversationNotifier, NotifyOutcome,
};

use super::*;
use crate::{store::now_ms, transport::tests::test_service};

const SVC: &str = "svc:receiver";
const PEER: &str = "did:key:zPeer";

#[test]
fn backoff_curve_grows_with_age_and_caps() {
    assert_eq!(backoff_for_age(0), 1_000);
    assert_eq!(backoff_for_age(500), 1_000);
    assert_eq!(backoff_for_age(1_500), 2_000);
    assert_eq!(backoff_for_age(4_000), 4_000);
    assert_eq!(backoff_for_age(10_000), 8_000);
    assert_eq!(backoff_for_age(20_000), 16_000);
    assert_eq!(backoff_for_age(100_000), 64_000);
    assert_eq!(backoff_for_age(300_000), 256_000);
    assert_eq!(backoff_for_age(600_000), 300_000);
    assert_eq!(backoff_for_age(1_000_000), 300_000);
}

#[derive(Debug)]
struct Fixed {
    asked: AtomicUsize,
    outcome: fn() -> NotifyOutcome,
}

impl Fixed {
    fn new(outcome: fn() -> NotifyOutcome) -> Arc<Self> {
        Arc::new(Self { asked: AtomicUsize::new(0), outcome })
    }
}

#[async_trait]
impl ConversationNotifier for Fixed {
    async fn notify_message(&self, _: &str, _: ConversationMessage) -> NotifyOutcome {
        self.asked.fetch_add(1, Ordering::SeqCst);
        (self.outcome)()
    }

    async fn notify_delivery_state(&self, _: &str, _: String, _: ConversationDeliveryState) {}
}

async fn service_with(
    dir: &std::path::Path,
    outcome: fn() -> NotifyOutcome,
) -> (Arc<ConversationService>, Arc<Fixed>, Arc<ConversationStore>) {
    let service = test_service(dir).await;
    let notifier = Fixed::new(outcome);
    service.register_service_notifier(
        SVC.to_string(),
        Arc::downgrade(&notifier) as std::sync::Weak<dyn ConversationNotifier>,
    );
    let store = service.store_for(SVC).await.unwrap();
    (service, notifier, store)
}

/// An undecided incoming row that is already due for a re-ask.
fn due_undecided(store: &ConversationStore, id: &str, received_at: i64) {
    let conv = store.get_or_create_direct(PEER, "conv:1", received_at).unwrap();
    let conn = store.conn().lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    store
        .insert_incoming_if_absent(
            &tx,
            &conv,
            id,
            PEER,
            received_at,
            "text/plain",
            b"hello",
            &[0u8; 64],
            received_at,
            100,
        )
        .unwrap();
    tx.commit().unwrap();
    drop(conn);
    store.update_undecided_retry(id, 0, 0).unwrap();
}

#[tokio::test]
async fn an_unanswered_ask_is_counted_and_not_repeated_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (service, notifier, store) = service_with(dir.path(), || NotifyOutcome::NoAnswer).await;
    due_undecided(&store, "m:1", now_ms());

    service.renotify_undecided_once().await;
    service.renotify_undecided_once().await;

    assert_eq!(notifier.asked.load(Ordering::SeqCst), 1, "the second tick must wait");
    let row = store.get_message("m:1").unwrap().unwrap();
    assert_eq!(row.admission, "undecided");
    assert_eq!(row.notify_attempts, 1);
    assert!(row.next_notify_at.is_some_and(|t| t > now_ms()));
}

#[tokio::test]
async fn an_answered_ask_settles_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _notifier, store) =
        service_with(dir.path(), || NotifyOutcome::Answered(syneroym_rpc::Admission::Accept)).await;
    due_undecided(&store, "m:1", now_ms());

    service.renotify_undecided_once().await;

    let row = store.get_message("m:1").unwrap().unwrap();
    assert_eq!(row.admission, "accepted");
    assert!(row.visible_seq > 0);
}

#[tokio::test]
async fn rows_the_host_wrote_itself_are_never_put_to_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let (service, notifier, store) = service_with(dir.path(), || NotifyOutcome::NoAnswer).await;
    due_undecided(&store, "m:sys", now_ms());
    store.conn().lock().unwrap().execute("UPDATE messages SET system = 1", []).unwrap();

    service.renotify_undecided_once().await;

    assert_eq!(notifier.asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_old_held_message_expires_and_its_text_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _notifier, store) = service_with(dir.path(), || NotifyOutcome::NoAnswer).await;
    // Received at the start of the epoch: far older than the 30-day limit.
    due_undecided(&store, "m:old", 1_000);
    store
        .apply_admission("m:old", &syneroym_rpc::Admission::Hold("group-hidden".into()), 1_000)
        .unwrap();
    due_undecided(&store, "m:new", now_ms());
    store
        .apply_admission("m:new", &syneroym_rpc::Admission::Hold("group-hidden".into()), now_ms())
        .unwrap();

    service.expire_held_once().await;

    let old = store.get_message("m:old").unwrap().unwrap();
    assert_eq!(old.admission, "dropped");
    assert_eq!(old.admission_reason.as_deref(), Some("expired"));
    assert!(old.body.is_empty());
    assert_eq!(store.get_message("m:new").unwrap().unwrap().admission, "held");
}

#[tokio::test]
async fn a_delete_is_followed_by_a_scrub_on_the_next_tick() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _notifier, store) =
        service_with(dir.path(), || NotifyOutcome::Answered(syneroym_rpc::Admission::Accept)).await;
    due_undecided(&store, "m:1", now_ms());
    service.renotify_undecided_once().await;
    store.delete_message("conv:1", "m:1", now_ms()).unwrap();

    service.wal_checkpoint_once().await;

    assert!(!store.take_wal_checkpoint_flag(), "a finished scrub leaves nothing to retry");
}
