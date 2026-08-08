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
/// every other endpoint-record publisher on this host uses. Takes an
/// already-built `Arc<RegistryClient>` rather than building its own -- see
/// `RegistryAnchorWriter`'s identical doc for why: one client, shared with
/// the anchor writer, not a second pkarr client and socket per supervisor.
#[derive(Debug)]
pub struct RegistryTier1Writer {
    client: Arc<RegistryClient>,
}

impl RegistryTier1Writer {
    #[must_use]
    pub fn new(client: Arc<RegistryClient>) -> Self {
        Self { client }
    }

    /// `None` when this node has no `substrate.registry_url` configured
    /// (ADR-0022 §2, matrix row 11): a single-node deployment does not use
    /// cross-app discovery and must not be broken to enable a feature it
    /// does not use, so the supervisor holds no writer rather than one
    /// that quietly does nothing.
    #[must_use]
    pub fn from_registry_client(
        client: Option<Arc<RegistryClient>>,
    ) -> Option<Arc<dyn Tier1Writer>> {
        client.map(|c| Arc::new(Self::new(c)) as Arc<dyn Tier1Writer>)
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

/// `sign_tier1_record`'s own failure, distinct from a bare `VaultError` so a
/// caller can tell "could not read the key" apart from "read a key, but not
/// the one this instance is supposed to have" without matching on error text
/// (the fragility this codebase already flags elsewhere for exactly that
/// pattern).
#[derive(Debug)]
pub enum Tier1SignError {
    Vault(VaultError),
    /// The vault's `app-<id>` key does not derive the DID
    /// `desired_state.app_master_did` recorded at the last `adopt` --
    /// reachable when an `import-master` lands without a following `adopt`
    /// (A7 §0.5, task.md's own S1 note). Publishing here would name a DID
    /// nobody looks up while `status` reports a different one; this must
    /// refuse rather than silently pick one.
    IdentityMismatch {
        expected: String,
        actual: String,
    },
}

impl fmt::Display for Tier1SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vault(e) => write!(f, "{e}"),
            Self::IdentityMismatch { expected, actual } => write!(
                f,
                "this instance's row records app master {expected}, but the vault's app-<id> key \
                 derives {actual} -- run `import-master` for the correct key, then `adopt`, \
                 before this record is published"
            ),
        }
    }
}

impl From<VaultError> for Tier1SignError {
    fn from(e: VaultError) -> Self {
        Self::Vault(e)
    }
}

/// Builds and signs this instance's Tier-1 record, without ever contacting
/// a writer: a locked vault, a missing app master, or a vault key that does
/// not match `expected_app_did` all stop here, before any network call is
/// attempted.
///
/// `ttl_secs` becomes the record's own declared `EndpointInfo.ttl`, which is
/// what the community registry's sweep evicts an entry by once it is
/// `Some` (`DEFAULT_REGISTRY_TTL_SECS`, 2h, only when absent) --
/// deliberately not left `None`: this record is refreshed on
/// `refresh_interval_secs`, a caller-supplied cadence that can be far
/// longer than 2h (the master-anchor default this slice's own refresh
/// reuses is 12h), so leaving the default would mean the record is evicted
/// and unresolvable for most of the interval between refreshes. Set with
/// the same generosity `DEFAULT_ENDPOINT_NOT_AFTER_SECS`'s own doc argues
/// for: comfortably wider than one refresh cycle, so a single slow or
/// delayed pass does not cost a resolution failure.
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
/// answers for; recorded as a backlog row).
#[allow(clippy::too_many_arguments)]
pub async fn sign_tier1_record(
    vault: &MasterVault,
    app_instance_id: &str,
    expected_app_did: &str,
    supervisor_did: &str,
    generation: u64,
    refresh_interval_secs: u64,
) -> Result<SignedEndpointInfo, Tier1SignError> {
    let master = keys::existing_app_master(vault, app_instance_id).await?.ok_or_else(|| {
        Tier1SignError::Vault(VaultError::Storage(anyhow::anyhow!(
            "app master DID is recorded on this instance's row but not found in this supervisor's \
             vault"
        )))
    })?;
    let actual_did = substrate::derive_did_key(&master.public_key());
    if actual_did != expected_app_did {
        return Err(Tier1SignError::IdentityMismatch {
            expected: expected_app_did.to_string(),
            actual: actual_did,
        });
    }
    let not_after = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
    // A 3x margin over the refresh interval: two consecutive missed
    // passes still leave the record resolvable, matching the multi-pass
    // margin `DEFAULT_ENDPOINT_NOT_AFTER_SECS`'s own doc argues for at
    // the 30-day scale.
    let ttl_secs = refresh_interval_secs.saturating_mul(3);
    let info = EndpointInfo {
        service_id: actual_did,
        substrate_id: supervisor_did.to_string(),
        endpoint_type: EndpointType::Substrate,
        mechanisms: vec![],
        nickname: Some(app_instance_id.to_string()),
        is_private: false,
        ttl: Some(ttl_secs),
        not_after,
        generation,
    };
    info.sign(&master).map_err(|e| Tier1SignError::Vault(VaultError::Storage(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::MasterKind;

    const INTERVAL: u64 = 12 * 3600;

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

        let err =
            sign_tier1_record(&v, "inst-1", "did:key:zExpected", "did:key:zNode", 0, INTERVAL)
                .await
                .unwrap_err();
        assert!(matches!(err, Tier1SignError::Vault(VaultError::Storage(_))), "{err:?}");
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
        let (app_did, _) = keys::app_master(&v, "inst-1").await.unwrap();

        let signed =
            sign_tier1_record(&v, "inst-1", &app_did, "did:key:zSupervisorNode", 3, INTERVAL)
                .await
                .unwrap();
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

        let signed =
            sign_tier1_record(&v, "inst-1", &app_did, "did:key:zNode", 0, INTERVAL).await.unwrap();
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

        let err =
            sign_tier1_record(&v, "inst-1", "did:key:zExpected", "did:key:zNode", 0, INTERVAL)
                .await
                .unwrap_err();
        assert!(matches!(err, Tier1SignError::Vault(VaultError::Locked)), "{err:?}");
    }

    /// The vault's own key must match the DID the row recorded at the last
    /// `adopt` -- a mismatch (an `import-master` not yet followed by an
    /// `adopt`) must refuse rather than publish under whichever DID the
    /// vault happens to hold.
    #[tokio::test]
    async fn a_vault_key_that_does_not_match_the_rows_recorded_did_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path()).await;
        // The vault actually holds this instance's real app master...
        let (actual_did, _) = keys::app_master(&v, "inst-1").await.unwrap();

        // ...but the row claims a different one (the stale-handover case).
        let err =
            sign_tier1_record(&v, "inst-1", "did:key:zStaleRowClaim", "did:key:zNode", 0, INTERVAL)
                .await
                .unwrap_err();
        match err {
            Tier1SignError::IdentityMismatch { expected, actual } => {
                assert_eq!(expected, "did:key:zStaleRowClaim");
                assert_eq!(actual, actual_did);
            }
            other => panic!("expected IdentityMismatch, got {other:?}"),
        }
    }

    /// S1-1: the record's own `ttl` must comfortably outlive the interval
    /// it is refreshed on, or the registry's sweep (which evicts by this
    /// exact field once it is `Some`) removes the entry well before the
    /// next refresh lands.
    #[tokio::test]
    async fn the_records_ttl_leaves_multiple_refresh_cycles_of_margin() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path()).await;
        let (app_did, _) = keys::app_master(&v, "inst-1").await.unwrap();

        let signed =
            sign_tier1_record(&v, "inst-1", &app_did, "did:key:zNode", 0, INTERVAL).await.unwrap();

        let ttl = signed.info.ttl.expect("the record must declare an explicit ttl");
        assert!(
            ttl >= INTERVAL * 2,
            "ttl ({ttl}s) must leave at least one full refresh cycle of margin over the interval \
             ({INTERVAL}s) it is refreshed on"
        );
    }
}
