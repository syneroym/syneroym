//! The member's mailbox secret key, kept in the service's own vault under a
//! name guests cannot read.

use syneroym_core::mailbox::HostedMailbox;
use syneroym_data_db::vault_names;
use syneroym_mailbox_format::OPEN_MAX_TTL_SECS;

use super::*;

impl ControlPlaneService {
    /// Stores the member's mailbox secret in the service's vault.
    pub(super) async fn store_mailbox_key(
        &self,
        service_id: &str,
        key: &MailboxSecretKey,
    ) -> anyhow::Result<()> {
        let store = self.storage_provider.open_service_db(service_id, &self.key_store).await?;
        store.write_secret(vault_names::MAILBOX_SECRET, key.to_seed().as_slice()).await
    }

    /// Removes it. A vault that cannot be opened is only a warning here: a
    /// key left behind is never used, because a service is pulled for only
    /// while its record names a mailbox.
    ///
    /// A service without a database never stored a key, so nothing is opened
    /// or created for it: a deploy of a tcp or container service, or the
    /// undeploy of a service that never ran, leaves no new files behind.
    pub(super) async fn clear_mailbox_key(&self, service_id: &str) {
        match self.storage_provider.service_exists(service_id).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                tracing::warn!("Could not check the database of service {service_id}: {e}");
                return;
            }
        }
        let result = async {
            let store = self.storage_provider.open_service_db(service_id, &self.key_store).await?;
            store.delete_secret(vault_names::MAILBOX_SECRET).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("Failed to clear the mailbox key of service {service_id}: {e}");
        }
    }

    /// Records the mailbox the service pulls from. A redeploy to the same
    /// mailbox keeps the queue lifetime the receiver already learned.
    pub(super) async fn record_hosted_mailbox(
        &self,
        service_id: &str,
        mailbox: &str,
    ) -> anyhow::Result<()> {
        let max_ttl_secs = match self.registry.mailbox_of(service_id) {
            Some(known) if known.mailbox == mailbox => known.max_ttl_secs,
            _ => OPEN_MAX_TTL_SECS,
        };
        let hosted = HostedMailbox { mailbox: mailbox.to_string(), max_ttl_secs };
        self.registry.set_mailbox(service_id.to_string(), hosted).await
    }

    /// Forgets it. Only a warning, like the key: a stale record only makes
    /// the receiver fence longer than needed.
    pub(super) async fn forget_hosted_mailbox(&self, service_id: &str) {
        if let Err(e) = self.registry.remove_mailbox(service_id).await {
            tracing::warn!("Failed to remove the mailbox record of service {service_id}: {e}");
        }
    }

    /// The member's mailbox secret, if this service has one.
    pub async fn mailbox_key(&self, service_id: &str) -> anyhow::Result<Option<MailboxSecretKey>> {
        let store = self.storage_provider.open_service_db(service_id, &self.key_store).await?;
        let Some(bytes) = store.reveal_secret(vault_names::MAILBOX_SECRET).await? else {
            return Ok(None);
        };
        let seed = <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("stored mailbox key is not 32 bytes"))?;
        Ok(Some(MailboxSecretKey::from_seed(seed)))
    }
}
