//! DAG entry representation, canonical byte encoding, AEAD sealing,
//! and wire structures for group conversation delivery and synchronization.

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

pub const MAX_PARENTS: usize = 8;
pub const GROUP_KEY_CONTENT_TYPE: &str = "application/vnd.syneroym.group-key+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Message,
    Membership,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct MembershipPayload {
    pub action: String,
    pub subject_address: String,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub subject_sig_key: [u8; 32],
    pub new_epoch: u64,
    pub member_list_hash: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WireEntry {
    pub entry_id: String,
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub epoch: u64,
    pub kind: EntryKind,
    pub parents: Vec<String>,
    pub ciphertext: Option<Vec<u8>>,
    pub nonce: Option<[u8; 12]>,
    pub payload: Option<MembershipPayload>,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerAssertion {
    pub address: String,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub sig_key: [u8; 32],
    pub timestamp_ms: i64,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub nonce: [u8; 16],
    #[serde(with = "crate::wire::fixed_bytes")]
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupPushRequest {
    pub from: PeerAssertion,
    pub group: String,
    pub entries: Vec<WireEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupPushAck {
    pub accepted: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupSyncRequest {
    pub from: PeerAssertion,
    pub group: String,
    pub after_seq: i64,
    pub limit: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupSyncResponse {
    pub entries: Vec<WireEntry>,
    /// Store sequence number of each entry in `entries`, same order and
    /// length. Lets the caller advance its cursor to exactly the highest
    /// sequence it actually applied, rather than to `next_seq` regardless
    /// of whether every entry in the page validated.
    pub seqs: Vec<i64>,
    pub next_seq: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupKeyPayload {
    pub group_id: String,
    pub epoch: u64,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub key: [u8; 32],
    pub members: Vec<String>,
    pub owner: String,
}

/// Builds the canonical entry header prefix up to and including the parents
/// block. This prefix serves as the AAD for AEAD sealing.
#[must_use]
pub fn canonical_entry_prefix(
    conversation_id: &str,
    author: &str,
    sender_timestamp_ms: i64,
    epoch: u64,
    kind: EntryKind,
    parents: &[String],
) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(64 + conversation_id.len() + author.len() + parents.len() * 40);
    out.extend_from_slice(b"syneroym:conversation:dag:v1");
    out.push(0);
    out.extend_from_slice(conversation_id.as_bytes());
    out.push(0);
    out.extend_from_slice(author.as_bytes());
    out.push(0);
    out.extend_from_slice(&sender_timestamp_ms.to_be_bytes());
    out.push(0);
    out.extend_from_slice(&epoch.to_be_bytes());
    out.push(0);
    match kind {
        EntryKind::Message => out.extend_from_slice(b"message"),
        EntryKind::Membership => out.extend_from_slice(b"membership"),
    }
    out.push(0);
    out.extend_from_slice(&(parents.len() as u64).to_be_bytes());
    for parent in parents {
        out.extend_from_slice(&(parent.len() as u64).to_be_bytes());
        out.extend_from_slice(parent.as_bytes());
    }
    out
}

/// Builds the complete canonical bytes for a DAG entry, which is what the
/// author signs and what the entry ID is derived from.
#[must_use]
pub fn canonical_entry_bytes(entry: &WireEntry) -> Vec<u8> {
    let mut out = canonical_entry_prefix(
        &entry.conversation_id,
        &entry.author,
        entry.sender_timestamp_ms,
        entry.epoch,
        entry.kind,
        &entry.parents,
    );
    match entry.kind {
        EntryKind::Message => {
            let ct = entry.ciphertext.as_deref().unwrap_or(&[]);
            out.extend_from_slice(&(ct.len() as u64).to_be_bytes());
            out.extend_from_slice(ct);
            if let Some(nonce) = &entry.nonce {
                out.extend_from_slice(&(nonce.len() as u64).to_be_bytes());
                out.extend_from_slice(nonce);
            } else {
                out.extend_from_slice(&0u64.to_be_bytes());
            }
        }
        EntryKind::Membership => {
            // `MembershipPayload` is plain data (strings, fixed-size byte
            // arrays, a u64) with no type that can fail to serialize —
            // an error here means the struct grew a field that can't
            // round-trip, a real bug. Failing loud beats silently
            // canonicalizing to empty bytes, which would still produce a
            // signature, just not one over the payload actually being sent.
            #[allow(clippy::expect_used)]
            let payload_bytes = entry
                .payload
                .as_ref()
                .map(|p| serde_json::to_vec(p).expect("MembershipPayload always serializes"))
                .unwrap_or_default();
            out.extend_from_slice(&(payload_bytes.len() as u64).to_be_bytes());
            out.extend_from_slice(&payload_bytes);
            out.extend_from_slice(&0u64.to_be_bytes());
        }
    }
    out
}

#[must_use]
pub fn sign_entry(signing_key: &SigningKey, header: &[u8]) -> [u8; 64] {
    signing_key.sign(header).to_bytes()
}

#[must_use]
pub fn verify_entry(verifying_key: &VerifyingKey, header: &[u8], signature: &[u8; 64]) -> bool {
    let sig = Signature::from_bytes(signature);
    verifying_key.verify_strict(header, &sig).is_ok()
}

pub fn seal(
    epoch_key: &[u8; 32],
    header_prefix: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; 12]), String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(epoch_key));
    let mut nonce_bytes = [0u8; 12];
    rand::Rng::fill(&mut rand::rng(), &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let payload = Payload { msg: plaintext, aad: header_prefix };
    let ciphertext = cipher.encrypt(nonce, payload).map_err(|e| format!("seal error: {e}"))?;
    Ok((ciphertext, nonce_bytes))
}

pub fn open(
    epoch_key: &[u8; 32],
    header_prefix: &[u8],
    nonce: &[u8; 12],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(epoch_key));
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload { msg: ciphertext, aad: header_prefix };
    cipher.decrypt(nonce, payload).map_err(|e| format!("open error: {e}"))
}

#[must_use]
pub fn encode_body(content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + content_type.len() + body.len());
    out.extend_from_slice(&(content_type.len() as u64).to_be_bytes());
    out.extend_from_slice(content_type.as_bytes());
    out.extend_from_slice(body);
    out
}

pub fn decode_body(encoded: &[u8]) -> Result<(String, Vec<u8>), String> {
    if encoded.len() < 8 {
        return Err("encoded body too short".to_string());
    }
    let Ok(len_bytes) = encoded[..8].try_into() else {
        return Err("invalid header bytes".to_string());
    };
    let ct_len = u64::from_be_bytes(len_bytes) as usize;
    if encoded.len() < 8 + ct_len {
        return Err("encoded body truncated".to_string());
    }
    let content_type = std::str::from_utf8(&encoded[8..8 + ct_len])
        .map_err(|e| format!("invalid utf-8 in content_type: {e}"))?
        .to_string();
    let body = encoded[8 + ct_len..].to_vec();
    Ok((content_type, body))
}

#[must_use]
pub fn canonical_peer_assertion_bytes(
    group_id: &str,
    address: &str,
    sig_key: &[u8; 32],
    timestamp_ms: i64,
    nonce: &[u8; 16],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + group_id.len() + address.len() + 32 + 8 + 16);
    out.extend_from_slice(b"syneroym:conversation:peer-assertion:v1");
    out.push(0);
    out.extend_from_slice(group_id.as_bytes());
    out.push(0);
    out.extend_from_slice(address.as_bytes());
    out.push(0);
    out.extend_from_slice(sig_key);
    out.push(0);
    out.extend_from_slice(&timestamp_ms.to_be_bytes());
    out.push(0);
    out.extend_from_slice(nonce);
    out
}

#[must_use]
pub fn sign_peer_assertion(
    signing_key: &SigningKey,
    address: &str,
    group_id: &str,
    timestamp_ms: i64,
    nonce: &[u8; 16],
) -> PeerAssertion {
    let sig_key = signing_key.verifying_key().to_bytes();
    let bytes = canonical_peer_assertion_bytes(group_id, address, &sig_key, timestamp_ms, nonce);
    let signature = signing_key.sign(&bytes).to_bytes();
    PeerAssertion { address: address.to_string(), sig_key, timestamp_ms, nonce: *nonce, signature }
}

pub fn verify_peer_assertion(
    verifying_key: &VerifyingKey,
    group_id: &str,
    assertion: &PeerAssertion,
) -> bool {
    let bytes = canonical_peer_assertion_bytes(
        group_id,
        &assertion.address,
        &assertion.sig_key,
        assertion.timestamp_ms,
        &assertion.nonce,
    );
    let Ok(sig) = Signature::try_from(assertion.signature.as_slice()) else {
        return false;
    };
    verifying_key.verify(&bytes, &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::derive_entry_id;

    fn sample_message_entry() -> (SigningKey, [u8; 32], WireEntry) {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let epoch_key = [7u8; 32];
        let prefix =
            canonical_entry_prefix("conv:group1", "svc:alice", 1000, 1, EntryKind::Message, &[]);
        let (ciphertext, nonce) = seal(&epoch_key, &prefix, b"hello group").unwrap();
        let mut entry = WireEntry {
            entry_id: String::new(),
            conversation_id: "conv:group1".to_string(),
            author: "svc:alice".to_string(),
            sender_timestamp_ms: 1000,
            epoch: 1,
            kind: EntryKind::Message,
            parents: vec![],
            ciphertext: Some(ciphertext),
            nonce: Some(nonce),
            payload: None,
            signature: [0u8; 64],
        };
        let header = canonical_entry_bytes(&entry);
        entry.entry_id = derive_entry_id(&header);
        entry.signature = sign_entry(&signing_key, &header);
        (signing_key, epoch_key, entry)
    }

    #[test]
    fn a_one_bit_change_anywhere_in_the_header_fails_verification() {
        let (sk, _, entry) = sample_message_entry();
        let vk = sk.verifying_key();
        let header = canonical_entry_bytes(&entry);
        assert!(verify_entry(&vk, &header, &entry.signature));

        let mut tampered_header = header.clone();
        for i in 0..tampered_header.len() {
            tampered_header[i] ^= 0x01;
            assert!(!verify_entry(&vk, &tampered_header, &entry.signature));
            tampered_header[i] ^= 0x01;
        }
    }

    #[test]
    fn moving_a_byte_across_the_parents_length_prefix_cannot_produce_a_collision() {
        let parents_a = vec!["ent:12".to_string(), "ent:34".to_string()];
        let prefix_a =
            canonical_entry_prefix("conv:g", "a", 100, 1, EntryKind::Message, &parents_a);

        let parents_b = vec!["ent:1".to_string(), "2ent:34".to_string()];
        let prefix_b =
            canonical_entry_prefix("conv:g", "a", 100, 1, EntryKind::Message, &parents_b);

        assert_ne!(prefix_a, prefix_b);
    }

    #[test]
    fn sealed_ciphertext_does_not_open_under_a_different_epoch_key() {
        let (_, epoch_key, entry) = sample_message_entry();
        let prefix = canonical_entry_prefix(
            &entry.conversation_id,
            &entry.author,
            entry.sender_timestamp_ms,
            entry.epoch,
            entry.kind,
            &entry.parents,
        );
        let ct = entry.ciphertext.as_ref().unwrap();
        let nonce = entry.nonce.as_ref().unwrap();

        let opened = open(&epoch_key, &prefix, nonce, ct);
        assert!(opened.is_ok());
        assert_eq!(opened.unwrap(), b"hello group");

        let mut diff_key = epoch_key;
        diff_key[0] ^= 0x01;
        assert!(open(&diff_key, &prefix, nonce, ct).is_err());
    }

    #[test]
    fn sealed_ciphertext_does_not_open_when_the_header_aad_is_altered() {
        let (_, epoch_key, entry) = sample_message_entry();
        let prefix = canonical_entry_prefix(
            &entry.conversation_id,
            &entry.author,
            entry.sender_timestamp_ms,
            entry.epoch,
            entry.kind,
            &entry.parents,
        );
        let ct = entry.ciphertext.as_ref().unwrap();
        let nonce = entry.nonce.as_ref().unwrap();

        let mut altered_prefix = prefix.clone();
        altered_prefix.push(0);
        assert!(open(&epoch_key, &altered_prefix, nonce, ct).is_err());
    }

    #[test]
    fn peer_assertion_signs_and_verifies() {
        let sk = SigningKey::generate(&mut rand_core::OsRng);
        let vk = sk.verifying_key();
        let nonce = [42u8; 16];
        let assertion = sign_peer_assertion(&sk, "svc:alice", "conv:group1", 1000, &nonce);
        assert!(verify_peer_assertion(&vk, "conv:group1", &assertion));

        assert!(!verify_peer_assertion(&vk, "conv:othergroup", &assertion));
    }
}
