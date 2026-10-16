//! The chain rules and token structure for operator passports.

use syneroym_ucan::{Capability, CapabilityToken, ChainVerifyOpts, verify_chain};

use crate::{error::RegisterError, issue::ensure_canonical_did, passport_use::PassportUse};

/// The chain rules of one use at one host. `is_root` decides which issuers
/// may root a passport.
pub(crate) struct ChainRules<'a> {
    pub(crate) use_: &'a PassportUse,
    pub(crate) host_node_did: &'a str,
    pub(crate) is_root: &'a (dyn Fn(&str) -> bool + Send + Sync),
}

impl ChainRules<'_> {
    /// Root, link, exact capability, delegation, expiry, then every
    /// signature through `verify_chain`. Same order and same errors as
    /// before the move.
    pub(crate) fn check(
        &self,
        chain: &PassportChain<'_>,
        now_secs: u64,
    ) -> Result<(), RegisterError> {
        let (passport, leaf) = (chain.passport, chain.leaf);
        if !(self.is_root)(&passport.issuer_did) {
            return Err(RegisterError::UntrustedRoot);
        }
        if passport.audience_did != leaf.issuer_did {
            return Err(RegisterError::BrokenChain);
        }
        if ![passport, leaf]
            .iter()
            .all(|t| self.use_.names_exactly(&t.capabilities[0], self.host_node_did))
        {
            return Err(self.wrong_capability());
        }
        // Checked here, not left to `verify_chain`: when the owner is also
        // an accepted root, `verify_chain` roots the child token on its own
        // and never reads the passport's caveat.
        if !passport.capabilities[0].can_delegate() {
            return Err(RegisterError::InvalidChain("the passport forbids delegation".into()));
        }
        if now_secs >= passport.expires_at_secs || now_secs >= leaf.expires_at_secs {
            return Err(RegisterError::Expired);
        }
        self.verify_signatures(chain, now_secs)
    }

    fn verify_signatures(
        &self,
        chain: &PassportChain<'_>,
        now_secs: u64,
    ) -> Result<(), RegisterError> {
        let is_root = |issuer: &str, cap: &Capability| {
            (self.is_root)(issuer) && self.use_.names_exactly(cap, self.host_node_did)
        };
        let opts = ChainVerifyOpts {
            expected_audience_did: chain.subject(),
            is_trusted_root: &is_root,
            now_secs,
        };
        let granted = verify_chain(chain.leaf, &opts)
            .map_err(|e| RegisterError::InvalidChain(format!("{e:#}")))?;
        // `verify_chain` drops a capability it cannot back, so an empty
        // result is a refusal.
        if !granted.iter().any(|c| self.use_.names_exactly(c, self.host_node_did)) {
            return Err(RegisterError::InvalidChain(
                "the passport does not back the child token's capability".into(),
            ));
        }
        Ok(())
    }

    fn wrong_capability(&self) -> RegisterError {
        RegisterError::WrongCapability {
            ability: self.use_.ability.to_string(),
            resource: self.use_.resource(self.host_node_did).0,
        }
    }
}

/// A child token with exactly one proof, the passport.
pub(crate) struct PassportChain<'a> {
    pub(crate) leaf: &'a CapabilityToken,
    pub(crate) passport: &'a CapabilityToken,
}

impl<'a> PassportChain<'a> {
    /// The shape check. No signature work happens here.
    pub(crate) fn parse(leaf: &'a CapabilityToken) -> Result<Self, RegisterError> {
        let malformed = |msg: &str| Err(RegisterError::Malformed(msg.to_string()));
        let [passport] = leaf.proofs.as_slice() else {
            return malformed("the child token must carry exactly one proof, the passport");
        };
        if !passport.proofs.is_empty() {
            return malformed("the passport must not carry proofs");
        }
        if leaf.capabilities.len() != 1 {
            return malformed("the child token must carry exactly one capability");
        }
        if passport.capabilities.len() != 1 {
            return malformed("the passport must carry exactly one capability");
        }
        ensure_canonical_did(&leaf.audience_did)
            .map_err(|e| RegisterError::Malformed(format!("the subject: {e:#}")))?;
        Ok(Self { leaf, passport })
    }

    pub(crate) fn subject(&self) -> &'a str {
        &self.leaf.audience_did
    }

    /// The use limit, read from the passport only.
    pub(crate) fn count_limit(&self, use_: &PassportUse) -> Result<Option<u64>, RegisterError> {
        match self.passport.facts.get(use_.count_key) {
            None => Ok(use_.open_count_limit),
            Some(v) => v.as_u64().map(Some).ok_or_else(|| {
                RegisterError::Malformed(format!("{} is not a whole number", use_.count_key))
            }),
        }
    }
}
