//! The member's mailbox secret key, kept in the service's own vault under a
//! name guests cannot read.

use syneroym_data_db::vault_names;

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
    pub(super) async fn clear_mailbox_key(&self, service_id: &str) {
        let result = async {
            let store = self.storage_provider.open_service_db(service_id, &self.key_store).await?;
            store.delete_secret(vault_names::MAILBOX_SECRET).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("Failed to clear the mailbox key of service {service_id}: {e}");
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
