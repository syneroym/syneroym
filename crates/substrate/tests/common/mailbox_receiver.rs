//! One receiver member of a mailbox test: the master, the mailbox key it
//! derives, the service deployed on a node with a mailbox in its record, and
//! a double that registers the queue itself.
//!
//! The deploy is the real thing: the node's own mailbox receiver registers
//! and pulls. The double is only for a receiver whose node no longer hosts
//! the service, where the queue must be made by hand.

#![allow(clippy::cognitive_complexity, dead_code)]

use serde_json::json;
use syneroym_core::{
    dht_registry::{
        EndpointInfo, EndpointMailbox, EndpointType, RegistryClient, SignedEndpointInfo,
    },
    mailbox::METHOD_REGISTER,
};
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_mailbox_format::MailboxSecretKey;
use syneroym_sdk::{
    Publication, SyneroymClient,
    deploy::{Visibility, certify_instance, member_registry_record},
};

use super::{
    SubstrateNode,
    mailbox_client::{call, did_of, instance_client, not_after},
};

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
}
