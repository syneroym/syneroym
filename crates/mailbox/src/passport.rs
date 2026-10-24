//! Passport admission at `register`: a dedicated mailbox lets the owner of a
//! passport raise the limits of the queues of his services (ADR-0015 A10).

use std::{fs, path::Path, sync::Arc, time::Duration};

use anyhow::Context;
use metrics::counter;
use syneroym_core::{
    dht_registry::{self, MasterAnchorResolver},
    host_identity::HostIdentity,
    mailbox::MAX_CHILD_TOKEN_BYTES,
};
use syneroym_identity::DelegationCertificate;
use syneroym_passport::{
    CapabilityToken, Lease, LeaseStore, MAILBOX_USE, Proof, RegisterError, Registrar, SignerBinding,
};
use tokio::{task, time};
use tracing::{error, info, warn};

use crate::{caller::Instance, error::MailboxError};

/// The lease file of the mailbox, in `storage.db_dir`.
pub(crate) const MAILBOX_LEASES_DB: &str = "mailbox_passports.db";

/// Time allowed for the revocation lookup across the chain edges. A
/// `register` is one native call, and the receiver waits at most 10 s for
/// it, after the instance revocation check of up to 5 s.
const REVOCATION_LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);

/// What `register` needs to admit a child token.
pub(crate) struct MailboxPassports {
    registrar: Arc<Registrar>,
    resolver: Arc<dyn MasterAnchorResolver>,
}

impl MailboxPassports {
    /// Opens the lease store in `db_dir` for `mailbox/use`, rooted at the
    /// host's passport root (ADR-0015 A10 point 4).
    pub(crate) fn open(
        db_dir: &Path,
        host: &HostIdentity,
        resolver: Arc<dyn MasterAnchorResolver>,
        now: u64,
    ) -> anyhow::Result<Self> {
        fs::create_dir_all(db_dir)?;
        let leases = LeaseStore::open(db_dir, MAILBOX_LEASES_DB, &MAILBOX_USE, now)
            .context("could not open the mailbox lease store")?;
        Self::with_leases(host, resolver, leases)
    }

    #[cfg(test)]
    pub(crate) fn in_memory(
        host: &HostIdentity,
        resolver: Arc<dyn MasterAnchorResolver>,
    ) -> anyhow::Result<Self> {
        Self::with_leases(host, resolver, LeaseStore::open_in_memory(&MAILBOX_USE)?)
    }

    fn with_leases(
        host: &HostIdentity,
        resolver: Arc<dyn MasterAnchorResolver>,
        leases: LeaseStore,
    ) -> anyhow::Result<Self> {
        let root = host.passport_root().to_string();
        let registrar =
            Registrar::new(MAILBOX_USE, host.node_did.clone(), vec![root.clone()], leases)
                .context("the mailbox accepts passports")?;
        info!(%root, "the mailbox accepts passports issued by this DID");
        Ok(Self { registrar: Arc::new(registrar), resolver })
    }

    pub(crate) fn leases(&self) -> &LeaseStore {
        self.registrar.leases()
    }

    /// Checks `child_token` for `who`, runs the revocation check on every
    /// chain edge, and writes the lease.
    ///
    /// `now` is the clock of the whole call. The lookup takes at most
    /// 4 s, so a lease may start that much in the past.
    pub(crate) async fn admit(
        &self,
        who: &Instance,
        cert: DelegationCertificate,
        child_token: &str,
        now: u64,
    ) -> Result<Lease, MailboxError> {
        if child_token.len() > MAX_CHILD_TOKEN_BYTES {
            return Err(RegisterError::Malformed("child token over 16 KiB".to_string()).into());
        }
        let token: CapabilityToken = serde_json::from_str(child_token)
            .map_err(|e| RegisterError::Malformed(format!("child token does not parse: {e}")))?;
        let proof = Proof::Transport { verified_did: who.instance_did.clone() };
        let binding = SignerBinding::InstanceCertificate(cert);
        let checked = self.registrar.check(&token, &proof, &binding, now)?;
        // ADR-0015 §2 and A7: a revoked edge stops the registration. The
        // lookup needs the network, so it runs only after every offline
        // check passed. On timeout or network failure it counts as not
        // revoked.
        let edges = checked.token().chain_edges();
        let lookup = dht_registry::chain_edges_not_revoked(&edges, self.resolver.as_ref());
        match time::timeout(REVOCATION_LOOKUP_TIMEOUT, lookup).await {
            Ok(false) => return Err(RegisterError::Revoked.into()),
            Ok(true) => {}
            Err(_) => warn!("passport revocation lookup timed out; treating as not revoked"),
        }
        let registrar = Arc::clone(&self.registrar);
        let lease = task::spawn_blocking(move || registrar.grant(checked, now))
            .await
            .map_err(|e| MailboxError::Internal(format!("the lease write task failed: {e}")))?
            .inspect_err(|e| {
                if let RegisterError::Store(_) = e {
                    error!(error = %e, "mailbox passport store error");
                }
            })?;
        info!(passport_id = %lease.passport_id, subject = %lease.subject,
              lease_ends_at = lease.lease_ends_at, "mailbox passport lease granted");
        counter!("substrate.mailbox.passport_granted").increment(1);
        Ok(lease)
    }
}

#[cfg(test)]
mod tests;
