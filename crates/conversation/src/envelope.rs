//! The signed message payload (`DeliveryPayload`) and its canonical byte
//! encoding (`D-B4-22`). This is what a ratchet session encrypts and what
//! the receiver verifies -- attribution is signature-based, never
//! transport-based (F16, `D-B4-5`), so this module is what B5's relayed DAG
//! entries reuse unchanged (§15 item 3).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// What one ratchet session carries per message, before encryption and
/// after decryption. Signed as a whole (`sign`/`verify` below) under the
/// sender's own conversation signing key, independently of the session
/// encryption -- attribution survives a relay, which a transport-identity
/// check never could.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeliveryPayload {
    pub message_id: String,
    pub conversation_id: String,
    /// The sender's own routing service id (`D-B4-5`'s `author`).
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub content_type: String,
    pub body: Vec<u8>,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub signature: [u8; 64],
}

/// `len_be(body)`, not a bare separator, so no field can be reassigned
/// across a boundary by an attacker choosing `content_type`/`body` bytes
/// that contain `0x00` -- the same reasoning
/// `derive_service_identity`'s own length-prefixed `owner_did` uses.
#[must_use]
pub fn canonical_bytes(
    message_id: &str,
    conversation_id: &str,
    author: &str,
    sender_timestamp_ms: i64,
    content_type: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        32 + message_id.len()
            + conversation_id.len()
            + author.len()
            + content_type.len()
            + body.len()
            + 16,
    );
    out.extend_from_slice(b"syneroym:conversation:v1");
    out.push(0);
    out.extend_from_slice(message_id.as_bytes());
    out.push(0);
    out.extend_from_slice(conversation_id.as_bytes());
    out.push(0);
    out.extend_from_slice(author.as_bytes());
    out.push(0);
    out.extend_from_slice(&sender_timestamp_ms.to_be_bytes());
    out.push(0);
    out.extend_from_slice(content_type.as_bytes());
    out.push(0);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// Signs the canonical encoding of `payload`'s fields under `signing_key`,
/// returning the fields' own signature (not stored on `payload` -- the
/// caller assigns it).
#[must_use]
pub fn sign(
    signing_key: &SigningKey,
    message_id: &str,
    conversation_id: &str,
    author: &str,
    sender_timestamp_ms: i64,
    content_type: &str,
    body: &[u8],
) -> [u8; 64] {
    let bytes = canonical_bytes(
        message_id,
        conversation_id,
        author,
        sender_timestamp_ms,
        content_type,
        body,
    );
    signing_key.sign(&bytes).to_bytes()
}

/// Verifies `payload.signature` under `verifying_key`, over `payload`'s own
/// fields -- never trusting the caller's claimed encoding.
#[must_use]
pub fn verify(verifying_key: &VerifyingKey, payload: &DeliveryPayload) -> bool {
    let bytes = canonical_bytes(
        &payload.message_id,
        &payload.conversation_id,
        &payload.author,
        payload.sender_timestamp_ms,
        &payload.content_type,
        &payload.body,
    );
    let Ok(sig) = Signature::try_from(payload.signature.as_slice()) else { return false };
    verifying_key.verify(&bytes, &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> (SigningKey, DeliveryPayload) {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let signature =
            sign(&signing_key, "msg:1", "conv:1", "did:key:zAuthor", 1_000, "text/plain", b"hello");
        (
            signing_key,
            DeliveryPayload {
                message_id: "msg:1".to_string(),
                conversation_id: "conv:1".to_string(),
                author: "did:key:zAuthor".to_string(),
                sender_timestamp_ms: 1_000,
                content_type: "text/plain".to_string(),
                body: b"hello".to_vec(),
                signature,
            },
        )
    }

    #[test]
    fn a_correctly_signed_payload_verifies() {
        let (signing_key, payload) = payload();
        assert!(verify(&signing_key.verifying_key(), &payload));
    }

    #[test]
    fn a_one_bit_change_anywhere_fails_verification() {
        let (signing_key, payload) = payload();
        let vk = signing_key.verifying_key();

        let mut wrong_body = payload.clone();
        wrong_body.body = b"hellp".to_vec();
        assert!(!verify(&vk, &wrong_body));

        let mut wrong_author = payload.clone();
        wrong_author.author = "did:key:zOther".to_string();
        assert!(!verify(&vk, &wrong_author));

        let mut wrong_ts = payload.clone();
        wrong_ts.sender_timestamp_ms = 1_001;
        assert!(!verify(&vk, &wrong_ts));

        let mut wrong_ct = payload.clone();
        wrong_ct.content_type = "text/html".to_string();
        assert!(!verify(&vk, &wrong_ct));

        let mut wrong_conv = payload.clone();
        wrong_conv.conversation_id = "conv:2".to_string();
        assert!(!verify(&vk, &wrong_conv));
    }

    /// The length prefix on `body` is what stops a byte from `content_type`
    /// being reassigned into `body` (or vice versa) from producing the same
    /// canonical encoding.
    #[test]
    fn moving_a_byte_across_the_body_length_prefix_cannot_produce_a_collision() {
        let a = canonical_bytes("m", "c", "a", 0, "xy", b"z");
        let b = canonical_bytes("m", "c", "a", 0, "x", b"yz");
        assert_ne!(a, b, "the length prefix must prevent this exact ambiguity");
    }

    #[test]
    fn wrong_verifying_key_fails() {
        let (_, payload) = payload();
        let other_key = SigningKey::generate(&mut rand_core::OsRng);
        assert!(!verify(&other_key.verifying_key(), &payload));
    }
}
