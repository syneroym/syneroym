//! A real mailbox service, a receiver member and master-anchor stand-ins,
//! shared by the tests of the sender and the receiver side.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use serde_json::Value;
use syneroym_core::{
    dht_registry::{
        EndpointInfo, EndpointMailbox, EndpointType, MasterAnchorPayload, SignedEndpointInfo,
    },
    mailbox::AckOutcome,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_mailbox::MailboxService;
use syneroym_mailbox_format::MailboxSecretKey;
use syneroym_rpc::{
    AuthLevel, CallerContext, CallerProof, NativeInvocation, NativeResponse, NativeService,
    RpcError, RpcResult,
};
use tokio::sync::Notify;

use crate::{MasterAnchorResolver, handshake::now_secs};

/// Answers the master anchor lookups of a test: nothing is revoked unless
/// a test says so.
#[derive(Debug, Default)]
pub(crate) struct TestAnchors {
    pub(crate) revoked: Mutex<Vec<String>>,
    pub(crate) unreadable: AtomicBool,
    /// How many anchors were asked for.
    pub(crate) lookups: AtomicUsize,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for TestAnchors {
    async fn resolve_master_anchor(
        &self,
        _master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
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
pub(crate) struct CountingMailbox {
    inner: MailboxService,
    methods: Mutex<Vec<String>>,
    /// When set, every `deposit` is refused with this code.
    refuse_deposits: Mutex<Option<i32>>,
    /// Calls of a method that fail with a code, until cleared.
    failures: Mutex<Vec<(String, i32)>>,
    /// A method that waits at the mailbox until the test lets it go.
    hold: Mutex<Option<Hold>>,
    /// Added to `visible_again_at_secs` in every claim answer.
    claim_clock_shift_secs: Mutex<i64>,
    /// Replaces `limits.max_ttl_secs` in every register answer.
    register_ttl: Mutex<Option<u64>>,
    /// Replaces the results of every ack answer.
    ack_results: Mutex<Option<Vec<AckOutcome>>>,
}

/// The two ends of a held call.
#[derive(Debug, Clone)]
struct Hold {
    method: String,
    entered: Arc<Notify>,
    proceed: Arc<Notify>,
}

impl CountingMailbox {
    pub(crate) fn new(inner: MailboxService) -> Self {
        Self {
            inner,
            methods: Mutex::new(Vec::new()),
            refuse_deposits: Mutex::new(None),
            failures: Mutex::new(Vec::new()),
            hold: Mutex::new(None),
            claim_clock_shift_secs: Mutex::new(0),
            register_ttl: Mutex::new(None),
            ack_results: Mutex::new(None),
        }
    }

    /// Every call of `method` fails with `code`, until `clear_failures`.
    pub(crate) fn fail_with(&self, method: &str, code: i32) {
        self.failures.lock().unwrap().push((method.to_string(), code));
    }

    pub(crate) fn clear_failures(&self) {
        self.failures.lock().unwrap().clear();
    }

    /// The next call of `method` waits. Returns what the test uses to see it
    /// arrive and to let it go.
    pub(crate) fn hold(&self, method: &str) -> (Arc<Notify>, Arc<Notify>) {
        let (entered, proceed) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let hold =
            Hold { method: method.to_string(), entered: entered.clone(), proceed: proceed.clone() };
        *self.hold.lock().unwrap() = Some(hold);
        (entered, proceed)
    }

    /// Moves the time a claim answer says its messages come back.
    pub(crate) fn shift_claim_clock(&self, secs: i64) {
        *self.claim_clock_shift_secs.lock().unwrap() = secs;
    }

    /// Every register answer says the queue's lifetime is `secs`.
    pub(crate) fn answer_register_ttl(&self, secs: u64) {
        *self.register_ttl.lock().unwrap() = Some(secs);
    }

    /// Every ack answer says `results`, whatever the mailbox did.
    pub(crate) fn answer_acks(&self, results: Vec<AckOutcome>) {
        *self.ack_results.lock().unwrap() = Some(results);
    }

    /// Rewrites a real answer as the test asked.
    fn rewrite(&self, method: &str, payload: &mut Value) {
        let shift = *self.claim_clock_shift_secs.lock().unwrap();
        if method == "claim" && shift != 0 {
            let at = payload["visible_again_at_secs"].as_i64().unwrap_or(0);
            payload["visible_again_at_secs"] = (at + shift).into();
        }
        if let Some(secs) = *self.register_ttl.lock().unwrap()
            && method == "register"
        {
            payload["limits"]["max_ttl_secs"] = secs.into();
        }
        if let Some(results) = self.ack_results.lock().unwrap().clone()
            && method == "ack"
        {
            payload["results"] = serde_json::to_value(results).unwrap();
        }
    }

    fn failure_of(&self, method: &str) -> Option<i32> {
        self.failures.lock().unwrap().iter().find(|(m, _)| m == method).map(|(_, code)| *code)
    }
}

impl CountingMailbox {
    pub(crate) fn refuse_deposits_with(&self, code: i32) {
        *self.refuse_deposits.lock().unwrap() = Some(code);
    }

    pub(crate) fn count(&self, method: &str) -> usize {
        self.methods.lock().unwrap().iter().filter(|m| *m == method).count()
    }
}

#[async_trait::async_trait]
impl NativeService for CountingMailbox {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        self.methods.lock().unwrap().push(invocation.method.clone());
        let held = {
            let mut hold = self.hold.lock().unwrap();
            if hold.as_ref().is_some_and(|h| h.method == invocation.method) {
                hold.take()
            } else {
                None
            }
        };
        if let Some(hold) = held {
            hold.entered.notify_one();
            hold.proceed.notified().await;
        }
        if let Some(code) = self.failure_of(&invocation.method) {
            return Err(RpcError::Custom(code, "refused".to_string(), None));
        }
        let refusal = *self.refuse_deposits.lock().unwrap();
        if let Some(code) = refusal
            && invocation.method == "deposit"
        {
            return Err(RpcError::Custom(code, "refused".to_string(), None));
        }
        let method = invocation.method.clone();
        let mut answer = self.inner.dispatch(invocation).await?;
        self.rewrite(&method, &mut answer.payload);
        Ok(answer)
    }
}

/// The receiving service member.
pub(crate) struct Receiver {
    master: Identity,
    pub(crate) instance: Identity,
    pub(crate) cert: DelegationCertificate,
    pub(crate) mailbox_key: MailboxSecretKey,
}

impl Receiver {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn service_id(&self) -> String {
        substrate::derive_did_key(&self.master.public_key())
    }

    pub(crate) fn caller(&self) -> CallerContext {
        let mut caller =
            CallerContext::verified(&self.service_id(), AuthLevel::Delegated, Vec::new());
        caller.proof = Some(CallerProof {
            pubkey_hex: hex::encode(self.instance.public_key().to_bytes()),
            delegation_json: Some(self.cert.to_json().unwrap()),
            ..Default::default()
        });
        caller
    }

    pub(crate) fn record(&self, mailbox_did: &str) -> SignedEndpointInfo {
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
