//! Shared test fixtures for the mailbox crate.

use std::{collections::HashMap, sync::Mutex};

use anyhow::anyhow;
use syneroym_core::{
    dht_registry::{
        EndpointInfo, EndpointMailbox, EndpointType, MasterAnchorPayload, MasterAnchorResolver,
        SignedEndpointInfo,
    },
    mailbox::QueueLimits,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_mailbox_format::{
    MailboxPublicKey, MailboxSecretKey, MessageContent, Recipient, Sealed, Sender, seal,
};
use syneroym_rpc::{AuthLevel, CallerContext, CallerProof, SessionContext};

use crate::{caller::Instance, limits::Limits};

pub(crate) struct Member {
    pub master: Identity,
    pub instance: Identity,
    pub cert: DelegationCertificate,
    pub mailbox: MailboxSecretKey,
    pub mailbox_public: MailboxPublicKey,
}

impl Member {
    pub(crate) fn new(lifetime_secs: u64) -> Self {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            lifetime_secs,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mailbox = MailboxSecretKey::generate().unwrap();
        let mailbox_public = mailbox.public_key();
        Self { master, instance, cert, mailbox, mailbox_public }
    }

    pub(crate) fn service_id(&self) -> String {
        substrate::derive_did_key(&self.master.public_key())
    }

    pub(crate) fn caller(&self) -> CallerContext {
        CallerContext {
            caller_did: self.service_id(),
            app_instance: None,
            session: SessionContext {
                subject_did: self.service_id(),
                capabilities: Vec::new(),
                ..Default::default()
            },

            auth: AuthLevel::Delegated,
            proof: Some(CallerProof {
                pubkey_hex: hex::encode(self.instance.public_key().to_bytes()),
                delegation_json: Some(self.cert.to_json().unwrap()),
            }),
        }
    }

    pub(crate) fn instance(&self) -> Instance {
        Instance {
            master_did: self.service_id(),
            instance_did: self.cert.temporary_did.clone(),
            issued_at_secs: self.cert.issued_at_secs,
        }
    }
}

pub(crate) fn record(member: &Member, mailbox_did: &str) -> SignedEndpointInfo {
    let now = syneroym_passport::now_secs();
    let info = EndpointInfo {
        service_id: member.service_id(),
        substrate_id: member.service_id(),
        endpoint_type: EndpointType::Service,
        mechanisms: Vec::new(),
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: now + 30 * 24 * 3600,
        generation: 0,
        mailbox: Some(EndpointMailbox {
            service_id: mailbox_did.to_string(),
            key: member.mailbox_public,
        }),
    };
    info.sign(&member.master).unwrap()
}

pub(crate) fn sealed_deposit(from: &Member, to: &Member, ttl: u64, now: u64) -> Sealed {
    let content = MessageContent {
        interface: "marker-interface".to_string(),
        method: "marker-method".to_string(),
        params: serde_json::json!({"test": "data"}),
        idempotency_key: "test-idem-key".to_string(),
    };
    let sender = Sender { instance: &from.instance, certificate: &from.cert };
    let to_service_id = to.service_id();
    let recipient = Recipient { service_id: &to_service_id, mailbox_key: &to.mailbox_public };
    seal(&content, recipient, sender, ttl, now).unwrap()
}

#[derive(Debug, Default)]
pub(crate) struct Anchors {
    anchors: Mutex<HashMap<String, MasterAnchorPayload>>,
    fail: Mutex<bool>,
}

impl Anchors {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn publish(&self, master_did: &str) {
        let mut map = self.anchors.lock().unwrap();
        map.entry(master_did.to_string()).or_insert_with(|| MasterAnchorPayload {
            revoked_keys: Vec::new(),
            ..Default::default()
        });
    }

    pub(crate) fn revoke(&self, master_did: &str, key_did: &str) {
        let mut map = self.anchors.lock().unwrap();
        let payload = map.entry(master_did.to_string()).or_insert_with(|| MasterAnchorPayload {
            revoked_keys: Vec::new(),
            ..Default::default()
        });
        if !payload.revoked_keys.contains(&key_did.to_string()) {
            payload.revoked_keys.push(key_did.to_string());
        }
    }

    pub(crate) fn set_fail(&self, fail: bool) {
        *self.fail.lock().unwrap() = fail;
    }
}

#[async_trait::async_trait]
impl MasterAnchorResolver for Anchors {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        if *self.fail.lock().unwrap() {
            return Err(anyhow!("simulated resolver failure"));
        }
        let map = self.anchors.lock().unwrap();
        map.get(master_id).cloned().ok_or_else(|| anyhow!("no anchor for {master_id}"))
    }
}

pub(crate) fn small_limits() -> Limits {
    Limits {
        queue: QueueLimits { max_bytes: 200 * 1024, max_messages: 3, max_ttl_secs: 604_800 },
        receipt_keep_secs: 100,
        visibility_secs: 10,
        queue_lifetime_secs: 1_000,
        max_total_bytes: 10 * 1024 * 1024,
        max_queues: 4,
    }
}

pub(crate) fn store_with(limits: Limits) -> crate::store::MailboxStore {
    crate::store::MailboxStore::open_in_memory(limits).unwrap()
}
