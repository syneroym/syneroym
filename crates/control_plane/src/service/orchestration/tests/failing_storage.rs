//! A registry store that fails `save` for one interface, so a test can fail
//! `EndpointRegistry::register` on purpose.

use anyhow::Result;
use syneroym_core::{
    local_registry::SubstrateEndpoint,
    mailbox::HostedMailbox,
    storage::{AppInstanceManagement, EndpointStorage, MockStorage},
};

/// Wraps `MockStorage`, failing `save` for one specific interface name --
/// lets a test deterministically fail `EndpointRegistry::register`
/// (used by `register_wasm_endpoints`/`deploy_container_service`'s
/// registration loop) without needing a real network/podman failure.
pub(super) struct FailingEndpointStorage {
    pub(super) inner: MockStorage,
    pub(super) fail_interface: String,
}

#[async_trait::async_trait]
impl EndpointStorage for FailingEndpointStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        self.inner.load_all().await
    }
    async fn save(
        &self,
        service_id: &str,
        interface_name: &str,
        endpoint: &SubstrateEndpoint,
    ) -> Result<()> {
        if interface_name == self.fail_interface {
            anyhow::bail!("simulated registry storage failure for {interface_name}");
        }
        self.inner.save(service_id, interface_name, endpoint).await
    }
    async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
        self.inner.remove(service_id, interface_name).await
    }
    async fn load_all_owners(&self) -> Result<Vec<(String, String)>> {
        self.inner.load_all_owners().await
    }
    async fn save_owner(&self, service_id: &str, owner_did: &str) -> Result<()> {
        self.inner.save_owner(service_id, owner_did).await
    }
    async fn remove_owner(&self, service_id: &str) -> Result<()> {
        self.inner.remove_owner(service_id).await
    }
    async fn load_all_mailboxes(&self) -> Result<Vec<(String, HostedMailbox)>> {
        self.inner.load_all_mailboxes().await
    }
    async fn save_mailbox(&self, service_id: &str, mailbox: &HostedMailbox) -> Result<()> {
        self.inner.save_mailbox(service_id, mailbox).await
    }
    async fn remove_mailbox(&self, service_id: &str) -> Result<()> {
        self.inner.remove_mailbox(service_id).await
    }
    async fn load_all_certs(&self) -> Result<Vec<(String, String)>> {
        self.inner.load_all_certs().await
    }
    async fn save_cert(&self, service_id: &str, certificate_json: &str) -> Result<()> {
        self.inner.save_cert(service_id, certificate_json).await
    }
    async fn remove_cert(&self, service_id: &str) -> Result<()> {
        self.inner.remove_cert(service_id).await
    }
    async fn load_all_deploy_facts(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        self.inner.load_all_deploy_facts().await
    }
    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
        manifest_hash: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<()> {
        self.inner
            .save_deploy_facts(
                service_id,
                service_type,
                health_check_json,
                manifest_hash,
                visibility,
            )
            .await
    }
    async fn remove_deploy_facts(&self, service_id: &str) -> Result<()> {
        self.inner.remove_deploy_facts(service_id).await
    }
    async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>> {
        self.inner.load_all_app_contexts().await
    }
    async fn save_app_context(
        &self,
        service_id: &str,
        app_instance_id: &str,
        service_name: &str,
    ) -> Result<()> {
        self.inner.save_app_context(service_id, app_instance_id, service_name).await
    }
    async fn remove_app_context(&self, service_id: &str) -> Result<()> {
        self.inner.remove_app_context(service_id).await
    }
    async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>> {
        self.inner.load_all_bindings().await
    }
    async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        topology_entry_json: &str,
    ) -> Result<()> {
        self.inner
            .save_binding(service_id, app_instance_id, dependency_name, topology_entry_json)
            .await
    }
    async fn load_binding(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> Result<Option<String>> {
        self.inner.load_binding(service_id, dependency_name).await
    }
    async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>> {
        self.inner.load_bindings_for(service_id).await
    }
    async fn load_all_app_instance_management(
        &self,
    ) -> Result<Vec<(String, AppInstanceManagement)>> {
        self.inner.load_all_app_instance_management().await
    }
    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> Result<()> {
        self.inner.save_app_instance_management(app_instance_id, management).await
    }
    async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()> {
        self.inner.remove_app_instance_management(app_instance_id).await
    }
}
