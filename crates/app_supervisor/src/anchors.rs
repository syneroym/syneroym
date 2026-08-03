//! The master-anchor writes the supervisor makes on its own schedule
//! (M05A A5d): the periodic republication that keeps every managed master's
//! anchor verifiable, and the revocation an operator asks for.
//!
//! Behind a trait rather than a bare `RegistryClient` for the same reason
//! `SubstrateActor` is: both writes are on the resident loop's own path and
//! on an operator verb's, and neither should need a live HTTP registry to
//! be tested.

use std::{fmt, sync::Arc};

use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::Identity;

/// The two anchor writes a supervisor issues.
///
/// Both are read-modify-write against the registry's current anchor for
/// that master, and both rely on the same single-writer invariant custody
/// already establishes: mint-in-place means exactly one vault ever holds a
/// given master, and `export-master`/`import-master` move a file rather
/// than granting two live processes concurrent access.
#[async_trait::async_trait]
pub trait AnchorWriter: fmt::Debug + Send + Sync {
    /// Re-signs and republishes `master`'s anchor, carrying forward
    /// everything the current one holds. An anchor stops verifying 24 hours
    /// after it was signed, so this is a standing duty, not a one-time
    /// publish.
    async fn refresh(&self, master: &Identity) -> Result<(), String>;
    /// Adds a derived instance DID to `master`'s revoked-key list and
    /// republishes. `instance_did` is the certificate's `temporary_did`,
    /// never the master's own.
    async fn revoke_instance(&self, master: &Identity, instance_did: &str) -> Result<(), String>;
}

/// The production writer: this node's configured HTTP registry (and, when
/// enabled, the DHT), reached through the same client `roymctl identity
/// publish-anchor` uses.
#[derive(Debug)]
pub struct RegistryAnchorWriter {
    client: RegistryClient,
}

impl RegistryAnchorWriter {
    #[must_use]
    pub fn new(enable_dht: bool, registry_url: String) -> Self {
        Self { client: RegistryClient::new(enable_dht, Some(registry_url)) }
    }

    /// `None` when this node has no `substrate.registry_url` configured.
    /// An anchor published nowhere is worse than no anchor at all -- every
    /// consumer resolving it would fail closed on a missing record it
    /// cannot distinguish from a revoked one -- so the supervisor holds no
    /// writer rather than a writer that quietly does nothing.
    #[must_use]
    pub fn from_registry_url(
        enable_dht: bool,
        registry_url: Option<&str>,
    ) -> Option<Arc<dyn AnchorWriter>> {
        registry_url
            .map(|url| Arc::new(Self::new(enable_dht, url.to_string())) as Arc<dyn AnchorWriter>)
    }
}

#[async_trait::async_trait]
impl AnchorWriter for RegistryAnchorWriter {
    async fn refresh(&self, master: &Identity) -> Result<(), String> {
        self.client.refresh_master_anchor(master).await.map_err(|e| e.to_string())
    }

    async fn revoke_instance(&self, master: &Identity, instance_did: &str) -> Result<(), String> {
        self.client.revoke_instance_key(master, instance_did).await.map_err(|e| e.to_string())
    }
}
