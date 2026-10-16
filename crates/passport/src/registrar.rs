//! The registration checks of one service for one passport use.
//!
//! Registration is two calls. `check` runs every offline check and writes
//! nothing. The service then runs the DHT revocation check on every chain
//! edge (it needs the network, so it runs only after the cheap checks
//! passed), and on success calls `grant`, which runs the deny check, the use
//! limit count and the lease write under one lock.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, ensure};
use syneroym_ucan::CapabilityToken;

use crate::{
    chain::{ChainRules, PassportChain},
    challenge_store::ChallengeStore,
    error::RegisterError,
    issue::{PassportId, ensure_canonical_did},
    lease_store::{Lease, LeaseStore},
    passport_use::PassportUse,
    proof::{Challenge, Proof, SignerBinding},
};

/// Gives each `Registrar` of the process its own id.
static NEXT_REGISTRAR_ID: AtomicU64 = AtomicU64::new(1);

/// The registration checks of one service for one use.
#[derive(Debug)]
pub struct Registrar {
    id: u64,
    use_: PassportUse,
    host_node_did: String,
    accepted_roots: Vec<String>,
    challenges: ChallengeStore,
    leases: LeaseStore,
}

/// The result of `Registrar::check`. Only `check` can make one (private
/// fields), so `grant` never writes a lease for a token that was not checked.
#[derive(Debug, Clone)]
pub struct CheckedRegistration {
    /// The `Registrar` that made this value. Only that one may grant it.
    registrar_id: u64,
    /// The presented child token, with the passport inside.
    token: CapabilityToken,
    /// The lease `grant` will write.
    lease: Lease,
    /// The use limit read from the passport.
    limit: Option<u64>,
}

impl CheckedRegistration {
    /// The checked token, for the revocation check of every chain edge.
    #[must_use]
    pub fn token(&self) -> &CapabilityToken {
        &self.token
    }

    /// The lease `grant` will write, for logs.
    #[must_use]
    pub fn lease(&self) -> &Lease {
        &self.lease
    }
}

impl Registrar {
    /// `accepted_roots`: the operator roots this service trusts (the host's
    /// controller DID and/or its node DID). It must not be empty. Every DID
    /// must be canonical, and `leases` must belong to the same use.
    pub fn new(
        use_: PassportUse,
        host_node_did: String,
        accepted_roots: Vec<String>,
        leases: LeaseStore,
    ) -> Result<Self> {
        ensure!(!accepted_roots.is_empty(), "a passport service needs at least one accepted root");
        ensure_canonical_did(&host_node_did).context("the host node DID")?;
        for root in &accepted_roots {
            ensure_canonical_did(root).context("an accepted root")?;
        }
        ensure!(
            leases.ability() == use_.ability,
            "the lease store belongs to {}, not to {}",
            leases.ability(),
            use_.ability
        );
        Ok(Self {
            id: NEXT_REGISTRAR_ID.fetch_add(1, Ordering::Relaxed),
            use_,
            host_node_did,
            accepted_roots,
            challenges: ChallengeStore::default(),
            leases,
        })
    }

    #[must_use]
    pub fn passport_use(&self) -> &PassportUse {
        &self.use_
    }

    #[must_use]
    pub fn leases(&self) -> &LeaseStore {
        &self.leases
    }

    /// A fresh challenge for the challenge form.
    pub fn issue_challenge(&self, now_secs: u64) -> Result<Challenge> {
        self.challenges.issue(now_secs)
    }

    /// Every offline check: shape, proof of possession, signer binding,
    /// chain, and the limit value. Writes nothing, except that a challenge
    /// is used up. The caller then runs the DHT revocation check on
    /// `checked.token()` (it needs the network) and, if that passes, calls
    /// `grant`.
    pub fn check(
        &self,
        token: &CapabilityToken,
        proof: &Proof,
        binding: &SignerBinding,
        now_secs: u64,
    ) -> Result<CheckedRegistration, RegisterError> {
        let chain = PassportChain::parse(token)?;
        let proven =
            proof.verify(&self.challenges, &self.host_node_did, &self.use_, token, now_secs)?;
        binding.check(proven, chain.subject(), now_secs)?;
        let is_root = |issuer: &str| self.is_root(issuer);
        ChainRules { use_: &self.use_, host_node_did: &self.host_node_did, is_root: &is_root }
            .check(&chain, now_secs)?;
        let limit = chain.count_limit(&self.use_)?;
        let passport_id =
            PassportId::of(chain.passport).map_err(|e| RegisterError::Malformed(e.to_string()))?;
        let lease = Lease {
            passport_id,
            owner_did: chain.passport.audience_did.clone(),
            subject: chain.subject().to_string(),
            lease_ends_at: chain
                .leaf
                .expires_at_secs
                .min(chain.passport.expires_at_secs)
                .min(now_secs.saturating_add(self.use_.max_lease_secs)),
            registered_at: now_secs,
            // Never the child's facts: an owner could widen a limit there.
            limits: chain.passport.facts.clone(),
        };
        Ok(CheckedRegistration { registrar_id: self.id, token: token.clone(), lease, limit })
    }

    /// The deny check, the use limit count and the lease write, under one
    /// lock. Call it only after the revocation check passed.
    pub fn grant(
        &self,
        checked: CheckedRegistration,
        now_secs: u64,
    ) -> Result<Lease, RegisterError> {
        if checked.registrar_id != self.id {
            return Err(RegisterError::Store(
                "the registration was checked by another registrar".into(),
            ));
        }
        // The revocation lookup may take seconds. A token that expired in
        // that time is refused, so a lease never starts already ended.
        if now_secs >= checked.lease.lease_ends_at {
            return Err(RegisterError::Expired);
        }
        self.leases.grant(checked.lease.clone(), checked.limit, now_secs)?;
        Ok(checked.lease)
    }

    fn is_root(&self, issuer: &str) -> bool {
        self.accepted_roots.iter().any(|r| r == issuer)
    }
}

#[cfg(test)]
mod tests;
