//! A shared fixture for the registration tests: an operator, a host, an
//! owner, a subject and an instance key, with a registrar for one use.
//!
//! Every token and every challenge takes its time from `Fixture::now`, read
//! once, so a boundary test compares exact values. A
//! `DelegationCertificate` is the one exception: it reads the wall clock
//! itself, so its tests keep a margin of several seconds.

use serde_json::{Map, Value};
use syneroym_identity::{DelegationCertificate, Identity, delegation, substrate};
use syneroym_ucan::CapabilityToken;

use crate::{
    Lease, LeaseStore, MAILBOX_USE, PassportUse, Proof, RELAY_HOME, RegisterError, Registrar,
    SignerBinding, child_token, issue_passport, now_secs, sign_challenge,
};

/// The life of an instance certificate in these tests.
pub(crate) const CERT_LIFE_SECS: u64 = 600;
/// The default passport life in these tests. Longer than the challenge life
/// and the certificate life, so a test of either is not hidden by expiry.
pub(crate) const PASSPORT_LIFE_SECS: u64 = 3_600;

pub(crate) fn did(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

pub(crate) fn new_did() -> String {
    did(&Identity::generate().unwrap())
}

/// Another spelling of `did` that decodes to the same key, if the z-base-32
/// decoder accepts one: the last character carries spare bits.
pub(crate) fn other_spelling(did: &str) -> Option<String> {
    const ALPHABET: &str = "ybndrfg8ejkmcpqxot1uwisza345h769";
    let key = substrate::resolve_did_key(did).ok()?;
    let (head, last) = did.split_at(did.len() - 1);
    ALPHABET
        .chars()
        .filter(|c| c.to_string() != last)
        .map(|c| format!("{head}{c}"))
        .find(|other| substrate::resolve_did_key(other).is_ok_and(|k| k == key))
}

/// A child token with the passport as its one proof, built by hand so a
/// test can set fields `child_token` would refuse. Signed by `owner`.
pub(crate) fn hand_child(
    owner: &Identity,
    passport: &CapabilityToken,
    audience_did: &str,
    facts: Map<String, Value>,
) -> CapabilityToken {
    let mut token = CapabilityToken {
        issuer_did: did(owner),
        audience_did: audience_did.to_string(),
        anchor_did: None,
        capabilities: passport.capabilities.clone(),
        facts,
        not_before_secs: passport.not_before_secs,
        expires_at_secs: passport.expires_at_secs,
        proofs: vec![passport.clone()],
        signature: String::new(),
    };
    token.sign(owner).unwrap();
    token
}

#[derive(Debug)]
pub(crate) struct Fixture {
    /// The accepted root.
    pub operator: Identity,
    /// A fresh DID, not the operator.
    pub host_node_did: String,
    pub owner: Identity,
    /// Relay: the node key. Mailbox: the member master.
    pub subject: Identity,
    /// Mailbox: the instance key of the subject.
    pub instance: Identity,
    pub registrar: Registrar,
    /// Wall-clock now, read once.
    pub now: u64,
}

impl Fixture {
    pub(crate) fn new(use_: PassportUse) -> Self {
        let operator = Identity::generate().unwrap();
        let host_node_did = new_did();
        let registrar = Registrar::new(
            use_,
            host_node_did.clone(),
            vec![did(&operator)],
            LeaseStore::open_in_memory().unwrap(),
        )
        .unwrap();
        Self {
            operator,
            host_node_did,
            owner: Identity::generate().unwrap(),
            subject: Identity::generate().unwrap(),
            instance: Identity::generate().unwrap(),
            registrar,
            now: now_secs(),
        }
    }

    pub(crate) fn relay() -> Self {
        Self::new(RELAY_HOME)
    }

    pub(crate) fn mailbox() -> Self {
        Self::new(MAILBOX_USE)
    }

    /// The same fixture with another root set and an empty lease store.
    pub(crate) fn with_roots(mut self, roots: Vec<String>) -> Self {
        self.registrar = Registrar::new(
            *self.registrar.passport_use(),
            self.host_node_did.clone(),
            roots,
            LeaseStore::open_in_memory().unwrap(),
        )
        .unwrap();
        self
    }

    pub(crate) fn use_(&self) -> PassportUse {
        *self.registrar.passport_use()
    }

    pub(crate) fn passport(&self, limits: Map<String, Value>, life_secs: u64) -> CapabilityToken {
        self.passport_from(&self.operator, limits, life_secs)
    }

    pub(crate) fn passport_from(
        &self,
        issuer: &Identity,
        limits: Map<String, Value>,
        life_secs: u64,
    ) -> CapabilityToken {
        let use_ = self.use_();
        issue_passport(
            issuer,
            &self.host_node_did,
            &use_,
            &did(&self.owner),
            limits,
            life_secs,
            self.now,
        )
        .unwrap()
    }

    /// A child token that lives as long as its passport.
    pub(crate) fn child(&self, passport: &CapabilityToken, subject: &Identity) -> CapabilityToken {
        child_token(&self.owner, passport, &did(subject), u64::MAX, self.now).unwrap()
    }

    /// A passport with `limits` and the default life, delegated to the
    /// fixture subject.
    pub(crate) fn token(&self, limits: Map<String, Value>) -> CapabilityToken {
        self.child(&self.passport(limits, PASSPORT_LIFE_SECS), &self.subject)
    }

    pub(crate) fn challenge_proof(
        &self,
        signer: &Identity,
        token: &CapabilityToken,
        at: u64,
    ) -> Proof {
        let challenge = self.registrar.issue_challenge(at).unwrap();
        let use_ = self.use_();
        let signature =
            sign_challenge(signer, &self.host_node_did, &use_, &challenge, token).unwrap();
        Proof::Challenge { challenge, signer_did: did(signer), signature }
    }

    pub(crate) fn transport_proof(verified_did: &str) -> Proof {
        Proof::Transport { verified_did: verified_did.to_string() }
    }

    pub(crate) fn instance_binding(master: &Identity, instance: &Identity) -> SignerBinding {
        Self::certificate(master, instance, delegation::SCOPE_SERVICE_INSTANCE)
    }

    pub(crate) fn certificate(
        master: &Identity,
        instance: &Identity,
        scope: &str,
    ) -> SignerBinding {
        SignerBinding::InstanceCertificate(
            DelegationCertificate::issue(
                master,
                instance.public_key(),
                CERT_LIFE_SECS,
                scope.to_string(),
            )
            .unwrap(),
        )
    }

    pub(crate) fn register_with(
        &self,
        token: &CapabilityToken,
        proof: &Proof,
        binding: &SignerBinding,
        at: u64,
    ) -> Result<Lease, RegisterError> {
        let checked = self.registrar.check(token, proof, binding, at)?;
        self.registrar.grant(checked, at)
    }

    /// Challenge form and same key, with a challenge issued at `at`.
    pub(crate) fn register_relay(
        &self,
        token: &CapabilityToken,
        signer: &Identity,
        at: u64,
    ) -> Result<Lease, RegisterError> {
        let proof = self.challenge_proof(signer, token, at);
        self.register_with(token, &proof, &SignerBinding::SameKey, at)
    }

    /// Transport form and an instance certificate from `master`.
    pub(crate) fn register_mailbox(
        &self,
        token: &CapabilityToken,
        instance: &Identity,
        master: &Identity,
        at: u64,
    ) -> Result<Lease, RegisterError> {
        let proof = Self::transport_proof(&did(instance));
        self.register_with(token, &proof, &Self::instance_binding(master, instance), at)
    }

    /// The normal path of this fixture's use, for `subject`: the relay
    /// helper for `relay/home`, the mailbox helper (with a fresh instance
    /// key) for `mailbox/use`.
    pub(crate) fn register_as(
        &self,
        token: &CapabilityToken,
        subject: &Identity,
        at: u64,
    ) -> Result<Lease, RegisterError> {
        if self.use_() == RELAY_HOME {
            self.register_relay(token, subject, at)
        } else {
            self.register_mailbox(token, &Identity::generate().unwrap(), subject, at)
        }
    }

    /// `register_as` for the fixture subject.
    pub(crate) fn register(
        &self,
        token: &CapabilityToken,
        at: u64,
    ) -> Result<Lease, RegisterError> {
        self.register_as(token, &self.subject, at)
    }
}

/// One fixture per use, so each chain case runs with both proof forms.
pub(crate) fn both() -> [Fixture; 2] {
    [Fixture::relay(), Fixture::mailbox()]
}

pub(crate) fn limits(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}
