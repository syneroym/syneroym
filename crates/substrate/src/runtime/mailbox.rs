//! The mailbox role: registers the native service and builds its worker.

use std::sync::Arc;

use anyhow::bail;
use syneroym_control_plane::MAILBOX_RESERVED_SERVICE_ID;
#[cfg(feature = "mailbox")]
use syneroym_core::dht_registry::{MasterAnchorResolver, RegistryClient};
use syneroym_core::{
    config::SubstrateConfig,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    mailbox::MAILBOX_INTERFACE,
};
#[cfg(feature = "mailbox")]
use syneroym_mailbox::MailboxService;
#[cfg(feature = "mailbox")]
use syneroym_rpc::NativeService;

use super::handles::SharedNodeHandles;

#[cfg(test)]
mod tests;

#[cfg(feature = "mailbox")]
pub(super) struct MailboxParts {
    pub(super) worker: syneroym_mailbox::MailboxWorker,
}
#[cfg(not(feature = "mailbox"))]
pub(super) struct MailboxParts;

/// Registers the mailbox on interface `mailbox` of this node, or removes a
/// registration left from an earlier boot when the role is off.
pub(super) async fn init_mailbox(
    config: &SubstrateConfig,
    node_did: &str,
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Option<MailboxParts>> {
    #[cfg(feature = "mailbox")]
    {
        let Some(role) = &config.roles.mailbox else {
            clean_stale_mailbox_registration(endpoint_registry, node_did).await?;
            return Ok(None);
        };
        require_registry(config)?;
        // The same client the router builds (route_handler.rs), so the
        // revocation check reads the anchors the handshake reads.
        let client = Arc::new(RegistryClient::new(
            config.substrate.enable_bep0044_dht,
            config.substrate.registry_url.clone(),
        ));
        let service = Arc::new(MailboxService::open(
            role,
            &config.storage.db_dir,
            node_did,
            client.clone() as Arc<dyn MasterAnchorResolver>,
        )?);
        endpoint_registry
            .register(
                node_did.to_string(),
                MAILBOX_INTERFACE.to_string(),
                SubstrateEndpoint::NativeHostChannel {
                    service_id: MAILBOX_RESERVED_SERVICE_ID.to_string(),
                },
            )
            .await?;
        shared.native_dispatch().insert(
            MAILBOX_RESERVED_SERVICE_ID.to_string(),
            service.clone() as Arc<dyn NativeService>,
        );
        Ok(Some(MailboxParts { worker: service.worker(client) }))
    }

    #[cfg(not(feature = "mailbox"))]
    {
        let _ = shared;
        if config.roles.mailbox.is_some() {
            bail!(
                "[roles.mailbox] is configured but this binary was built without the `mailbox` \
                 feature"
            );
        }
        clean_stale_mailbox_registration(endpoint_registry, node_did).await?;
        Ok(None)
    }
}

async fn clean_stale_mailbox_registration(
    endpoint_registry: &EndpointRegistry,
    node_did: &str,
) -> anyhow::Result<()> {
    if endpoint_registry.lookup_by_service(node_did).iter().any(|(i, _)| i == MAILBOX_INTERFACE) {
        endpoint_registry.remove(node_did, MAILBOX_INTERFACE).await?;
    }
    Ok(())
}

#[cfg(feature = "mailbox")]
pub(crate) fn require_registry(config: &SubstrateConfig) -> anyhow::Result<()> {
    if config.substrate.registry_url.is_none() && !config.substrate.enable_bep0044_dht {
        bail!(
            "[roles.mailbox] needs a registry: set substrate.registry_url or \
             substrate.enable_bep0044_dht. Without one the mailbox cannot check revocation, so it \
             could serve no receiver, and it cannot republish their service records"
        );
    }
    Ok(())
}
