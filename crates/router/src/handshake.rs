//! Handshake Authorization and Preamble Verification
//!
//! Confirms the client's temporary identity key is authorized by their master
//! DID and that the caller holds the private key.

use std::time::Duration;

use anyhow::{Result, anyhow};
use ed25519_dalek::VerifyingKey;
pub use syneroym_core::dht_registry::MasterAnchorResolver;
use syneroym_identity::{DelegationCertificate, delegation::TRANSPORT_SCOPES, substrate};
use syneroym_rpc::OnBehalfProof;
use tokio::time;

use crate::{
    RoutePreamble,
    preamble::{PROOF_WINDOW_SECS, check_signature},
};

#[derive(Debug)]
pub struct VerifiedIdentity {
    pub master_did: String,
    pub temporary_did: String,
}

#[derive(Debug)]
pub struct HandshakeVerifier;

/// The parts of one signed proof, from a preamble or from `on_behalf`.
struct Presented<'a> {
    pubkey_hex: &'a str,
    delegation: Option<DelegationCertificate>,
    service_id: &'a str,
    interface: &'a str,
    ts: Option<u64>,
    sig_hex: Option<&'a str>,
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl HandshakeVerifier {
    /// Wall-clock form, kept for existing callers.
    pub async fn verify_preamble(
        preamble: &RoutePreamble,
        resolver: &dyn MasterAnchorResolver,
    ) -> Result<VerifiedIdentity> {
        Self::verify_preamble_at(preamble, resolver, now_secs()).await
    }

    pub async fn verify_preamble_at(
        preamble: &RoutePreamble,
        resolver: &dyn MasterAnchorResolver,
        now_secs: u64,
    ) -> Result<VerifiedIdentity> {
        let pubkey_hex = preamble
            .pubkey
            .as_deref()
            .ok_or_else(|| anyhow!("Missing client public key (pubkey) in preamble"))?;
        verify_presented(
            Presented {
                pubkey_hex,
                delegation: preamble.delegation.clone(),
                service_id: &preamble.service_id,
                interface: &preamble.interface,
                ts: preamble.ts,
                sig_hex: preamble.sig.as_deref(),
            },
            resolver,
            now_secs,
        )
        .await
    }

    pub async fn verify_on_behalf_at(
        proof: &OnBehalfProof,
        resolver: &dyn MasterAnchorResolver,
        now_secs: u64,
    ) -> Result<VerifiedIdentity> {
        let delegation = proof
            .delegation
            .as_deref()
            .map(DelegationCertificate::from_json)
            .transpose()
            .map_err(|e| anyhow!("unreadable on_behalf certificate: {e}"))?;
        verify_presented(
            Presented {
                pubkey_hex: &proof.pubkey,
                delegation,
                service_id: &proof.service_id,
                interface: &proof.interface,
                ts: Some(proof.ts),
                sig_hex: Some(&proof.sig),
            },
            resolver,
            now_secs,
        )
        .await
    }
}

async fn verify_presented(
    p: Presented<'_>,
    resolver: &dyn MasterAnchorResolver,
    now_secs: u64,
) -> Result<VerifiedIdentity> {
    let key = decode_ed25519(p.pubkey_hex)?;
    let ts = p.ts.ok_or_else(|| anyhow!("preamble names a pubkey but has no ts"))?;
    let sig = p.sig_hex.ok_or_else(|| anyhow!("preamble names a pubkey but has no sig"))?;
    if ts.abs_diff(now_secs) > PROOF_WINDOW_SECS {
        return Err(anyhow!(
            "preamble ts is more than {PROOF_WINDOW_SECS} s from this node's clock"
        ));
    }
    check_signature(&key, p.service_id, p.interface, ts, sig)?;
    let temporary_did = substrate::derive_did_key(&key);
    let Some(cert) = p.delegation else {
        return Ok(VerifiedIdentity { master_did: temporary_did.clone(), temporary_did });
    };
    // `master_did` is read from the certificate itself, so verifying against
    // `cert.master_did` is a self-consistency check on this path --
    // the connection asserts "I am delegated by M" and M is whatever
    // the certificate says; binding to a target is resolved
    // downstream on `master_did`. The scope check is what actually
    // enforces something here: both a person's delegated device key
    // and a service instance route a connection under a master's
    // identity, so the ingress accepts either transport scope and
    // leaves narrowing to whoever uses the connection.
    cert.verify_at(&cert.master_did, &TRANSPORT_SCOPES, now_secs)?;
    if cert.temporary_did != temporary_did {
        return Err(anyhow!(
            "Delegation certificate temporary_did does not match preamble pubkey DID"
        ));
    }
    let anchor =
        time::timeout(Duration::from_secs(5), resolver.resolve_master_anchor(&cert.master_did))
            .await
            .map_err(|_| anyhow!("Timeout resolving master anchor"))??;

    if anchor.revoked_keys.contains(&temporary_did) {
        return Err(anyhow!(
            "Temporary DID {temporary_did} has been revoked by master {}",
            cert.master_did
        ));
    }

    Ok(VerifiedIdentity { master_did: cert.master_did.clone(), temporary_did })
}

fn decode_ed25519(pubkey_hex: &str) -> Result<VerifyingKey> {
    let bytes =
        hex::decode(pubkey_hex).map_err(|e| anyhow!("Invalid hex in client pubkey: {e}"))?;
    let array: [u8; 32] = bytes.try_into().map_err(|_| anyhow!("Invalid client pubkey length"))?;
    VerifyingKey::from_bytes(&array).map_err(|e| anyhow!("Invalid client pubkey: {e}"))
}

#[cfg(test)]
mod tests;
