//! Tier 1 of the logical discovery overlay (ADR-0022 §2): the registry
//! record that maps an app instance's own master DID to the substrate
//! supervising it -- "which supervisor holds this app". A sibling of
//! `anchors::AnchorWriter`, not a variant of it: the app master delegates
//! nothing and gets no revocation anchor of its own (ADR-0022 §3, the
//! app-master-has-no-anchor finding), so this writer only ever publishes
//! an `EndpointInfo`, never a `MasterAnchorPayload`.

use std::{fmt, sync::Arc, time::SystemTime};

use syneroym_core::dht_registry::{
    DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, RegistryClient, SignedEndpointInfo,
};
use syneroym_identity::substrate;

use crate::keys::{self, MasterVault, VaultError};

/// The one write this slice issues: publish or refresh an app instance's
/// Tier-1 record. Behind a trait for the same reason `AnchorWriter` is --
/// the resident loop's own path needs it testable with no live HTTP
/// registry.
#[async_trait::async_trait]
pub trait Tier1Writer: fmt::Debug + Send + Sync {
    async fn publish(&self, record: &SignedEndpointInfo) -> Result<(), String>;
}

/// The production writer: this node's configured HTTP registry (and, when
/// enabled, the DHT), reached through the same `RegistryClient::register`
/// every other endpoint-record publisher on this host uses.
#[derive(Debug)]
pub struct RegistryTier1Writer {
    client: RegistryClient,
}

impl RegistryTier1Writer {
    #[must_use]
    pub fn new(enable_dht: bool, registry_url: String) -> Self {
        Self { client: RegistryClient::new(enable_dht, Some(registry_url)) }
    }

    /// `None` when this node has no `substrate.registry_url` configured
    /// (ADR-0022 §2, matrix row 11): a single-node deployment does not use
    /// cross-app discovery and must not be broken to enable a feature it
    /// does not use, so the supervisor holds no writer rather than one
    /// that quietly does nothing.
    #[must_use]
    pub fn from_registry_url(
        enable_dht: bool,
        registry_url: Option<&str>,
    ) -> Option<Arc<dyn Tier1Writer>> {
        registry_url
            .map(|url| Arc::new(Self::new(enable_dht, url.to_string())) as Arc<dyn Tier1Writer>)
    }
}

#[async_trait::async_trait]
impl Tier1Writer for RegistryTier1Writer {
    async fn publish(&self, record: &SignedEndpointInfo) -> Result<(), String> {
        // `sync_dht: false`, the same reasoning `refresh_master_anchor`
        // gives: the HTTP publish is the operative guarantee, and blocking
        // a resident-loop pass on a real mainline-DHT publish would trade
        // a synchronous guarantee this call does not need.
        self.client.register(record, false).await.map_err(|e| e.to_string())
    }
}

/// Builds and signs this instance's Tier-1 record, without ever contacting
/// a writer: a locked vault or a missing app master both stop here, before
/// any network call is attempted.
///
/// The convention for the three fields `EndpointInfo` has no natural
/// meaning for on an app-identity record: `endpoint_type`/`mechanisms`
/// name the *supervising node*, not a way to connect to the app itself --
/// nobody connects to an app DID, and the supervising node's own record
/// (resolved separately, through the ordinary Tier-3 lookup on
/// `substrate_id`) already carries the real mechanisms. `nickname` is the
/// app instance's human name. `is_private` is `false` until a per-app
/// visibility declaration exists (ADR-0022 §5's visibility surface is
/// per-logical-service, not yet modeled at the whole-app level this record
/// answers for).
pub async fn sign_tier1_record(
    vault: &MasterVault,
    app_instance_id: &str,
    supervisor_did: &str,
    generation: u64,
) -> Result<SignedEndpointInfo, VaultError> {
    let master = keys::existing_app_master(vault, app_instance_id).await?.ok_or_else(|| {
        VaultError::Storage(anyhow::anyhow!(
            "app master DID is recorded on this instance's row but not found in this supervisor's \
             vault"
        ))
    })?;
    let not_after = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
    let info = EndpointInfo {
        service_id: substrate::derive_did_key(&master.public_key()),
        substrate_id: supervisor_did.to_string(),
        endpoint_type: EndpointType::Substrate,
        mechanisms: vec![],
        nickname: Some(app_instance_id.to_string()),
        is_private: false,
        ttl: None,
        not_after,
        generation,
    };
    info.sign(&master).map_err(VaultError::Storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::MasterKind;

    async fn vault(dir: &std::path::Path) -> MasterVault {
        let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> =
            Arc::new(syneroym_data_db::SqliteStorageProvider::new(dir.join("db"), false).unwrap());
        let key_store = Arc::new(syneroym_data_keystore::KeyStore::new());
        key_store.inject_kek([7u8; 32]).unwrap();
        MasterVault::new(storage_provider, key_store, "supervisor".to_string(), dir.join("backups"))
    }

    /// D-S1-6's other half, exercised directly against the signer rather
    /// than through the pass-tick wrapper: no app master in the vault
    /// means no record, and nothing gets minted to fill the gap.
    #[tokio::test]
    async fn no_app_master_in_the_vault_fails_without_minting_one() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path()).await;

        let err = sign_tier1_record(&v, "inst-1", "did:key:zNode", 0).await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)), "{err:?}");
        assert!(v.get("app-inst-1").await.unwrap().is_none(), "signing must never mint");
    }

    /// The record names the supervising node in `substrate_id` -- pinned
    /// so a second publisher of this record cannot diverge on the
    /// convention.
    #[tokio::test]
    async fn the_record_names_the_supervising_node_in_substrate_id() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path()).await;
        v.get_or_mint("app-inst-1", MasterKind::App).await.unwrap();

        let signed = sign_tier1_record(&v, "inst-1", "did:key:zSupervisorNode", 3).await.unwrap();
        assert_eq!(signed.info.substrate_id, "did:key:zSupervisorNode");
        assert_eq!(signed.info.nickname, Some("inst-1".to_string()));
        assert_eq!(signed.info.endpoint_type, EndpointType::Substrate);
        assert_eq!(signed.info.generation, 3);
        assert!(signed.verify().is_ok());
    }

    /// The record is signed with the app master, not any other key this
    /// vault might hold -- `service_id` must resolve to the exact DID
    /// `app_master` minted, or Tier 1 answers under the wrong identity.
    #[tokio::test]
    async fn the_record_is_signed_with_the_app_master_and_verifies_against_the_app_did() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path()).await;
        let (app_did, _) = keys::app_master(&v, "inst-1").await.unwrap();

        let signed = sign_tier1_record(&v, "inst-1", "did:key:zNode", 0).await.unwrap();
        assert_eq!(signed.info.service_id, app_did);
        assert!(signed.verify().is_ok());
    }

    /// D-C-2's own local pin: a locked vault stops the signer before any
    /// writer would be reached -- there is no writer parameter here at
    /// all, so a failure at this layer can never touch the registry.
    #[tokio::test]
    async fn a_locked_vault_fails_the_refresh_without_touching_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> = Arc::new(
            syneroym_data_db::SqliteStorageProvider::new(dir.path().join("db"), true).unwrap(),
        );
        let key_store = Arc::new(syneroym_data_keystore::KeyStore::new());
        // Never injected: this vault is genuinely locked, not merely
        // unlocked-and-empty.
        let v = MasterVault::new(
            storage_provider,
            key_store,
            "supervisor".to_string(),
            dir.path().join("backups"),
        );

        let err = sign_tier1_record(&v, "inst-1", "did:key:zNode", 0).await.unwrap_err();
        assert!(matches!(err, VaultError::Locked), "{err:?}");
    }
}
