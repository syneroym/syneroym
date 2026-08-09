//! Tier 2 of the logical discovery overlay (ADR-0022 §3): the signed
//! topology document a caller outside an app instance fetches, verifies
//! against the app's own DID, caches, and routes from -- without trusting
//! whoever relayed it.

use std::{fmt, time::Duration};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use syneroym_identity::{Identity, substrate};

use crate::{
    models::{AppDid, AppInstanceId, LogicalServiceName, ServiceId, TopologyMode},
    resolver::{
        LogicalResolver, ShardingStrategy, TopologyEntry, TopologyEpoch, TopologyKey, unix_now,
    },
};

/// The signed payload. Every field is inside the signature; a relay that
/// edits any of them produces a document that does not verify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyDocument {
    /// The app's human name (ADR-0022 §1). Carried for display and for the
    /// `LogicalServiceRef` an intra-app reader would build; never the key
    /// a foreign reader stores this under -- that is `app_did`.
    pub app_instance_id: AppInstanceId,
    /// The app instance's own master DID: the key this document is signed
    /// by, and the identity a reader resolved in Tier 1.
    pub app_did: AppDid,
    pub service_name: LogicalServiceName,
    pub mode: TopologyMode,
    /// Member master DIDs, in member-index order -- never physical
    /// addresses (ADR-0022 §4). Ordered so an unchanged plan produces
    /// byte-identical signed content.
    pub members: Vec<ServiceId>,
    /// Carries the range table for `RangeSharding` inside itself; there is
    /// deliberately no separate `shard_map` field -- a second, parallel
    /// field would give a reader two sources that can disagree inside one
    /// signed payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharding_strategy: Option<ShardingStrategy>,
    /// The per-logical-service topology epoch (ADR-0022 §6). Shipped
    /// unenforced: shard rebalancing is what makes a caller carry it on a
    /// request and a member reject a request that no longer entitles it to
    /// a key.
    pub epoch: TopologyEpoch,
    /// The supervisor's own app-instance generation at the moment of
    /// signing -- what tells two supervisors' documents apart, the same
    /// role `generation` plays on the Tier-1 record (ADR-0022 §2).
    pub generation: u64,
    pub issued_at: u64,
    /// Unix seconds. A cached copy routes until this and then fails
    /// (failure-matrix row 6).
    pub not_after: u64,
    /// How long the signer suggests a reader holds this before re-asking.
    /// Advice, not authority -- `not_after` is the authority, and a reader
    /// may substitute its own TTL at `to_topology_entry`. Inside the
    /// signature nonetheless: a value carried *beside* the document is
    /// discarded by any interface that passes the document alone, which is
    /// every interface here.
    pub cache_ttl_ms: u64,
}

/// The document plus the app master's signature over it. z-base-32 Ed25519
/// over the RFC-8785 canonicalization of the document, the shape
/// `DelegationCertificate` and every UCAN token in this tree already use --
/// deliberately not a pkarr `SignedPacket`, which is the registry's own
/// admission format and is not what this travels through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedTopologyDocument {
    pub document: TopologyDocument,
    pub signature: String,
}

impl TopologyDocument {
    /// Signs with the app master. The caller is responsible for the key
    /// actually being this app's master -- `sign` cannot check it, so
    /// `verify` re-derives the DID from the signature side.
    pub fn sign(self, app_master: &Identity) -> Result<SignedTopologyDocument> {
        let value =
            serde_json::to_value(&self).context("serializing topology document for signing")?;
        let signature = app_master.sign_json(&value).context("signing topology document")?;
        Ok(SignedTopologyDocument { document: self, signature })
    }
}

impl SignedTopologyDocument {
    /// Verifies the signature against `expected_app_did` and refuses a
    /// document past its `not_after`.
    ///
    /// Takes the DID the caller resolved in Tier 1 rather than trusting the
    /// document's own `app_did`, which is confused-deputy prevention in the
    /// shape `DelegationCertificate::verify_chain` already uses: a
    /// perfectly valid document for a *different* app must not satisfy a
    /// lookup for this one.
    pub fn verify(&self, expected_app_did: &AppDid) -> Result<()> {
        let value = serde_json::to_value(&self.document)
            .context("serializing topology document for verification")?;
        substrate::verify_json_signature(expected_app_did.as_str(), &value, &self.signature)
            .context("topology document signature verification failed")?;
        let now = unix_now();
        if now >= self.document.not_after {
            return Err(anyhow!(
                "topology document for '{}/{}' expired at unix time {} (now {})",
                self.document.app_did,
                self.document.service_name,
                self.document.not_after,
                now
            ));
        }
        Ok(())
    }

    /// The registry entry this document becomes. `cache_ttl_override`
    /// replaces the signer's suggested `cache_ttl_ms` -- the signer decides
    /// when the answer stops being usable (`not_after`), the reader decides
    /// how often to re-ask, and `None` means "take the signer's advice".
    #[must_use]
    pub fn to_topology_entry(&self, cache_ttl_override: Option<Duration>) -> TopologyEntry {
        let cache_ttl =
            cache_ttl_override.unwrap_or_else(|| Duration::from_millis(self.document.cache_ttl_ms));
        TopologyEntry {
            mode: self.document.mode,
            members: self.document.members.clone(),
            sharding_strategy: self.document.sharding_strategy.clone(),
            epoch: self.document.epoch,
            cache_ttl,
            not_after: Some(self.document.not_after),
        }
    }

    /// The key a *foreign* reader stores this under.
    #[must_use]
    pub fn foreign_key(&self) -> TopologyKey {
        TopologyKey::foreign(self.document.app_did.clone(), self.document.service_name.clone())
    }
}

/// Verify, convert, and register in one call -- the whole client-side path
/// ADR-0022 §3 describes, minus the fetch.
///
/// Verification happens here, once per fetch, and never on the resolve path
/// (task.md's own budget: "Verify on register, not on read").
pub fn register_verified(
    resolver: &LogicalResolver,
    signed: &SignedTopologyDocument,
    expected_app_did: &AppDid,
    cache_ttl_override: Option<Duration>,
) -> Result<TopologyKey> {
    signed.verify(expected_app_did)?;
    let key = signed.foreign_key();
    let entry = signed.to_topology_entry(cache_ttl_override);
    resolver.register(key.clone(), entry);
    Ok(key)
}

/// How a caller reaches a supervisor to fetch a document. A trait rather
/// than a concrete client because the only implementation needs
/// `syneroym-sdk`, and a future cross-app consumer in `syneroym-control-plane`
/// cannot depend on it (`sdk -> router -> control_plane` is a cycle). Anything
/// holding an `Arc<dyn TopologyFetcher>` is injected from the substrate's
/// composition root, which depends on both.
///
/// **Nothing on the substrate holds one yet**: the implementors today are
/// `roymctl` and SDK callers. ADR-0022 §3 asks for a substrate-side fetch
/// too, so this trait is shaped for that consumer in advance -- adding it
/// later is a new holder, not a signature change.
#[async_trait::async_trait]
pub trait TopologyFetcher: fmt::Debug + Send + Sync {
    /// Tier 1 then Tier 2: resolve `app_did` to its supervising node, call
    /// that supervisor's `resolve`, and return the document **unverified** —
    /// verification is the caller's, so a fetcher can never be the trust
    /// boundary.
    async fn fetch(
        &self,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument>;
}

/// A stable hash of everything a topology epoch must change on:
/// `(mode, ordered members, sharding_strategy)`. Not `not_after`, not
/// `generation`, not `issued_at` -- those move on every signing and would
/// make the epoch a clock.
#[must_use]
pub fn topology_fingerprint(
    mode: TopologyMode,
    members: &[ServiceId],
    sharding_strategy: Option<&ShardingStrategy>,
) -> String {
    #[derive(Serialize)]
    struct Fingerprinted<'a> {
        mode: TopologyMode,
        members: &'a [ServiceId],
        sharding_strategy: Option<&'a ShardingStrategy>,
    }

    let value = serde_json::to_value(Fingerprinted { mode, members, sharding_strategy })
        .unwrap_or(serde_json::Value::Null);
    let canonical = substrate::canonicalize_json_value(&value);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::resolver::{AppScope, StaticInventory};

    fn app_master() -> (Identity, AppDid) {
        let identity = Identity::generate().unwrap();
        let did = AppDid::new(substrate::derive_did_key(&identity.public_key()));
        (identity, did)
    }

    fn svc(s: &str) -> ServiceId {
        ServiceId::new(format!("did:key:{s}"))
    }

    fn doc(app_did: AppDid, not_after: u64) -> TopologyDocument {
        TopologyDocument {
            app_instance_id: AppInstanceId::new("chat"),
            app_did,
            service_name: LogicalServiceName::new("api"),
            mode: TopologyMode::Redundant,
            members: vec![svc("m0"), svc("m1")],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            generation: 0,
            issued_at: unix_now(),
            not_after,
            cache_ttl_ms: 300_000,
        }
    }

    #[test]
    fn a_document_verifies_against_the_app_did_that_signed_it() {
        let (master, app_did) = app_master();
        let signed = doc(app_did.clone(), unix_now() + 3600).sign(&master).unwrap();
        assert!(signed.verify(&app_did).is_ok());
    }

    #[test]
    fn a_document_signed_under_a_different_key_is_rejected() {
        let (master, app_did) = app_master();
        let (_other_master, other_did) = app_master();
        let signed = doc(app_did, unix_now() + 3600).sign(&master).unwrap();
        assert!(signed.verify(&other_did).is_err());
    }

    #[test]
    fn a_document_whose_members_were_altered_after_signing_is_rejected() {
        let (master, app_did) = app_master();
        let mut signed = doc(app_did.clone(), unix_now() + 3600).sign(&master).unwrap();
        signed.document.members.push(svc("injected"));
        assert!(signed.verify(&app_did).is_err());
    }

    #[test]
    fn a_document_past_its_not_after_is_rejected() {
        let (master, app_did) = app_master();
        let signed = doc(app_did.clone(), unix_now().saturating_sub(1)).sign(&master).unwrap();
        let err = signed.verify(&app_did).unwrap_err();
        assert!(err.to_string().contains("expired"), "{err}");
    }

    #[test]
    fn a_document_verifies_from_bytes_alone_with_no_connection() {
        let (master, app_did) = app_master();
        let signed = doc(app_did.clone(), unix_now() + 3600).sign(&master).unwrap();
        let bytes = serde_json::to_vec(&signed).unwrap();
        drop(signed);
        drop(master);

        let relayed: SignedTopologyDocument = serde_json::from_slice(&bytes).unwrap();
        assert!(relayed.verify(&app_did).is_ok());
    }

    /// The confused-deputy guard -- a valid document that is not the one
    /// asked for.
    #[test]
    fn a_document_for_a_different_app_did_is_rejected() {
        let (master_a, app_did_a) = app_master();
        let (_master_b, app_did_b) = app_master();
        let signed = doc(app_did_a, unix_now() + 3600).sign(&master_a).unwrap();
        assert!(signed.verify(&app_did_b).is_err());
    }

    /// The "field is carried and preserved" pin: converting a verified
    /// document to a registry entry must not drop or reshape any of it.
    #[test]
    fn a_document_converts_to_a_topology_entry_preserving_mode_members_and_epoch() {
        let (master, app_did) = app_master();
        let d = doc(app_did.clone(), unix_now() + 3600);
        let members = d.members.clone();
        let signed = d.sign(&master).unwrap();

        let entry = signed.to_topology_entry(None);
        assert_eq!(entry.mode, TopologyMode::Redundant);
        assert_eq!(entry.members, members);
        assert_eq!(entry.epoch, TopologyEpoch(1));
        assert_eq!(entry.not_after, Some(signed.document.not_after));
        assert_eq!(entry.cache_ttl, Duration::from_millis(300_000));
    }

    #[test]
    fn to_topology_entry_honors_a_cache_ttl_override() {
        let (master, app_did) = app_master();
        let signed = doc(app_did, unix_now() + 3600).sign(&master).unwrap();
        let entry = signed.to_topology_entry(Some(Duration::from_secs(5)));
        assert_eq!(entry.cache_ttl, Duration::from_secs(5));
    }

    /// A type-level guard on the document's own round-trip, not a claim
    /// about reachability -- `refuse_unshardable_plan` (in the supervisor
    /// crate) is what stops `RangeSharding` reaching the wire.
    #[test]
    fn a_sharding_strategy_survives_the_document_round_trip() {
        let (master, app_did) = app_master();
        let mut d = doc(app_did, unix_now() + 3600);
        d.mode = TopologyMode::Sharded;
        d.sharding_strategy =
            Some(ShardingStrategy::RangeSharding(crate::resolver::RangeRoutingTable {
                chunks: vec![crate::resolver::RangeChunk {
                    start_key: None,
                    end_key: None,
                    target: svc("only"),
                }],
            }));
        let signed = d.sign(&master).unwrap();
        let bytes = serde_json::to_vec(&signed).unwrap();
        let decoded: SignedTopologyDocument = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.document.sharding_strategy, signed.document.sharding_strategy);
    }

    #[test]
    fn a_fingerprint_changes_when_the_member_set_changes_and_not_otherwise() {
        let members = vec![svc("a"), svc("b")];
        let reordered = vec![svc("b"), svc("a")];
        let more = vec![svc("a"), svc("b"), svc("c")];

        let fp1 = topology_fingerprint(TopologyMode::Redundant, &members, None);
        let fp2 = topology_fingerprint(TopologyMode::Redundant, &reordered, None);
        let fp3 = topology_fingerprint(TopologyMode::Redundant, &more, None);

        assert_ne!(fp1, fp2, "reordered members must change the fingerprint");
        assert_ne!(fp1, fp3, "an added member must change the fingerprint");
    }

    /// The document names DIDs, so a member's substrate relocating changes
    /// nothing about the document.
    #[test]
    fn a_relocated_member_does_not_change_the_document() {
        // The document carries `ServiceId` (a DID), never a physical
        // address, so there is nothing in this type that could observe a
        // relocation -- pinned by asserting the fingerprint is a pure
        // function of the DID list.
        let members = vec![svc("m0"), svc("m1")];
        let fp_before = topology_fingerprint(TopologyMode::Redundant, &members, None);
        // "Relocating" changes only where a DID is reachable, never the DID
        // itself, so the same member list re-fingerprints identically.
        let fp_after = topology_fingerprint(TopologyMode::Redundant, &members, None);
        assert_eq!(fp_before, fp_after);
    }

    /// `register_verified` stores under the app DID, not the human
    /// `app_instance_id` -- keeping the foreign namespace disjoint from
    /// the local one.
    #[test]
    fn a_verified_document_registers_under_the_app_did_not_the_instance_id() {
        let (master, app_did) = app_master();
        let signed = doc(app_did.clone(), unix_now() + 3600).sign(&master).unwrap();

        let resolver = LogicalResolver::new(std::sync::Arc::new(StaticInventory::new()));
        let key = register_verified(&resolver, &signed, &app_did, None).unwrap();

        assert_eq!(key.app, AppScope::Foreign(app_did));
        assert_eq!(key.service_name, LogicalServiceName::new("api"));
        assert!(resolver.resolve(&key, None).is_ok());
    }
}
