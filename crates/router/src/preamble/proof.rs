use std::{fmt, sync::Arc};

use anyhow::{Result, anyhow};
use ed25519_dalek::{Signature, VerifyingKey};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_rpc::OnBehalfProof;

use super::RoutePreamble;

/// Domain tag of the signed bytes (ADR-0002 A1).
const PROOF_TAG: &[u8] = b"syneroym-preamble-v1";

/// How far a preamble's `ts` may be from the receiver's clock.
pub const PROOF_WINDOW_SECS: u64 = 15 * 60;

/// The bytes a caller signs: the tag, `service_id`, `interface` and `ts`
/// as decimal text, each as a 4-byte big-endian length and then the
/// bytes, so no value can be read as a separator.
pub(crate) fn proof_bytes(service_id: &str, interface: &str, ts: u64) -> Vec<u8> {
    let ts_str = ts.to_string();
    let fields = [PROOF_TAG, service_id.as_bytes(), interface.as_bytes(), ts_str.as_bytes()];
    let total_len = fields.iter().map(|f| 4 + f.len()).sum();
    let mut out = Vec::with_capacity(total_len);
    for field in fields {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field);
    }
    out
}

impl RoutePreamble {
    /// Names `key` as the caller and proves it holds it: sets `pubkey`,
    /// `ts` and `sig` over this preamble's `service_id` and `interface`.
    /// Call it after the route fields are final.
    pub fn sign(&mut self, key: &Identity, now_secs: u64) {
        self.pubkey = Some(hex::encode(key.public_key().to_bytes()));
        self.ts = Some(now_secs);
        let sig = key.sign(&proof_bytes(&self.service_id, &self.interface, now_secs));
        self.sig = Some(hex::encode(sig.to_bytes()));
    }
}

/// Checks `sig_hex` by `key` over the proof bytes.
pub(crate) fn check_signature(
    key: &VerifyingKey,
    service_id: &str,
    interface: &str,
    ts: u64,
    sig_hex: &str,
) -> Result<()> {
    let sig_bytes = hex::decode(sig_hex).map_err(|e| anyhow!("invalid signature hex: {e}"))?;
    let sig_array: [u8; 64] =
        sig_bytes.try_into().map_err(|_| anyhow!("signature must be 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_array);
    let bytes = proof_bytes(service_id, interface, ts);
    key.verify_strict(&bytes, &signature)
        .map_err(|e| anyhow!("preamble signature verification failed: {e}"))
}

/// Who an outbound call presents itself as. Chosen once per call, and
/// signed into the preamble on every attempt.
#[derive(Clone)]
pub struct OutboundIdentity {
    pub key: Arc<Identity>,
    pub delegation: Option<DelegationCertificate>,
    pub on_behalf: Option<OnBehalfProof>,
}

impl OutboundIdentity {
    pub fn key_only(key: Arc<Identity>) -> Self {
        Self { key, delegation: None, on_behalf: None }
    }

    /// `route` with this identity set and signed at `now_secs`.
    #[must_use]
    pub fn apply(&self, route: &RoutePreamble, now_secs: u64) -> RoutePreamble {
        let mut p = route.clone();
        p.delegation = self.delegation.clone();
        p.on_behalf = self.on_behalf.clone();
        p.sign(&self.key, now_secs);
        p
    }
}

impl fmt::Debug for OutboundIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key_did = substrate::derive_did_key(&self.key.public_key());
        let master_did = self.delegation.as_ref().map(|c| c.master_did.as_str()).unwrap_or("-");
        let on_behalf = if self.on_behalf.is_some() { "present" } else { "absent" };
        f.debug_struct("OutboundIdentity")
            .field("key_did", &key_did)
            .field("master_did", &master_did)
            .field("on_behalf", &on_behalf)
            .finish()
    }
}
