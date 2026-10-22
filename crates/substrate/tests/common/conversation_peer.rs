//! One conversation node with the fixture guest deployed on it, shared by
//! the cross-node conversation tests.

#![allow(clippy::cognitive_complexity, dead_code)]

use serde_json::Value;
use syneroym_conversation::ids::derive_conversation_id;
use syneroym_core::config::AppSandboxRole;
use syneroym_identity::Identity;

use super::{
    SubstrateNode,
    conversation_fixture::{
        Deploy, deploy_fixture, history_messages, publish_endpoint, publish_master_anchor,
        publish_node_record,
    },
};

/// One conversation node with the fixture guest deployed on it.
pub struct Peer {
    pub node: SubstrateNode,
    pub master: Identity,
    pub did: String,
}

impl Peer {
    /// Boots a node with a fast delivery tick and deploys the fixture as a
    /// certified service. `shared_registry` is the first node's registry.
    pub async fn boot(shared_registry: Option<&str>, role: AppSandboxRole, wasm: &[u8]) -> Self {
        Self::boot_as(Identity::generate().unwrap(), shared_registry, role, wasm, Deploy::Certified)
            .await
    }

    pub async fn boot_as(
        master: Identity,
        shared_registry: Option<&str>,
        role: AppSandboxRole,
        wasm: &[u8],
        mode: Deploy,
    ) -> Self {
        let mut builder = SubstrateNode::builder()
            .owner(&Identity::generate().unwrap())
            .inject_kek_bytes([0xcd; 32])
            .configure(move |c| c.roles.app_sandbox = Some(role.clone()));
        if let Some(url) = shared_registry {
            builder = builder.shared_registry(url);
        }
        let mut node = builder.boot().await;
        let did = deploy_fixture(&mut node, &master, wasm.to_vec(), mode).await;
        Self { node, master, did }
    }

    pub fn registry(&self) -> &str {
        self.node.registry_url()
    }

    /// This peer's own view of its conversation with `other`.
    pub fn conversation_with(&self, other: &Peer) -> String {
        derive_conversation_id(&self.did, &other.did)
    }

    /// Makes `other` resolvable through this peer's own registry, for a peer
    /// that does not share the others' registry.
    pub async fn introduce(&self, other: &Peer) {
        let mechanisms = other.node.substrate_client.lookup().await.unwrap().info.mechanisms;
        publish_endpoint(&other.did, other.node.did(), mechanisms, &other.master, self.registry())
            .await;
        publish_master_anchor(&other.did, &other.master, self.registry()).await;
        publish_node_record(&other.node, self.registry()).await;
    }

    pub async fn history_with(&self, other: &Peer) -> Vec<Value> {
        history_messages(&self.node, &self.did, &self.conversation_with(other)).await
    }

    pub async fn teardown(self) {
        self.node.teardown().await;
    }
}
