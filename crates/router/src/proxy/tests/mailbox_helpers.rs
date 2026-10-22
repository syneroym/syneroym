//! A sender node with a real mailbox service on the same node, a receiver
//! that registered there, and a `MockHop` standing in for the receiver's
//! own node.

use std::sync::atomic::AtomicBool;

use serde_json::json;
use syneroym_core::{
    config::MailboxRole,
    dht_registry::{EndpointInfo, EndpointMailbox, EndpointType, SignedEndpointInfo},
    mailbox::{
        AckAnswer, AckOutcome, ClaimAnswer, METHOD_ACK, METHOD_CLAIM, METHOD_REGISTER,
        METHOD_STATUS, StatusAnswer,
    },
};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_mailbox::MailboxService;
use syneroym_mailbox_format::{
    MailboxSecretKey, OpenPolicy, OpenedMessage, Receipt, RejectCode, SentDeposit, open,
};
use syneroym_rpc::{DepositTicket, NativeDispatchRegistry};

use super::helpers::*;
use crate::{handshake::now_secs, net_iroh::ResolvedPeer};

pub(super) const SENDER: &str = "did:key:zMailboxSender";
const OWNER: &str = "did:key:zOwner";

/// A master DID with no key behind it that we ever use: any valid did:key.
fn fresh_did() -> String {
    substrate::derive_did_key(&Identity::generate().unwrap().public_key())
}

/// Answers the master anchor lookups of a test: nothing is revoked unless
/// a test says so.
#[derive(Debug, Default)]
pub(super) struct TestAnchors {
    pub(super) revoked: Mutex<Vec<String>>,
    pub(super) unreadable: AtomicBool,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for TestAnchors {
    async fn resolve_master_anchor(
        &self,
        _master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("the anchor cannot be read"));
        }
        Ok(MasterAnchorPayload {
            revoked_keys: self.revoked.lock().unwrap().clone(),
            ..Default::default()
        })
    }
}

/// The real mailbox service, with a count of the methods it was called with.
#[derive(Debug)]
pub(super) struct CountingMailbox {
    inner: MailboxService,
    methods: Mutex<Vec<String>>,
}

impl CountingMailbox {
    pub(super) fn count(&self, method: &str) -> usize {
        self.methods.lock().unwrap().iter().filter(|m| *m == method).count()
    }
}

#[async_trait::async_trait]
impl NativeService for CountingMailbox {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        self.methods.lock().unwrap().push(invocation.method.clone());
        self.inner.dispatch(invocation).await
    }
}

/// The receiving service member.
pub(super) struct Receiver {
    master: Identity,
    pub(super) instance: Identity,
    pub(super) cert: DelegationCertificate,
    pub(super) mailbox_key: MailboxSecretKey,
}

impl Receiver {
    fn new() -> Self {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        Self { master, instance, cert, mailbox_key: MailboxSecretKey::generate().unwrap() }
    }

    pub(super) fn service_id(&self) -> String {
        substrate::derive_did_key(&self.master.public_key())
    }

    fn caller(&self) -> CallerContext {
        let mut caller =
            CallerContext::verified(&self.service_id(), AuthLevel::Delegated, Vec::new());
        caller.proof = Some(CallerProof {
            pubkey_hex: hex::encode(self.instance.public_key().to_bytes()),
            delegation_json: Some(self.cert.to_json().unwrap()),
            ..Default::default()
        });
        caller
    }

    fn record(&self, mailbox_did: &str) -> SignedEndpointInfo {
        EndpointInfo {
            service_id: self.service_id(),
            substrate_id: self.service_id(),
            endpoint_type: EndpointType::Service,
            mechanisms: Vec::new(),
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: now_secs() + 30 * 24 * 3600,
            generation: 0,
            mailbox: Some(EndpointMailbox {
                service_id: mailbox_did.to_string(),
                key: self.mailbox_key.public_key(),
            }),
        }
        .sign(&self.master)
        .unwrap()
    }
}

pub(super) struct SenderNode {
    pub(super) router: ProxyRouter,
    pub(super) outbox: Option<Arc<ProxyOutbox>>,
    pub(super) hop: Arc<MockHop>,
    pub(super) mailbox: Arc<CountingMailbox>,
    pub(super) mailbox_did: String,
    /// What the router asks about a receiver's revoked keys.
    pub(super) router_anchors: Arc<TestAnchors>,
    pub(super) receiver: Receiver,
    pub(super) registry: EndpointRegistry,
    pub(super) sender_master: Identity,
    pub(super) sender_instance_pubkey_hex: String,
    pub(super) sender_cert: DelegationCertificate,
    _native_dispatch: NativeDispatchRegistry,
    _dir: tempfile::TempDir,
}

pub(super) async fn sender_node(role: MailboxRole, outcomes: Vec<MockOutcome>) -> SenderNode {
    build_sender_node(role, outcomes, None).await
}

/// A sender node that also has a durable outbox and knows the receiver
/// only from a stored record (so the receiver has a mailbox but no address).
pub(super) async fn outbox_sender_node(max_attempts: u8) -> SenderNode {
    build_sender_node(MailboxRole::default(), Vec::new(), Some(max_attempts)).await
}

async fn build_sender_node(
    role: MailboxRole,
    outcomes: Vec<MockOutcome>,
    outbox_attempts: Option<u8>,
) -> SenderNode {
    let registry = empty_registry();
    let node_identity = Arc::new(Identity::generate().unwrap());
    registry.set_owner(SENDER.to_string(), OWNER.to_string()).await.unwrap();
    let instance = node_identity.derive_service_identity(OWNER, SENDER);
    let sender_master = Identity::generate().unwrap();
    let sender_cert = DelegationCertificate::issue(
        &sender_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(SENDER.to_string(), sender_cert.clone()).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mailbox_did = fresh_did();
    let mailbox_anchors = Arc::new(TestAnchors::default());
    let service = MailboxService::open(&role, dir.path(), &mailbox_did, mailbox_anchors).unwrap();
    let mailbox = Arc::new(CountingMailbox { inner: service, methods: Mutex::new(Vec::new()) });
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    native_dispatch.insert(mailbox_did.clone(), mailbox.clone() as Arc<dyn NativeService>);
    registry
        .register(
            mailbox_did.clone(),
            "mailbox".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: mailbox_did.clone() },
        )
        .await
        .unwrap();

    let hop = Arc::new(MockHop::with_outcomes(outcomes));
    let router_anchors = Arc::new(TestAnchors::default());
    let router = ProxyRouter::new(
        registry.clone(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1,
            backoff_multiplier: 2.0,
            max_backoff_ms: 5,
        },
    )
    .with_anchor_resolver(router_anchors.clone());

    let receiver = Receiver::new();
    let (router, outbox) = match outbox_attempts {
        Some(attempts) => {
            let (router, outbox) =
                with_outbox(router, &registry, dir.path(), attempts, &receiver, &mailbox_did).await;
            (router, Some(outbox))
        }
        None => (router, None),
    };

    let node = SenderNode {
        router,
        outbox,
        hop,
        mailbox,
        mailbox_did,
        router_anchors,
        receiver,
        registry,
        sender_master,
        sender_instance_pubkey_hex: hex::encode(instance.public_key().to_bytes()),
        sender_cert,
        _native_dispatch: native_dispatch,
        _dir: dir,
    };
    node.receiver_registers().await;
    node
}

/// Gives `router` a durable outbox for `SENDER` and a stored record of the
/// receiver, so the receiver is known by its mailbox alone.
async fn with_outbox(
    router: ProxyRouter,
    registry: &EndpointRegistry,
    root: &std::path::Path,
    max_attempts: u8,
    receiver: &Receiver,
    mailbox_did: &str,
) -> (ProxyRouter, Arc<ProxyOutbox>) {
    use syneroym_app_orchestration::{LogicalResolver, StaticInventory};
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    // The worker only drains services the registry still knows.
    registry
        .register(
            SENDER.to_string(),
            "caller-iface".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: SENDER.to_string() },
        )
        .await
        .unwrap();
    let service_dir = root.join("services").join(SENDER);
    std::fs::create_dir_all(&service_dir).unwrap();
    std::fs::write(service_dir.join("state.db"), b"").unwrap();

    let provider = Arc::new(SqliteStorageProvider::new(root, false).unwrap());
    let config = QueueConfig {
        retry: RetryPolicy {
            max_attempts,
            initial_backoff_ms: 1,
            backoff_multiplier: 2.0,
            max_backoff_ms: 4,
        },
        visibility_timeout_ms: 0,
        dlq_max_rows: 100,
        max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
    };
    let resolver = Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())));
    let outbox = Arc::new(ProxyOutbox::new(provider, Arc::new(KeyStore::new()), resolver, config));

    let records = Arc::new(crate::peer_records::PeerRecordStore::open_in_memory().unwrap());
    records.remember(&receiver.record(mailbox_did)).await.unwrap();
    (router.with_outbox(outbox.clone()).with_peer_records(records), outbox)
}

impl SenderNode {
    async fn call_mailbox_as_receiver(&self, method: &str, params: Value) -> Value {
        let invocation = NativeInvocation {
            interface: "mailbox".to_string(),
            method: method.to_string(),
            params,
            caller: self.receiver.caller(),
        };
        self.mailbox.dispatch(invocation).await.unwrap().payload
    }

    async fn receiver_registers(&self) {
        let record = self.receiver.record(&self.mailbox_did);
        self.call_mailbox_as_receiver(METHOD_REGISTER, json!({ "record": record })).await;
    }

    /// A keyed call from the sender service to the receiver.
    pub(super) fn request(&self) -> ProxyRequest {
        let mut req = base_request(&self.receiver.service_id(), "greeter");
        req.method = "greet".to_string();
        req.params = json!({ "who": "world" });
        req.caller = CallerContext::service_system(SENDER);
        req.origin = CallOrigin::Guest { service_id: SENDER.to_string() };
        req.idempotent = true;
        req.idempotency_key = Some("key-1".to_string());
        req
    }

    /// What the sender learns about the receiver from its signed record.
    pub(super) fn peer(&self, with_address: bool) -> ResolvedPeer {
        ResolvedPeer {
            addr: with_address.then(|| synthetic_peer().addr),
            mailbox: Some(EndpointMailbox {
                service_id: self.mailbox_did.clone(),
                key: self.receiver.mailbox_key.public_key(),
            }),
            host_key: None,
        }
    }

    pub(super) fn peer_without_mailbox(&self) -> ResolvedPeer {
        ResolvedPeer { mailbox: None, ..self.peer(true) }
    }

    /// The receiver claims everything that waits, opens each message and
    /// acks it with `receipt_for`.
    pub(super) async fn receiver_acks(
        &self,
        receipt_for: impl Fn(&OpenedMessage, &Receiver) -> Receipt,
    ) {
        let claimed = self.call_mailbox_as_receiver(METHOD_CLAIM, json!({ "max": 10 })).await;
        let claimed: ClaimAnswer = serde_json::from_value(claimed).unwrap();
        assert!(!claimed.messages.is_empty(), "a message waits for the receiver");
        let mut acks = Vec::new();
        for message in &claimed.messages {
            let opened = open(
                &self.receiver.mailbox_key,
                &self.receiver.service_id(),
                &message.deposit_id,
                &message.sealed,
                OpenPolicy::default(),
                now_secs(),
            )
            .unwrap();
            acks.push(json!({
                "sender": opened.sender_master_did(),
                "deposit_id": message.deposit_id,
                "receipt": receipt_for(&opened, &self.receiver),
            }));
        }
        let acked = self.call_mailbox_as_receiver(METHOD_ACK, json!({ "acks": acks })).await;
        let acked: AckAnswer = serde_json::from_value(acked).unwrap();
        assert!(acked.results.iter().all(|r| *r == AckOutcome::Stored), "{acked:?}");
    }

    pub(super) async fn receiver_delivers(&self) {
        self.receiver_acks(|opened, receiver| {
            opened.sign_delivered(&receiver.instance, &receiver.cert, now_secs()).unwrap()
        })
        .await;
    }

    pub(super) async fn receiver_rejects_as_not_eligible(&self) {
        self.receiver_acks(|opened, receiver| {
            opened
                .sign_rejection(
                    &receiver.instance,
                    &receiver.cert,
                    RejectCode::NotEligible,
                    now_secs(),
                )
                .unwrap()
        })
        .await;
    }

    /// The receipt the mailbox holds for `sent`, as the sender reads it.
    pub(super) async fn receipt_for(&self, sent: &SentDeposit) -> Receipt {
        let sender = self.router.sender_identity(SENDER).unwrap();
        let params = json!({ "deposit_ids": [sent.deposit_id] });
        let call = ProxyRouter::mailbox_request(&sender, &self.mailbox_did, METHOD_STATUS, params);
        let answer = self.router.invoke_inner(&call).await.unwrap();
        let answer: StatusAnswer = serde_json::from_value(answer).unwrap();
        answer.receipts.into_iter().next().unwrap().receipt
    }
}

/// What the sender kept for `ticket`.
pub(super) fn sent_of(ticket: &DepositTicket) -> SentDeposit {
    let context: Value = serde_json::from_slice(&ticket.context).unwrap();
    serde_json::from_value(context["sent"].clone()).unwrap()
}
