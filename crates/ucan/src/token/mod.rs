//! Signed UCAN capability tokens and delegation-chain verification
//! (ADR-0015 §2).

use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use syneroym_identity::{Identity, substrate};

use crate::capability::Capability;

/// Clock-skew tolerance for `not_before` (mirrors `DelegationCertificate`'s
/// 300 s future-issue tolerance).
pub(crate) const CLOCK_SKEW_SECS: u64 = 300;

/// Upper bound on the total number of tokens (leaf + every proof,
/// transitively) a single chain may contain. Verifying a chain costs one
/// Ed25519 verify per node, and the router additionally resolves one
/// revocation anchor per `(issuer, audience)` edge -- both proportional to
/// node count, not to any depth limit `serde_json`'s own recursion guard
/// already enforces. Breadth (many sibling `proofs` at one level) isn't
/// bounded by that recursion-depth guard at all, so an unrooted,
/// self-minted chain could otherwise force an arbitrarily large amount of
/// verification/network work before being rejected. 64 comfortably covers
/// any real delegation depth while keeping worst-case cost bounded and
/// cheap.
const MAX_CHAIN_NODES: usize = 64;

fn now_secs() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX epoch")
        .map(|d| d.as_secs())
}

/// A signed UCAN capability token (ADR-0015 §2). `proofs` are the parent
/// tokens forming the delegation chain. The Ed25519 signature covers the
/// RFC-8785 canonicalization of every field **except `signature` and
/// `proofs`** — each proof is independently signed by its own issuer, and
/// chain continuity (`proof.audience_did == child.issuer_did`) binds them, so
/// a valid proof cannot be repackaged under a child the proof's issuer never
/// signed for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub issuer_did: String,
    pub audience_did: String,
    /// The original principal this token's chain acts for, when that differs
    /// from `issuer_did`/`audience_did` (ADR-0015 A5, amended). Signed, so a
    /// middle service cannot rewrite it without invalidating its own
    /// signature; `verify_chain` enforces the propagation invariant: every
    /// `Some(a)` is either self-declared (`a == issuer_did`) or inherited
    /// unchanged from a continuity-respecting proof. `None` means "no anchor
    /// asserted" -- a direct caller is implicitly its own anchor
    /// (`SessionContext::anchor_did` falls back to `subject_did`, not a hard
    /// requirement here).
    #[serde(default)]
    pub anchor_did: Option<String>,
    pub capabilities: Vec<Capability>,
    /// Proven claims surfaced into `SessionContext.claims`; the data layer
    /// binds these as SQL `?` params. Empty by default.
    #[serde(default)]
    pub facts: Map<String, Value>,
    pub not_before_secs: u64,
    pub expires_at_secs: u64,
    #[serde(default)]
    pub proofs: Vec<CapabilityToken>,
    /// z-base-32 Ed25519 signature over the canonical body (sans this field
    /// and `proofs`).
    pub signature: String,
}

impl CapabilityToken {
    /// Builds the value covered by the signature: every field except
    /// `signature` and `proofs`. Deliberately field-by-field (not
    /// `serde_json::to_value(self)` with the two keys stripped afterward) --
    /// serializing `self` whole would serialize the entire nested `proofs`
    /// subtree first only to discard it, making per-node verification cost
    /// O(subtree size) and the whole chain walk quadratic in chain length.
    pub(crate) fn signing_value(&self) -> Value {
        serde_json::json!({
            "issuer_did": self.issuer_did,
            "audience_did": self.audience_did,
            "anchor_did": self.anchor_did,
            "capabilities": self.capabilities,
            "facts": self.facts,
            "not_before_secs": self.not_before_secs,
            "expires_at_secs": self.expires_at_secs,
        }) // canonicalization happens inside sign_json / verify_json_signature
    }

    /// Issue a new signed `CapabilityToken` with no anchor assertion
    /// (`anchor_did: None`). A direct grant this shape falls back to
    /// treating its audience as its own anchor (`SessionContext`). Use
    /// [`Self::issue_with_anchor`] to self-declare or propagate an anchor.
    pub fn issue(
        issuer: &Identity,
        audience_did: &str,
        capabilities: Vec<Capability>,
        facts: Map<String, Value>,
        expires_in_secs: u64,
        proofs: Vec<CapabilityToken>,
    ) -> Result<Self> {
        Self::issue_with_anchor(
            issuer,
            audience_did,
            None,
            capabilities,
            facts,
            expires_in_secs,
            proofs,
        )
    }

    /// Issue a new signed `CapabilityToken` carrying an explicit
    /// `anchor_did` (ADR-0015 A5, amended: an explicit signed stamp, not
    /// structural derivation). Two legitimate calls:
    /// - **Origination** — `anchor_did = Some(issuer's own DID)`: the principal
    ///   self-declares itself as the anchor.
    /// - **Propagation** — `anchor_did` copied unchanged from the DID this
    ///   issuer received in its own parent proof: a proxying service passes its
    ///   anchor through without alteration.
    ///
    /// `verify_chain` enforces this is the only shape a verifier will
    /// accept; asserting any other value makes the whole chain reject.
    pub fn issue_with_anchor(
        issuer: &Identity,
        audience_did: &str,
        anchor_did: Option<String>,
        capabilities: Vec<Capability>,
        facts: Map<String, Value>,
        expires_in_secs: u64,
        proofs: Vec<CapabilityToken>,
    ) -> Result<Self> {
        let issuer_did = substrate::derive_did_key(&issuer.public_key());
        let now = now_secs()?;
        let mut token = Self {
            issuer_did,
            audience_did: audience_did.to_string(),
            anchor_did,
            capabilities,
            facts,
            not_before_secs: now,
            expires_at_secs: now + expires_in_secs,
            proofs,
            signature: String::new(),
        };
        token.signature = issuer.sign_json(&token.signing_value())?;
        Ok(token)
    }

    /// Per-token structural verification (signature + time bounds); does not
    /// walk the proof chain.
    fn verify_self(&self, now_secs: u64) -> Result<()> {
        if self.not_before_secs >= self.expires_at_secs {
            return Err(anyhow!("token has non-positive validity window"));
        }
        if self.not_before_secs > now_secs + CLOCK_SKEW_SECS {
            return Err(anyhow!("token not_before is in the future"));
        }
        if now_secs >= self.expires_at_secs {
            return Err(anyhow!("token expired"));
        }
        substrate::verify_json_signature(&self.issuer_did, &self.signing_value(), &self.signature)
            .context("token signature verification failed")
    }

    /// Pre-order walk of this token and every proof in its chain, yielding
    /// each `(issuer_did, audience_did)` edge. Used by the router to check
    /// each edge's audience against the issuer's revocation anchor.
    #[must_use]
    pub fn chain_edges(&self) -> Vec<(&str, &str)> {
        let mut edges = vec![(self.issuer_did.as_str(), self.audience_did.as_str())];
        for proof in &self.proofs {
            edges.extend(proof.chain_edges());
        }
        edges
    }
}

/// Options for verifying a presented token chain.
pub struct ChainVerifyOpts<'a> {
    /// The DID this token must be addressed to (the verified connection
    /// identity). The leaf's `audience_did` must equal this — binds the
    /// token to the presenter, preventing replay of a token issued to
    /// someone else.
    pub expected_audience_did: &'a str,
    /// Returns whether `issuer_did` is a trusted root of authority for
    /// `capability` (its resource *and* the ability being claimed: an
    /// owner-rooted root may need to trust a resource for some abilities but
    /// not others, e.g. `data-layer/read` but not the `data-layer/admin`
    /// escape hatch, so the predicate needs the ability, not just the
    /// resource). The simplest root predicate is
    /// `|iss, _cap| iss == admin_root`. `Send + Sync` so `ChainVerifyOpts`
    /// (and futures holding it across an `.await`) stay usable from
    /// `tokio::spawn`ed connection handlers.
    pub is_trusted_root: &'a (dyn Fn(&str, &Capability) -> bool + Send + Sync),
    pub now_secs: u64,
}

impl fmt::Debug for ChainVerifyOpts<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainVerifyOpts")
            .field("expected_audience_did", &self.expected_audience_did)
            .field("is_trusted_root", &"<fn>")
            .field("now_secs", &self.now_secs)
            .finish()
    }
}

/// Verify the chain and return the set of capabilities the leaf provably
/// holds — i.e. each is either rooted directly at a trusted issuer or
/// attenuated from a valid, trusted proof. Capabilities that cannot be
/// traced to a trusted root are **dropped** (fail-closed at the capability
/// granularity); an entirely unbacked leaf yields an empty set, not an
/// error. Returns `Err` only on a structural failure (bad signature,
/// expiry, audience mismatch).
pub fn verify_chain(leaf: &CapabilityToken, opts: &ChainVerifyOpts<'_>) -> Result<Vec<Capability>> {
    if leaf.audience_did != opts.expected_audience_did {
        return Err(anyhow!(
            "token audience {} does not match presenter {}",
            leaf.audience_did,
            opts.expected_audience_did
        ));
    }
    // Cheap linear count-and-bail *before* any signature verification or
    // (in the router) revocation lookups -- an unrooted, self-minted chain
    // with wide `proofs` fan-out would otherwise force a proportionally
    // large number of Ed25519 verifies and per-edge anchor resolutions
    // before ultimately being rejected for granting nothing.
    if total_chain_nodes(leaf) > MAX_CHAIN_NODES {
        return Err(anyhow!(
            "UCAN chain has more than {MAX_CHAIN_NODES} tokens (leaf + proofs, transitively)"
        ));
    }
    granted_capabilities(leaf, opts)
}

fn total_chain_nodes(token: &CapabilityToken) -> usize {
    1 + token.proofs.iter().map(total_chain_nodes).sum::<usize>()
}

fn granted_capabilities(
    token: &CapabilityToken,
    opts: &ChainVerifyOpts<'_>,
) -> Result<Vec<Capability>> {
    token.verify_self(opts.now_secs)?; // fail-closed: bad link aborts
    let mut effective = Vec::new();
    // Verify proofs once; a proof that fails structurally makes the whole
    // presentation invalid (do not silently ignore a tampered proof).
    let parent_grants: Vec<Vec<Capability>> = token
        .proofs
        .iter()
        .map(|p| {
            if p.audience_did != token.issuer_did {
                // continuity break: this proof does not delegate to this issuer
                return Ok(Vec::new());
            }
            granted_capabilities(p, opts)
        })
        .collect::<Result<_>>()?;
    // ADR-0015 A5 (amended): a `Some(a)` anchor is self-declared (`a ==
    // token.issuer_did`) or must be substantiated by the *same* proof that
    // backs each admitted capability -- not merely present on some sibling
    // proof. Otherwise a token could combine a capability from one
    // delegation lineage with an anchor borrowed from an unrelated one (the
    // anchor's own lineage never actually authorized this capability). A
    // rooted capability has no delegation lineage at all, so it can never
    // substantiate a non-self-declared anchor -- exactly the shape that
    // matters, since a root's unconditional trust says nothing about who a
    // *different*, delegated anchor is acting for.
    let self_declared_anchor = token.anchor_did.as_deref() == Some(token.issuer_did.as_str());
    for cap in &token.capabilities {
        let rooted = (opts.is_trusted_root)(&token.issuer_did, cap);
        // ADR-0015 A3/A4: a parent capability only backs a child's if the
        // parent also permits further delegation. `can_delegate` is
        // *checked*, not conjoined, into the child (it is terminal, not
        // intersective like `where`/`fields`) -- once a held capability
        // carries `can_delegate: false`, nothing derived from it can
        // attenuate any further, no matter how many hops re-wrap it.
        let backing_proofs: Vec<&CapabilityToken> = token
            .proofs
            .iter()
            .zip(&parent_grants)
            .filter(|(_, grants)| grants.iter().any(|pc| pc.covers(cap) && pc.can_delegate()))
            .map(|(p, _)| p)
            .collect();
        let backed = !backing_proofs.is_empty();
        if !(rooted || backed) {
            // dropped (fail-closed) — issuer is neither a trusted root nor
            // holds a proof entailing this capability.
            continue;
        }
        if let Some(anchor) = &token.anchor_did
            && !self_declared_anchor
            && !backing_proofs.iter().any(|p| p.anchor_did.as_deref() == Some(anchor.as_str()))
        {
            return Err(anyhow!(
                "token issued by {} admits capability {cap:?} under anchor_did '{anchor}', but no \
                 proof backing that capability carries the same anchor",
                token.issuer_did
            ));
        }
        effective.push(cap.clone());
    }
    Ok(effective)
}

#[cfg(test)]
mod tests;
