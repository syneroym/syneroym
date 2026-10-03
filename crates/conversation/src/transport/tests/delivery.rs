//! Inbound delivery through the real `peer_deliver` path: signature check,
//! storage, the admission question to the app, and the answer to the sender.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use syneroym_rpc::{
    Admission, ConversationDeliveryState, ConversationError, ConversationHost, ConversationMessage,
    ConversationNotifier, DropAnswer, NotifyOutcome,
};

use super::{test_service, test_service_with, test_store};
use crate::{
    ConversationConfig, ConversationService,
    crypto::{PrekeyBundle, Session, SessionCrypto, X3dhDoubleRatchetCrypto},
    dag::{
        DELETION_REQUEST_CONTENT_TYPE, REFUSAL_NOTICE_CONTENT_TYPE, deletion_request_body,
        refusal_notice_body,
    },
    envelope::{self, DeliveryPayload},
    ids::derive_conversation_id,
    store::{ConversationConfig as StoreConfig, ConversationStore},
};

const SVC: &str = "svc:receiver";

/// A remote peer that signs and encrypts the way a real sender does.
struct Sender {
    crypto: X3dhDoubleRatchetCrypto,
    session: Session,
    key: SigningKey,
    address: String,
}

impl Sender {
    async fn connect(service: &ConversationService, address: &str) -> Self {
        let bundle_bytes = service.prekey_bundle(SVC, address).await.unwrap();
        let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();
        let store: ConversationStore = test_store();
        let crypto = X3dhDoubleRatchetCrypto::new();
        let session = crypto.begin_session(&store, address, SVC, &bundle).await.unwrap();
        let identity =
            store.local_identity_or_generate(crate::crypto::generate_identity_bytes).unwrap();
        let secret: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
        Self { crypto, session, key: SigningKey::from_bytes(&secret), address: address.to_string() }
    }

    fn conversation(&self) -> String {
        derive_conversation_id(SVC, &self.address)
    }

    /// Delivers one message and returns the reason the receiver refused it,
    /// if it told the sender.
    async fn deliver(
        &mut self,
        service: &ConversationService,
        id: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<Option<String>, ConversationError> {
        let conversation = self.conversation();
        let ts = crate::store::now_ms();
        let payload = DeliveryPayload {
            message_id: id.to_string(),
            conversation_id: conversation.clone(),
            author: self.address.clone(),
            sender_timestamp_ms: ts,
            content_type: content_type.to_string(),
            body: body.to_vec(),
            signature: envelope::sign(
                &self.key,
                id,
                &conversation,
                &self.address,
                ts,
                content_type,
                body,
            ),
        };
        let env = self.crypto.encrypt(&mut self.session, &payload).unwrap();
        let ack =
            service.peer_deliver(SVC, &self.address, serde_json::to_vec(&env).unwrap()).await?;
        let ack: crate::transport::DeliveryAck = serde_json::from_slice(&ack).unwrap();
        Ok(ack.refused)
    }
}

/// Answers every admission question with a fixed outcome and counts them.
#[derive(Debug)]
struct Scripted {
    asked: AtomicUsize,
    answer: fn() -> NotifyOutcome,
    delay: Duration,
}

impl Scripted {
    fn new(answer: fn() -> NotifyOutcome) -> Arc<Self> {
        Arc::new(Self { asked: AtomicUsize::new(0), answer, delay: Duration::ZERO })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ConversationNotifier for Scripted {
    async fn notify_message(&self, _service_id: &str, _msg: ConversationMessage) -> NotifyOutcome {
        self.asked.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        (self.answer)()
    }

    async fn notify_delivery_state(&self, _: &str, _: String, _: ConversationDeliveryState) {}
}

fn accept() -> NotifyOutcome {
    NotifyOutcome::Answered(Admission::Accept)
}

fn drop_and_report() -> NotifyOutcome {
    NotifyOutcome::Answered(Admission::Drop(DropAnswer {
        reason: "rate-limited".to_string(),
        report: true,
    }))
}

fn register(service: &ConversationService, notifier: &Arc<Scripted>) {
    service.register_service_notifier(
        SVC.to_string(),
        Arc::downgrade(notifier) as std::sync::Weak<dyn ConversationNotifier>,
    );
}

async fn stored(service: &ConversationService, id: &str) -> crate::store::StoredMessage {
    service.store_for(SVC).await.unwrap().get_message(id).unwrap().unwrap()
}

#[tokio::test]
async fn a_repeated_delivery_does_not_ask_the_app_twice() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let notifier = Scripted::new(accept);
    register(&service, &notifier);
    let mut sender = Sender::connect(&service, "svc:sender").await;

    sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();
    sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();

    assert_eq!(notifier.asked(), 1);
    assert_eq!(stored(&service, "m:1").await.admission, "accepted");
}

#[tokio::test]
async fn a_message_with_no_app_listening_stays_undecided() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let mut sender = Sender::connect(&service, "svc:sender").await;

    sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();

    assert_eq!(stored(&service, "m:1").await.admission, "undecided");
}

#[tokio::test]
async fn an_app_without_a_handler_has_its_messages_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let notifier = Scripted::new(|| NotifyOutcome::NoHandler);
    register(&service, &notifier);
    let mut sender = Sender::connect(&service, "svc:sender").await;

    sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();

    assert_eq!(stored(&service, "m:1").await.admission, "accepted");
}

#[tokio::test]
async fn a_slow_app_does_not_hold_back_the_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let config = ConversationConfig {
        store: StoreConfig { admission_ask_timeout_ms: 50, ..StoreConfig::default() },
    };
    let service = test_service_with(dir.path(), config).await;
    let notifier = Arc::new(Scripted {
        asked: AtomicUsize::new(0),
        answer: accept,
        delay: Duration::from_secs(30),
    });
    register(&service, &notifier);
    let mut sender = Sender::connect(&service, "svc:sender").await;

    let started = std::time::Instant::now();
    let refused = sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();

    assert!(started.elapsed() < Duration::from_secs(5), "the ask must be cut short");
    assert_eq!(refused, None);
    assert_eq!(stored(&service, "m:1").await.admission, "undecided");
}

#[tokio::test]
async fn a_reported_drop_tells_the_sender_and_a_resend_hears_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let notifier = Scripted::new(drop_and_report);
    register(&service, &notifier);
    let mut sender = Sender::connect(&service, "svc:sender").await;

    let first = sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();
    assert_eq!(first.as_deref(), Some("rate-limited"));

    // The receipt was lost, so the sender delivers the same message again.
    let again = sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();
    assert_eq!(again.as_deref(), Some("rate-limited"));
    assert_eq!(notifier.asked(), 1, "the app is not asked a second time");
}

#[tokio::test]
async fn a_system_message_is_stored_as_accepted_and_never_put_to_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let notifier = Scripted::new(accept);
    register(&service, &notifier);
    let mut sender = Sender::connect(&service, "svc:sender").await;
    sender.deliver(&service, "m:1", "text/plain", b"hello").await.unwrap();

    let body = deletion_request_body("m:1");
    sender.deliver(&service, "m:del", DELETION_REQUEST_CONTENT_TYPE, &body).await.unwrap();

    let system = stored(&service, "m:del").await;
    assert!(system.system);
    assert_eq!(system.admission, "accepted");
    let store = service.store_for(SVC).await.unwrap();
    assert!(store.undecided_messages(i64::MAX).unwrap().is_empty());
    assert_eq!(notifier.asked(), 1, "only the ordinary message was asked about");
}

#[tokio::test]
async fn a_refusal_notice_is_capped_and_only_the_peer_may_send_it() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let store = service.store_for(SVC).await.unwrap();
    let mut peer = Sender::connect(&service, "svc:peer").await;
    let mut stranger = Sender::connect(&service, "svc:stranger").await;
    let conversation = peer.conversation();
    store.get_or_create_direct("svc:peer", &conversation, 1_000).unwrap();
    store
        .insert_outgoing_and_enqueue(
            &conversation,
            "m:mine",
            SVC,
            1_000,
            "text/plain",
            b"to peer",
            &[0u8; 64],
            "svc:peer",
            1_000,
            false,
        )
        .unwrap();

    // A stranger names a message that is not in the stranger's conversation.
    let fake = refusal_notice_body("m:mine", "fake");
    stranger.deliver(&service, "m:fake", REFUSAL_NOTICE_CONTENT_TYPE, &fake).await.unwrap();
    assert_eq!(stored(&service, "m:mine").await.refused, None);

    let long = "r".repeat(500);
    let real = refusal_notice_body("m:mine", &long);
    peer.deliver(&service, "m:real", REFUSAL_NOTICE_CONTENT_TYPE, &real).await.unwrap();
    assert_eq!(stored(&service, "m:mine").await.refused.map(|r| r.chars().count()), Some(120));
}
