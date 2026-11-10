use std::{
    fmt,
    path::Path,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use serde_json::Value;
use syneroym_core::event_bus::{Cursor, Event, EventBus, ReadOutcome};
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_rpc::{
    Admission, ConversationDeliveryState, ConversationHistoryItem, ConversationHost,
    ConversationMessage, ConversationNotifier, DropAnswer, NotifyOutcome, ProxyError,
};

use crate::{
    dag::{
        self, DELETION_REQUEST_CONTENT_TYPE, GroupPushRequest, MembershipPayload,
        REFUSAL_NOTICE_CONTENT_TYPE, WireEntry, encode_body,
    },
    group::build_message_entry,
    host_impl::tests::ME,
    store::{ConversationRow, ConversationStore, now_ms},
    transport::tests::test_service,
};

const PEER: &str = "svc:peer";
const CONV_GROUP: &str = "conv:group1";
const GROUP_KEY: [u8; 32] = [42u8; 32];

// The probe rule for conversation tests:
// Call the service one step at a time and await each call before the next one;
// never spawn a call, and never run two calls at once with join! or select!.
// Then no blocking job is running when announce runs, and a failed try_lock can
// only mean that the caller itself holds the lock. The probe does not retry
// try_lock.

async fn event_service(dir: &Path) -> (Arc<crate::ConversationService>, Arc<EventBus>) {
    let service = test_service(dir).await;
    let bus = EventBus::new(service.registry().clone());
    service.set_event_bus(bus.clone());
    service
        .registry()
        .set_app_context(ME.to_string(), "app-1".to_string(), "conversation".to_string())
        .await
        .unwrap();
    (service, bus)
}

fn read_events(bus: &EventBus, app_instance: &str) -> Vec<Event> {
    let start = Cursor { run_id: bus.run_id().to_string(), seq: 0 };
    match bus.read(app_instance, Some(&start), 100, now_ms()) {
        ReadOutcome::Events { events, .. } => events,
        ReadOutcome::Reload { .. } => Vec::new(),
    }
}

fn test_message(
    id: &str,
    conv: &str,
    author: &str,
    content_type: &str,
    body: &[u8],
) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        conversation: conv.to_string(),
        author: author.to_string(),
        sender_timestamp: 1000,
        received_at: 1000,
        content_type: content_type.to_string(),
        body: body.to_vec(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
        outgoing: false,
        deleted_at: None,
        restored: false,
        visible_seq: 0,
        refused: None,
    }
}

struct FixedNotifier {
    outcome: Box<dyn Fn() -> NotifyOutcome + Send + Sync>,
}

impl fmt::Debug for FixedNotifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FixedNotifier")
    }
}

impl FixedNotifier {
    fn new<F>(outcome: F) -> Arc<Self>
    where
        F: Fn() -> NotifyOutcome + Send + Sync + 'static,
    {
        Arc::new(Self { outcome: Box::new(outcome) })
    }
}

#[async_trait]
impl ConversationNotifier for FixedNotifier {
    async fn notify_message(&self, _: &str, _: ConversationMessage) -> NotifyOutcome {
        (self.outcome)()
    }

    async fn notify_delivery_state(&self, _: &str, _: String, _: ConversationDeliveryState) {}
}

fn group_with(s: &ConversationStore, members: &[(&str, &SigningKey)]) -> ConversationRow {
    let conv = group_without_key(s, members);
    s.conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 1, \
             ?2, 1000)",
            rusqlite::params![CONV_GROUP, GROUP_KEY.as_slice()],
        )
        .unwrap();
    conv
}

fn group_without_key(s: &ConversationStore, members: &[(&str, &SigningKey)]) -> ConversationRow {
    let conn = s.conn().lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let conv = ConversationStore::get_or_create_group_shell(&tx, CONV_GROUP, "svc:owner", 1, 1_000)
        .unwrap();
    for (address, sk) in members {
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: (*address).to_string(),
            subject_sig_key: sk.verifying_key().to_bytes(),
            new_epoch: 1,
            member_list_hash: "hash".to_string(),
        };
        ConversationStore::apply_membership(&tx, CONV_GROUP, &payload).unwrap();
    }
    tx.commit().unwrap();
    conv
}

fn make_entry(
    sk: &SigningKey,
    author: &str,
    at: i64,
    content_type: &str,
    body: &[u8],
) -> WireEntry {
    build_message_entry(
        sk,
        CONV_GROUP,
        author,
        at,
        1,
        vec![],
        &GROUP_KEY,
        &encode_body(content_type, &[], body),
    )
    .unwrap()
}

fn push_request(alice: &SigningKey, conv_id: &str, entries: Vec<WireEntry>) -> GroupPushRequest {
    let assertion = dag::sign_peer_assertion(alice, "svc:alice", conv_id, now_ms(), &[0u8; 16]);
    GroupPushRequest { from: assertion, group: conv_id.to_string(), entries }
}

fn register_accept_notifier(service: &crate::ConversationService) -> Arc<FixedNotifier> {
    let notifier = FixedNotifier::new(|| NotifyOutcome::Answered(Admission::Accept));
    service.register_service_notifier(
        ME.to_string(),
        Arc::downgrade(&notifier) as Weak<dyn ConversationNotifier>,
    );
    notifier
}

#[tokio::test]
async fn sending_a_message_records_one_message_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let store = service.store_for(ME).await.unwrap();

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, _conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.message" {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })));
    }

    let conv = service.open_direct(ME, PEER).await.unwrap();
    let msg_id = service.send(ME, &conv, "text/plain", b"hi".to_vec()).await.unwrap();

    assert_eq!(probe_called.load(Ordering::SeqCst), 1);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "conversation.message");
    assert_eq!(events[0].service, "conversation");
    assert_eq!(events[0].subject, conv);
    assert_eq!(events[0].item.as_deref(), Some(msg_id.as_str()));
}

#[tokio::test]
async fn sending_a_group_message_records_one_message_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let store = service.store_for(ME).await.unwrap();
    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let me = SigningKey::generate(&mut rand_core::OsRng);
    let conv = group_with(&store, &[(ME, &me), ("svc:alice", &alice)]);

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, _conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.message" {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })));
    }

    let msg_id =
        service.send_group(ME, &store, &conv, "text/plain", &[], b"group send text").await.unwrap();

    assert_eq!(probe_called.load(Ordering::SeqCst), 1);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "conversation.message");
    assert_eq!(events[0].subject, conv.id);
    assert_eq!(events[0].item.as_deref(), Some(msg_id.as_str()));
}

#[tokio::test]
async fn an_accepted_inbound_message_records_one_event_after_it_is_visible() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let _notifier = register_accept_notifier(&service);
    let store = service.store_for(ME).await.unwrap();
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let target_msg = "m:inbound-1".to_string();

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let target = target_msg.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.message" {
                count.fetch_add(1, Ordering::SeqCst);
                let hist = store.history(conv_id, 10, None).expect("history query");
                assert!(
                    hist.items.iter().any(
                        |item| matches!(item, ConversationHistoryItem::Message(m) if m.id == target)
                    ),
                    "message not visible in history when event was announced"
                );
            }
        })));
    }

    let inbound = test_message(&target_msg, &conv, PEER, "text/plain", b"inbound text");
    service.deliver_inbound(ME, inbound).await.unwrap();

    assert_eq!(probe_called.load(Ordering::SeqCst), 1);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "conversation.message");
    assert_eq!(events[0].subject, conv);
    assert_eq!(events[0].item.as_deref(), Some(target_msg.as_str()));
}

#[tokio::test]
async fn a_held_or_dropped_inbound_message_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();

    let hold_notifier =
        FixedNotifier::new(|| NotifyOutcome::Answered(Admission::Hold("wait".into())));
    service.register_service_notifier(
        ME.to_string(),
        Arc::downgrade(&hold_notifier) as Weak<dyn ConversationNotifier>,
    );

    let msg1 = test_message("m:held", &conv, PEER, "text/plain", b"held text");
    service.deliver_inbound(ME, msg1).await.unwrap();

    let drop_notifier = FixedNotifier::new(|| {
        NotifyOutcome::Answered(Admission::Drop(DropAnswer {
            reason: "spam".into(),
            report: false,
        }))
    });
    service.register_service_notifier(
        ME.to_string(),
        Arc::downgrade(&drop_notifier) as Weak<dyn ConversationNotifier>,
    );

    let msg2 = test_message("m:dropped", &conv, PEER, "text/plain", b"dropped text");
    service.deliver_inbound(ME, msg2).await.unwrap();

    let events = read_events(&bus, "app-1");
    assert!(events.is_empty(), "held or dropped messages must record nothing");
}

#[tokio::test]
async fn a_racing_second_accept_records_no_second_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let _notifier = register_accept_notifier(&service);
    let store = service.store_for(ME).await.unwrap();
    let conv = service.open_direct(ME, PEER).await.unwrap();

    let msg = test_message("m:racing-accept", &conv, PEER, "text/plain", b"racing text");
    service.deliver_inbound(ME, msg).await.unwrap();

    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 1);

    let became_visible = service
        .admit_and_announce(&store, ME, &conv, "m:racing-accept", &Admission::Accept, 1001)
        .unwrap();
    assert!(!became_visible);

    let events_after = read_events(&bus, "app-1");
    assert_eq!(events_after.len(), 1, "no second event on duplicate accept");
}

#[tokio::test]
async fn every_delivery_state_change_records_a_delivery_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let msg_id = service.send(ME, &conv, "text/plain", b"delivery states".to_vec()).await.unwrap();

    assert_eq!(read_events(&bus, "app-1").len(), 1);

    for state in [
        ConversationDeliveryState::Pending,
        ConversationDeliveryState::Deposited,
        ConversationDeliveryState::Delivered,
        ConversationDeliveryState::Failed,
    ] {
        service.update_delivery_state(ME, &msg_id, state).await.unwrap();
    }

    service.retry(ME, &msg_id).await.unwrap();

    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 6);
    for e in &events[1..] {
        assert_eq!(e.kind, "conversation.delivery");
        assert_eq!(e.subject, conv);
        assert_eq!(e.item.as_deref(), Some(msg_id.as_str()));
    }
}

#[tokio::test]
async fn deleting_a_visible_message_records_a_message_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let msg_id = service.send(ME, &conv, "text/plain", b"delete me".to_vec()).await.unwrap();

    service.delete_message(ME, &msg_id, false).await.unwrap();

    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "conversation.message");
    assert_eq!(events[1].kind, "conversation.message");
    assert_eq!(events[1].subject, conv);
    assert_eq!(events[1].item.as_deref(), Some(msg_id.as_str()));
}

#[tokio::test]
async fn a_deletion_request_is_visible_to_history_when_its_event_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let _notifier = register_accept_notifier(&service);
    let store = service.store_for(ME).await.unwrap();
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let target_msg = "m:peer-msg-1".to_string();

    let inbound = test_message(&target_msg, &conv, PEER, "text/plain", b"original text");
    service.deliver_inbound(ME, inbound).await.unwrap();

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let target = target_msg.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, _conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.message" {
                count.fetch_add(1, Ordering::SeqCst);
                let row = store.get_message(&target).unwrap().unwrap();
                assert!(row.deleted_at.is_some(), "message was not deleted when event announced");
            }
        })));
    }

    let del_req = test_message(
        "m:peer-del-req",
        &conv,
        PEER,
        DELETION_REQUEST_CONTENT_TYPE,
        &dag::deletion_request_body(&target_msg),
    );
    service.deliver_inbound(ME, del_req).await.unwrap();

    assert_eq!(probe_called.load(Ordering::SeqCst), 1);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, "conversation.message");
    assert_eq!(events[1].subject, conv);
    assert_eq!(events[1].item.as_deref(), Some(target_msg.as_str()));
}

#[tokio::test]
async fn an_undecided_message_deleted_by_its_author_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let store = service.store_for(ME).await.unwrap();
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let target_msg = "m:undecided-1";

    {
        let conn = store.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        store
            .insert_incoming_if_absent(
                &tx,
                &conv,
                target_msg,
                PEER,
                1000,
                "text/plain",
                &[],
                b"undecided text",
                &[0u8; 64],
                1000,
                100,
            )
            .unwrap();
        tx.commit().unwrap();
    }

    let del_req = test_message(
        "m:undecided-del",
        &conv,
        PEER,
        DELETION_REQUEST_CONTENT_TYPE,
        &dag::deletion_request_body(target_msg),
    );
    service.deliver_inbound(ME, del_req).await.unwrap();

    let events = read_events(&bus, "app-1");
    assert!(events.is_empty(), "deletion of never-visible undecided message records nothing");
}

#[tokio::test]
async fn a_group_deletion_entry_is_visible_to_history_when_its_event_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let _notifier = register_accept_notifier(&service);
    let store = service.store_for(ME).await.unwrap();

    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let me = SigningKey::generate(&mut rand_core::OsRng);
    let conv = group_with(&store, &[(ME, &me), ("svc:alice", &alice)]);
    let msg_entry = make_entry(&alice, "svc:alice", 1000, "text/plain", b"group msg to delete");
    let target_id = msg_entry.entry_id.clone();

    let push_msg = push_request(&alice, &conv.id, vec![msg_entry]);
    service.group_push(ME, "did:key:zAlice", serde_json::to_vec(&push_msg).unwrap()).await.unwrap();

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let target = target_id.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, _conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.message" {
                count.fetch_add(1, Ordering::SeqCst);
                let row = store.get_message(&target).unwrap().unwrap();
                assert!(
                    row.deleted_at.is_some(),
                    "group message not marked deleted when event arrived"
                );
            }
        })));
    }

    let del_entry = make_entry(
        &alice,
        "svc:alice",
        1001,
        DELETION_REQUEST_CONTENT_TYPE,
        &dag::deletion_request_body(&target_id),
    );
    let push_del = push_request(&alice, &conv.id, vec![del_entry]);
    service.group_push(ME, "did:key:zAlice", serde_json::to_vec(&push_del).unwrap()).await.unwrap();

    assert_eq!(probe_called.load(Ordering::SeqCst), 1);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, "conversation.message");
    assert_eq!(events[1].subject, conv.id);
    assert_eq!(events[1].item.as_deref(), Some(target_id.as_str()));
}

#[tokio::test]
async fn a_peer_refusal_records_a_delivery_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let store = service.store_for(ME).await.unwrap();

    let conv = service.open_direct(ME, PEER).await.unwrap();
    let msg_id = service.send(ME, &conv, "text/plain", b"will refuse".to_vec()).await.unwrap();

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let target = msg_id.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, _conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.delivery" {
                count.fetch_add(1, Ordering::SeqCst);
                let row = store.get_message(&target).unwrap().unwrap();
                assert!(row.refused.is_some(), "row was not marked refused");
            }
        })));
    }

    let refusal_notice = test_message(
        "m:refusal-notice-1",
        &conv,
        PEER,
        REFUSAL_NOTICE_CONTENT_TYPE,
        &dag::refusal_notice_body(&msg_id, "refused by recipient"),
    );
    service.deliver_inbound(ME, refusal_notice).await.unwrap();

    assert_eq!(probe_called.load(Ordering::SeqCst), 1);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, "conversation.delivery");
    assert_eq!(events[1].subject, conv);
    assert_eq!(events[1].item.as_deref(), Some(msg_id.as_str()));
}

#[derive(Debug)]
struct RefusingDeliveryProxy {
    peer: Arc<crate::ConversationService>,
}

#[async_trait::async_trait]
impl syneroym_rpc::ServiceProxy for RefusingDeliveryProxy {
    async fn invoke(&self, request: syneroym_rpc::ProxyRequest) -> Result<Value, ProxyError> {
        if request.method == "prekey-bundle" {
            let bundle = self
                .peer
                .prekey_bundle(&request.target_service, ME)
                .await
                .map_err(|e| ProxyError::Transport(e.to_string()))?;
            return Ok(serde_json::from_slice(&bundle).unwrap());
        }
        if request.method == "deliver" {
            let ack = serde_json::json!({
                "message_id": request.idempotency_key.unwrap_or_default(),
                "refused": "refused by policy"
            });
            return Ok(ack);
        }
        Err(ProxyError::Transport("unsupported".to_string()))
    }
}

#[tokio::test]
async fn a_refused_delivery_ack_records_a_delivery_event() {
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir1.path()).await;
    let peer = event_service(dir2.path()).await.0;
    let store = service.store_for(ME).await.unwrap();

    let master = Identity::generate().unwrap();
    let instance = Identity::generate().unwrap();
    let cert = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    service.registry().set_owner(ME.to_string(), "did:key:zOwner".to_string()).await.unwrap();
    service.registry().set_instance_cert(ME.to_string(), cert).await.unwrap();

    let proxy: Arc<dyn syneroym_rpc::ServiceProxy> = Arc::new(RefusingDeliveryProxy { peer });
    service.set_service_proxy(Arc::downgrade(&proxy));

    let conv = service.open_direct(ME, "svc:bob").await.unwrap();
    let msg_id = service.send(ME, &conv, "text/plain", b"refuse me".to_vec()).await.unwrap();

    let probe_called = Arc::new(AtomicUsize::new(0));
    {
        let store = store.clone();
        let target = msg_id.clone();
        let count = probe_called.clone();
        service.set_announce_probe(Some(Box::new(move |kind, _conv_id| {
            assert!(store.conn_is_free(), "connection lock was held when announce was called");
            if kind == "conversation.delivery" {
                count.fetch_add(1, Ordering::SeqCst);
                let row = store.get_message(&target).unwrap().unwrap();
                assert!(row.refused.is_some(), "row was not marked refused");
            }
        })));
    }

    service.drain_once().await;

    assert_eq!(probe_called.load(Ordering::SeqCst), 2);
    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 3);
    assert_eq!(events[1].kind, "conversation.delivery");
    assert_eq!(events[1].subject, conv);
    assert_eq!(events[1].item.as_deref(), Some(msg_id.as_str()));
    assert_eq!(events[2].kind, "conversation.delivery");
    assert_eq!(events[2].subject, conv);
    assert_eq!(events[2].item.as_deref(), Some(msg_id.as_str()));
}

#[tokio::test]
async fn an_accepted_group_entry_records_a_message_event() {
    let dir = tempfile::tempdir().unwrap();
    let (service, bus) = event_service(dir.path()).await;
    let _notifier = register_accept_notifier(&service);
    let store = service.store_for(ME).await.unwrap();

    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let me = SigningKey::generate(&mut rand_core::OsRng);
    let conv = group_with(&store, &[(ME, &me), ("svc:alice", &alice)]);
    let msg_entry = make_entry(&alice, "svc:alice", 1000, "text/plain", b"group welcome");
    let target_id = msg_entry.entry_id.clone();

    let push_msg = push_request(&alice, &conv.id, vec![msg_entry]);
    service.group_push(ME, "did:key:zAlice", serde_json::to_vec(&push_msg).unwrap()).await.unwrap();

    let events = read_events(&bus, "app-1");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "conversation.message");
    assert_eq!(events[0].subject, conv.id);
    assert_eq!(events[0].item.as_deref(), Some(target_id.as_str()));
}

#[tokio::test]
async fn import_records_nothing() {
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let (service1, _) = event_service(dir1.path()).await;
    let (service2, bus2) = event_service(dir2.path()).await;

    let conv = service1.open_direct(ME, PEER).await.unwrap();
    service1.send(ME, &conv, "text/plain", b"to export".to_vec()).await.unwrap();

    let mut cursor = None;
    let mut total_imported = 0;
    loop {
        let chunk = service1.export_history(ME, cursor).await.unwrap();
        total_imported += service2.import_history(ME, chunk.data).await.unwrap();
        cursor = chunk.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(total_imported > 0);

    let events2 = read_events(&bus2, "app-1");
    assert!(events2.is_empty(), "import must record no events");
}

#[tokio::test]
async fn no_bus_set_records_nothing_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    assert!(service.event_bus().is_none());

    let conv = service.open_direct(ME, PEER).await.unwrap();
    let msg_id = service.send(ME, &conv, "text/plain", b"nobus".to_vec()).await.unwrap();

    let inbound = test_message("m:nobus-inbound", &conv, PEER, "text/plain", b"inbound text");
    service.deliver_inbound(ME, inbound).await.unwrap();

    service.update_delivery_state(ME, &msg_id, ConversationDeliveryState::Delivered).await.unwrap();

    service.delete_message(ME, &msg_id, false).await.unwrap();

    let store = service.store_for(ME).await.unwrap();
    let msg = store.get_message(&msg_id).unwrap().unwrap();
    assert!(msg.deleted_at.is_some());
}
