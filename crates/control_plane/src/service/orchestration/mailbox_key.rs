//! The member's mailbox secret key, kept in the service's own vault under a
//! name guests cannot read.

use std::io::ErrorKind;

use syneroym_core::{dht_registry::SignedEndpointInfo, mailbox::HostedMailbox};
use syneroym_data_db::vault_names;
use syneroym_mailbox_format::OPEN_MAX_TTL_SECS;
use tokio::{fs as async_fs, time};

use super::*;
use crate::mailbox_pull::MailboxPullControl;

/// How long removing an old queue may hold up an undeploy or deploy: the
/// wait for a `register` in flight, then the `unregister`. A remote mailbox
/// is looked up first.
pub(super) const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// Stores the owner's child token for the mailbox passport, or clears
    /// the one an earlier deploy stored when this deploy has none.
    pub(super) async fn store_mailbox_passport(
        &self,
        service_id: &str,
        child_token: Option<&str>,
    ) -> anyhow::Result<()> {
        let store = self.storage_provider.open_service_db(service_id, &self.key_store).await?;
        match child_token {
            Some(token) => {
                store.write_secret(vault_names::MAILBOX_PASSPORT, token.as_bytes()).await
            }
            None => store.delete_secret(vault_names::MAILBOX_PASSPORT).await,
        }
    }

    /// Removes the key and the passport. A vault that cannot be opened is only
    /// a warning here: a key left behind is never used, because a service
    /// is pulled for only while its record names a mailbox.
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
            store.delete_secret(vault_names::MAILBOX_SECRET).await?;
            store.delete_secret(vault_names::MAILBOX_PASSPORT).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("Failed to clear the mailbox key of service {service_id}: {e}");
        }
    }

    /// Records the mailbox the service pulls from, and the guest methods it
    /// accepts from it. A redeploy to the same mailbox keeps the queue
    /// lifetime the receiver already learned.
    pub(super) async fn record_hosted_mailbox(
        &self,
        service_id: &str,
        mailbox: &str,
        methods: Vec<String>,
    ) -> anyhow::Result<()> {
        let max_ttl_secs = match self.registry.mailbox_of(service_id) {
            Some(known) if known.mailbox == mailbox => known.max_ttl_secs,
            _ => OPEN_MAX_TTL_SECS,
        };
        let hosted = HostedMailbox { mailbox: mailbox.to_string(), max_ttl_secs, methods };
        self.registry.set_mailbox(service_id.to_string(), hosted).await
    }

    /// Forgets it. Only a warning, like the key: a stale record only makes
    /// the receiver fence longer than needed.
    pub(super) async fn forget_hosted_mailbox(&self, service_id: &str) {
        if let Err(e) = self.registry.remove_mailbox(service_id).await {
            tracing::warn!("Failed to remove the mailbox record of service {service_id}: {e}");
        }
    }

    /// The record the deployer signed for `service_id`, as stored at deploy,
    /// even past its `not_after`. `None` when the service has none. The
    /// mailbox receiver registers with it; it is not verified here.
    pub async fn deploy_record(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<SignedEndpointInfo>> {
        let path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        match async_fs::read_to_string(&path).await {
            Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Removes `service_id`'s queue at `mailbox`, while its instance
    /// certificate is still installed: a remote mailbox only admits the call
    /// under that instance. Warns and goes on when it cannot.
    pub(super) async fn release_mailbox_queue(&self, service_id: &str, mailbox: &str) {
        let Some(puller) = self.mailbox_pull.get().and_then(Weak::upgrade) else { return };
        release_within(RELEASE_TIMEOUT, puller.as_ref(), service_id, mailbox).await;
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

    /// The owner's child token for this service's mailbox passport, if it
    /// has one.
    pub async fn mailbox_passport(&self, service_id: &str) -> anyhow::Result<Option<String>> {
        let store = self.storage_provider.open_service_db(service_id, &self.key_store).await?;
        let Some(bytes) = store.reveal_secret(vault_names::MAILBOX_PASSPORT).await? else {
            return Ok(None);
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| anyhow::anyhow!("stored mailbox passport is not UTF-8 text"))
    }
}

/// Asks `puller` to remove the queue and gives up after `limit`, with a
/// warning: an undeploy or deploy never waits longer than that for it.
pub(super) async fn release_within(
    limit: Duration,
    puller: &dyn MailboxPullControl,
    service_id: &str,
    mailbox: &str,
) {
    if time::timeout(limit, puller.release(service_id, mailbox)).await.is_err() {
        tracing::warn!(
            "Removing the old mailbox queue of {service_id} timed out; it ends with its lifetime"
        );
    }
}
