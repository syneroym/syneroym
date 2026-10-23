//! A test double that plays a receiver at a mailbox: it registers, claims,
//! opens what a sender deposited, and answers with a receipt it signs with
//! its own instance key.
//!
//! The receiver's service is usually also deployed on a node (and then
//! stopped), so a sender has a real record to resolve. Its mailbox key is
//! the one the deploy derives from the master, so a deploy and this double
//! agree on one key.

#![allow(clippy::cognitive_complexity, dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use syneroym_core::{
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointMailbox, EndpointType,
        RegistryClient, SignedEndpointInfo,
    },
    mailbox::{
        AckAnswer, AckOutcome, ClaimAnswer, MAILBOX_INTERFACE, METHOD_ACK, METHOD_CLAIM,
        METHOD_REGISTER,
    },
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_mailbox_format::{
    MailboxSecretKey, OpenPolicy, OpenedMessage, Receipt, RejectCode, open,
};
use syneroym_rpc::JsonRpcError;
use syneroym_sdk::{
    Publication, SyneroymClient,
    deploy::{Visibility, certify_instance, member_registry_record},
};

use super::SubstrateNode;

pub fn did_of(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

pub fn not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

/// A client that presents `instance` and its certificate to `node`.
pub async fn instance_client(
    node: &SubstrateNode,
    instance: &Identity,
    cert: &DelegationCertificate,
) -> SyneroymClient {
    let mut client =
        node.client_as(Identity::from_bytes(&instance.to_bytes())).with_delegation(cert.clone());
    client.connect().await.expect("failed to connect instance client");
    client
}

/// One mailbox method, with the JSON-RPC error split out as `(code, data)`.
pub async fn call(
    client: &SyneroymClient,
    method: &str,
    params: Value,
) -> Result<Value, (i32, Value)> {
    match client.request(MAILBOX_INTERFACE, method, params).await {
        Ok(res) => Ok(res.result),
        Err(e) => match e.downcast_ref::<JsonRpcError>() {
            Some(rpc_err) => Err((rpc_err.code, rpc_err.data.clone().unwrap_or(Value::Null))),
            None => panic!("unexpected transport or parse error: {e:?}"),
        },
    }
}

pub struct MailboxReceiver {
    pub master: Identity,
    pub instance: Identity,
    pub cert: DelegationCertificate,
    pub mailbox_key: MailboxSecretKey,
}

impl MailboxReceiver {
    pub fn new() -> Self {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mailbox_key = MailboxSecretKey::derive_from_master(&master);
        Self { master, instance, cert, mailbox_key }
    }

    pub fn service_id(&self) -> String {
        did_of(&self.master)
    }

    /// The mailbox this receiver names in its record.
    pub fn route(&self, mailbox_node_did: &str) -> EndpointMailbox {
        EndpointMailbox {
            service_id: mailbox_node_did.to_string(),
            key: self.mailbox_key.public_key(),
        }
    }

    /// The signed record the receiver registers at its mailbox.
    pub fn record(&self, host_node_did: &str, mailbox_node_did: &str) -> SignedEndpointInfo {
        EndpointInfo {
            service_id: self.service_id(),
            substrate_id: host_node_did.to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: not_after(),
            generation: 0,
            mailbox: Some(self.route(mailbox_node_did)),
        }
        .sign(&self.master)
        .unwrap()
    }

    /// Deploys `wasm` on `node` as this receiver's service: public, with a
    /// mailbox in its record and the mailbox key the master derives. The
    /// master's anchor is published first, because the mailbox and the
    /// sender both read it.
    pub async fn deploy(
        &self,
        node: &SubstrateNode,
        mailbox_node_did: &str,
        interface: &str,
        wasm: Vec<u8>,
    ) -> String {
        self.deploy_with_methods(node, mailbox_node_did, interface, wasm, &[]).await
    }

    /// [`Self::deploy`], with the guest methods that accept delivery from the
    /// mailbox (`<interface>#<method>`).
    pub async fn deploy_with_methods(
        &self,
        node: &SubstrateNode,
        mailbox_node_did: &str,
        interface: &str,
        wasm: Vec<u8>,
        methods: &[&str],
    ) -> String {
        let id = self.service_id();
        RegistryClient::new(false, Some(node.registry_url().to_string()))
            .publish_master_anchor(&id, vec![], None, &self.master, true)
            .await
            .expect("failed to publish the receiver master's anchor");
        let cert = certify_instance(&node.substrate_client, &self.master, &id, 168)
            .await
            .expect("failed to certify the receiver's instance");
        let record = member_registry_record(
            Visibility::Public,
            &id,
            node.did(),
            &self.master,
            not_after(),
            Some(self.route(mailbox_node_did)),
        )
        .unwrap()
        .unwrap();
        let publication = Publication::Public {
            record: serde_json::from_str(&record).unwrap(),
            mailbox_key: Some(self.mailbox_key.clone()),
            mailbox_methods: methods.iter().map(|m| m.parse().unwrap()).collect(),
        };
        node.substrate_client
            .deploy_svc_wasm(id.clone(), vec![interface.to_string()], wasm, publication, Some(cert))
            .await
            .expect("the receiver's service must deploy");
        id
    }

    pub async fn client(&self, mailbox_node: &SubstrateNode) -> SyneroymClient {
        instance_client(mailbox_node, &self.instance, &self.cert).await
    }

    /// Makes the queue the senders deposit into.
    pub async fn register(
        &self,
        client: &SyneroymClient,
        host_node_did: &str,
        mailbox_node_did: &str,
    ) {
        let record = self.record(host_node_did, mailbox_node_did);
        call(client, METHOD_REGISTER, json!({ "record": record }))
            .await
            .expect("the mailbox must accept the receiver's record");
    }

    /// Claims everything that waits, opens each message and acks it with
    /// the receipt `receipt_for` makes. Returns how many messages it acked.
    pub async fn claim_and_ack(
        &self,
        client: &SyneroymClient,
        receipt_for: impl Fn(&OpenedMessage, &Self) -> Receipt,
    ) -> usize {
        let claimed = call(client, METHOD_CLAIM, json!({ "max": 10 })).await.expect("claim failed");
        let claimed: ClaimAnswer = serde_json::from_value(claimed).unwrap();
        let mut acks = Vec::new();
        for message in &claimed.messages {
            let opened = open(
                &self.mailbox_key,
                &self.service_id(),
                &message.deposit_id,
                &message.sealed,
                OpenPolicy::default(),
                now_secs(),
            )
            .expect("the message must open with the receiver's mailbox key");
            acks.push(json!({
                "sender": opened.sender_master_did(),
                "deposit_id": message.deposit_id,
                "receipt": receipt_for(&opened, self),
            }));
        }
        if acks.is_empty() {
            return 0;
        }
        let count = acks.len();
        let acked = call(client, METHOD_ACK, json!({ "acks": acks })).await.expect("ack failed");
        let acked: AckAnswer = serde_json::from_value(acked).unwrap();
        assert!(acked.results.iter().all(|r| *r == AckOutcome::Stored), "{acked:?}");
        count
    }

    pub async fn deliver_all(&self, client: &SyneroymClient) -> usize {
        self.claim_and_ack(client, |opened, receiver| {
            opened.sign_delivered(&receiver.instance, &receiver.cert, now_secs()).unwrap()
        })
        .await
    }

    pub async fn reject_all_as_not_eligible(&self, client: &SyneroymClient) -> usize {
        self.claim_and_ack(client, |opened, receiver| {
            opened
                .sign_rejection(
                    &receiver.instance,
                    &receiver.cert,
                    RejectCode::NotEligible,
                    now_secs(),
                )
                .unwrap()
        })
        .await
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}
