//! One node that hosts both a service with a mailbox and the mailbox
//! itself, with a fake vault and a fake dispatch behind the puller.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use dashmap::DashMap;
use iroh::{EndpointAddr, SecretKey};
use serde_json::{Value, json};
use syneroym_core::{
    config::{MailboxRole, RetryPolicy},
    dht_registry::{
        EndpointInfo, EndpointMailbox, EndpointMechanism, EndpointType, RegistryClient,
        SignedEndpointInfo,
    },
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    mailbox::{HostedMailbox, PullSummary, StatusAnswer},
    storage::MockStorage,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_mailbox::MailboxService;
use syneroym_mailbox_format::{
    MailboxSecretKey, MessageContent, Receipt, Recipient, Sealed, Sender, SentDeposit, seal,
};
use syneroym_rpc::{
    AuthLevel, CallerContext, CallerProof, NativeDispatchRegistry, NativeInvocation, NativeService,
    ProxyError,
};
use tokio::time;

use super::super::{DeferredDispatch, MailboxPuller, ReceiverVault};
use crate::{
    handshake::now_secs,
    preamble::{OutboundIdentity, RoutePreamble},
    proxy::{ProxyRouter, RemoteHop, RemotePeer},
    route_handler::deferred::DeferredOutcome,
    test_support::{
        Substrate,
        mailbox::{CountingMailbox, TestAnchors},
        registry_serving_only,
    },
};

pub(super) const WEEK_SECS: u64 = 7 * 24 * 3600;
pub(super) const OWNER: &str = "did:key:zOwner";
pub(super) const INTERFACE: &str = "greeter";

/// A sending member: a master, an instance key and its certificate.
pub(super) struct SenderMember {
    pub(super) master: Identity,
    pub(super) instance: Identity,
    pub(super) cert: DelegationCertificate,
}

impl SenderMember {
    pub(super) fn new() -> Self {
        Self::with_cert_life(3600)
    }

    pub(super) fn with_cert_life(secs: u64) -> Self {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            secs,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        Self { master, instance, cert }
    }

    pub(super) fn did(&self) -> String {
        substrate::derive_did_key(&self.master.public_key())
    }

    pub(super) fn caller(&self) -> CallerContext {
        let mut caller = CallerContext::verified(&self.did(), AuthLevel::Delegated, Vec::new());
        caller.proof = Some(CallerProof {
            pubkey_hex: hex::encode(self.instance.public_key().to_bytes()),
            delegation_json: Some(self.cert.to_json().unwrap()),
            ..Default::default()
        });
        caller
    }
}

/// What the vault answers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VaultMode {
    Open,
    Locked,
    NoKey,
}

pub(super) struct FakeVault {
    pub(super) mode: Mutex<VaultMode>,
    pub(super) key: MailboxSecretKey,
    pub(super) record: Mutex<Option<SignedEndpointInfo>>,
}

#[async_trait::async_trait]
impl ReceiverVault for FakeVault {
    async fn mailbox_key(&self, _service_id: &str) -> anyhow::Result<Option<MailboxSecretKey>> {
        match *self.mode.lock().unwrap() {
            VaultMode::Open => Ok(Some(self.key.clone())),
            VaultMode::NoKey => Ok(None),
            VaultMode::Locked => Err(anyhow::anyhow!("the vault is locked")),
        }
    }

    async fn deploy_record(&self, _service_id: &str) -> anyhow::Result<Option<SignedEndpointInfo>> {
        Ok(self.record.lock().unwrap().clone())
    }
}

/// One call the fake dispatch saw.
#[derive(Clone)]
pub(super) struct Delivered {
    pub(super) interface: String,
    pub(super) method: String,
    pub(super) caller: CallerContext,
}

type Hook = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Records each call and answers the scripted outcomes, then `Succeeded`.
#[derive(Default)]
pub(super) struct FakeDispatch {
    pub(super) calls: Mutex<Vec<Delivered>>,
    pub(super) outcomes: Mutex<VecDeque<DeferredOutcome>>,
    /// Waits this long (on the tokio clock) before it answers.
    pub(super) delay: Mutex<Duration>,
    /// Runs once, at the first delivery, before it answers.
    pub(super) on_first: Mutex<Option<Hook>>,
}

impl FakeDispatch {
    pub(super) fn script(&self, outcome: DeferredOutcome) {
        self.outcomes.lock().unwrap().push_back(outcome);
    }

    pub(super) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub(super) fn last(&self) -> Delivered {
        self.calls.lock().unwrap().last().cloned().expect("a delivery")
    }
}

#[async_trait::async_trait]
impl DeferredDispatch for FakeDispatch {
    async fn dispatch_deferred(
        &self,
        _service_id: &str,
        interface: &str,
        content: &MessageContent,
        caller: &CallerContext,
    ) -> DeferredOutcome {
        self.calls.lock().unwrap().push(Delivered {
            interface: interface.to_string(),
            method: content.method.clone(),
            caller: caller.clone(),
        });
        let hook = self.on_first.lock().unwrap().take();
        if let Some(hook) = hook {
            hook().await;
        }
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            time::sleep(delay).await;
        }
        self.outcomes.lock().unwrap().pop_front().unwrap_or(DeferredOutcome::Succeeded)
    }
}

/// A remote hop that records the identity of each call and answers `null`.
#[derive(Debug, Default)]
pub(super) struct RecordingHop {
    pub(super) preambles: Mutex<Vec<RoutePreamble>>,
    pub(super) methods: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl RemoteHop for RecordingHop {
    async fn call(
        &self,
        _peer: &RemotePeer,
        route: &RoutePreamble,
        identity: Option<&OutboundIdentity>,
        request: &syneroym_rpc::JsonRpcRequest,
        _timeout: Duration,
    ) -> Result<Value, ProxyError> {
        let preamble = identity.map_or_else(|| route.clone(), |id| id.apply(route, now_secs()));
        self.preambles.lock().unwrap().push(preamble);
        self.methods.lock().unwrap().push(request.method.clone());
        Ok(json!({ "removed": true }))
    }
}

pub(super) struct PullNode {
    pub(super) puller: Arc<MailboxPuller>,
    pub(super) hop: Arc<RecordingHop>,
    pub(super) registry: EndpointRegistry,
    pub(super) mailbox: Arc<CountingMailbox>,
    pub(super) mailbox_did: String,
    pub(super) vault: Arc<FakeVault>,
    pub(super) dispatch: Arc<FakeDispatch>,
    /// What the node's proxy reads about a sender's revoked keys.
    pub(super) anchors: Arc<TestAnchors>,
    pub(super) service_id: String,
    pub(super) svc_master: Identity,
    pub(super) node_identity: Arc<Identity>,
    _native_dispatch: NativeDispatchRegistry,
    _dir: tempfile::TempDir,
}

/// What a test can set before the node is built.
#[derive(Default)]
pub(super) struct Setup {
    pub(super) methods: Vec<String>,
    pub(super) mailbox_role: MailboxRole,
    /// The mailbox is not on this node: a stand-in registry answers for its
    /// node record, and the hop records the calls.
    pub(super) remote_mailbox: bool,
}

pub(super) async fn pull_node(methods: &[&str]) -> PullNode {
    pull_node_with(Setup {
        methods: methods.iter().map(ToString::to_string).collect(),
        ..Setup::default()
    })
    .await
}

/// The hosted service: its master names the service id, and the node derives
/// the instance key from its own key and the owner.
async fn host_service(
    registry: &EndpointRegistry,
    node_identity: &Identity,
    svc_master: &Identity,
    mailbox_did: &str,
    methods: Vec<String>,
) {
    let service_id = substrate::derive_did_key(&svc_master.public_key());
    let instance = node_identity.derive_service_identity(OWNER, &service_id);
    let cert = DelegationCertificate::issue(
        svc_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_owner(service_id.clone(), OWNER.to_string()).await.unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();
    registry
        .register(
            service_id.clone(),
            INTERFACE.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: service_id.clone() },
        )
        .await
        .unwrap();
    let fact = HostedMailbox { mailbox: mailbox_did.to_string(), max_ttl_secs: WEEK_SECS, methods };
    registry.set_mailbox(service_id, fact).await.unwrap();
}

pub(super) async fn pull_node_with(setup: Setup) -> PullNode {
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mailbox_identity = Identity::generate().unwrap();
    let mailbox_did = substrate::derive_did_key(&mailbox_identity.public_key());

    let mailbox_anchors = Arc::new(TestAnchors::default());
    let service =
        MailboxService::open(&setup.mailbox_role, dir.path(), &mailbox_did, mailbox_anchors)
            .unwrap();
    let mailbox = Arc::new(CountingMailbox::new(service));
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    native_dispatch.insert(mailbox_did.clone(), mailbox.clone() as Arc<dyn NativeService>);
    if !setup.remote_mailbox {
        registry
            .register(
                mailbox_did.clone(),
                "mailbox".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: mailbox_did.clone() },
            )
            .await
            .unwrap();
    }

    let svc_master = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&svc_master.public_key());
    host_service(&registry, &node_identity, &svc_master, &mailbox_did, setup.methods).await;

    let mailbox_key = MailboxSecretKey::generate().unwrap();
    let record = service_record(&svc_master, &service_id, &mailbox_did, &mailbox_key);
    let vault = Arc::new(FakeVault {
        mode: Mutex::new(VaultMode::Open),
        key: mailbox_key,
        record: Mutex::new(Some(record)),
    });

    let anchors = Arc::new(TestAnchors::default());
    let hop = Arc::new(RecordingHop::default());
    let registry_client = if setup.remote_mailbox {
        Arc::new(RegistryClient::new(false, Some(mailbox_registry_url(&mailbox_identity).await)))
    } else {
        Arc::new(RegistryClient::new(false, None))
    };
    let proxy = Arc::new(
        ProxyRouter::new(
            registry.clone(),
            registry_client,
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity.clone(),
            RetryPolicy {
                max_attempts: 1,
                initial_backoff_ms: 1,
                backoff_multiplier: 2.0,
                max_backoff_ms: 2,
            },
        )
        .with_anchor_resolver(anchors.clone()),
    );
    let dispatch = Arc::new(FakeDispatch::default());
    let puller = Arc::new(MailboxPuller::new(
        proxy.clone(),
        dispatch.clone(),
        vault.clone(),
        registry.clone(),
    ));
    PullNode {
        puller,
        hop,
        registry,
        mailbox,
        mailbox_did,
        vault,
        dispatch,
        anchors,
        service_id,
        svc_master,
        node_identity,
        _native_dispatch: native_dispatch,
        _dir: dir,
    }
}

/// A registry that serves the node record of the mailbox host, so a call
/// to a mailbox that is not on this node reaches the recording hop.
async fn mailbox_registry_url(mailbox_identity: &Identity) -> String {
    let did = substrate::derive_did_key(&mailbox_identity.public_key());
    let addr = EndpointAddr::new(SecretKey::generate(&mut rand::rng()).public());
    let record = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did,
        endpoint_type: EndpointType::Substrate,
        mechanisms: vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: serde_json::to_vec(&addr).unwrap(),
            relay_url: None,
        }],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: now_secs() + 3600,
        generation: 0,
        mailbox: None,
    }
    .sign(mailbox_identity)
    .unwrap();
    registry_serving_only(&record, Substrate::Missing).await
}

pub(super) fn service_record(
    master: &Identity,
    service_id: &str,
    mailbox_did: &str,
    key: &MailboxSecretKey,
) -> SignedEndpointInfo {
    EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: Vec::new(),
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: now_secs() + 30 * 24 * 3600,
        generation: 0,
        mailbox: Some(EndpointMailbox {
            service_id: mailbox_did.to_string(),
            key: key.public_key(),
        }),
    }
    .sign(master)
    .unwrap()
}

pub(super) fn greet() -> MessageContent {
    MessageContent {
        interface: INTERFACE.to_string(),
        method: "greet".to_string(),
        params: json!({ "who": "world" }),
        idempotency_key: "key-1".to_string(),
    }
}

pub(super) fn with_key(key: &str) -> MessageContent {
    MessageContent { idempotency_key: key.to_string(), ..greet() }
}

impl PullNode {
    /// Runs one pass, which registers the queue when it is new.
    pub(super) async fn pass(&self) -> PullSummary {
        self.puller.pull_once().await
    }

    pub(super) async fn call_mailbox_as(
        &self,
        sender: &SenderMember,
        method: &str,
        params: Value,
    ) -> syneroym_rpc::RpcResult<Value> {
        let invocation = NativeInvocation {
            interface: "mailbox".to_string(),
            method: method.to_string(),
            params,
            caller: sender.caller(),
        };
        self.mailbox.dispatch(invocation).await.map(|r| r.payload)
    }

    /// Seals `content` for the hosted service and deposits it as `sender`.
    pub(super) async fn deposit(
        &self,
        sender: &SenderMember,
        content: &MessageContent,
    ) -> SentDeposit {
        let key = self.vault.key.public_key();
        let recipient = Recipient { service_id: &self.service_id, mailbox_key: &key };
        let as_sender = Sender { instance: &sender.instance, certificate: &sender.cert };
        let sealed = seal(content, recipient, as_sender, WEEK_SECS, now_secs()).unwrap();
        let params = serde_json::to_value(&sealed.deposit).unwrap();
        self.call_mailbox_as(sender, "deposit", params).await.unwrap();
        sealed.sent
    }

    /// Deposits an already sealed message as `depositor`.
    pub(super) async fn deposit_sealed(&self, depositor: &SenderMember, sealed: &Sealed) {
        let params = serde_json::to_value(&sealed.deposit).unwrap();
        self.call_mailbox_as(depositor, "deposit", params).await.unwrap();
    }

    /// The receipt the mailbox holds for `sent`, as its sender reads it.
    pub(super) async fn receipt(&self, sender: &SenderMember, sent: &SentDeposit) -> Receipt {
        let params = json!({ "deposit_ids": [sent.deposit_id] });
        let answer = self.call_mailbox_as(sender, "status", params).await.unwrap();
        let answer: StatusAnswer = serde_json::from_value(answer).unwrap();
        answer.receipts.into_iter().find(|e| e.deposit_id == sent.deposit_id).unwrap().receipt
    }
}
