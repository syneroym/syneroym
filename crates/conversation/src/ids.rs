//! `conversation-id`/`message-id` derivation.
//!
//! `conversation-id` is derived, not merely host-minted: it must come out
//! identically on both ends of a 1:1 conversation from nothing but the two
//! participants' own addresses. `message-id` is generated once, by the
//! sender at `send`, and carried inside the signed envelope — the
//! receiver never re-derives it, only verifies the signature over it.

use blake3::Hasher;

/// Order-independent over the address pair, so both participants compute
/// the same id regardless of who is "sender" and who is "receiver" for a
/// given call.
#[must_use]
pub fn derive_conversation_id(a: &str, b: &str) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Hasher::new();
    hasher.update(b"direct");
    hasher.update(&[0u8]);
    hasher.update(lo.as_bytes());
    hasher.update(&[0u8]);
    hasher.update(hi.as_bytes());
    format!("conv:{}", hex::encode(hasher.finalize().as_bytes()))
}

/// Content-derived and stable across delivery attempts: the fence the
/// receiver's store and the RPC dedup layer both rest on. `nonce` makes two
/// otherwise-identical sends (same author, conversation, timestamp,
/// content-type, and body -- possible if a guest resends the same text
/// within the same millisecond) distinguishable rather than colliding.
#[must_use]
pub fn derive_message_id(
    author: &str,
    conversation_id: &str,
    sender_timestamp_ms: i64,
    content_type: &str,
    body: &[u8],
    nonce: &[u8; 16],
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(author.as_bytes());
    hasher.update(&[0u8]);
    hasher.update(conversation_id.as_bytes());
    hasher.update(&[0u8]);
    hasher.update(&sender_timestamp_ms.to_be_bytes());
    hasher.update(&[0u8]);
    hasher.update(content_type.as_bytes());
    hasher.update(&[0u8]);
    hasher.update(body);
    hasher.update(nonce);
    format!("msg:{}", hex::encode(hasher.finalize().as_bytes()))
}

/// Minted by the owner, adopted verbatim by every other member. Not
/// derived from the member list: membership changes and the id must not.
#[must_use]
pub fn derive_group_id(owner_address: &str, created_at_ms: i64, nonce: &[u8; 16]) -> String {
    let mut h = Hasher::new();
    h.update(b"group");
    h.update(&[0u8]);
    h.update(owner_address.as_bytes());
    h.update(&[0u8]);
    h.update(&created_at_ms.to_be_bytes());
    h.update(nonce);
    format!("conv:{}", hex::encode(h.finalize().as_bytes()))
}

/// Content-derived over the entry's own signed header, so two substrates
/// that hold the same entry compute the same id and dedup on it.
#[must_use]
pub fn derive_entry_id(header: &[u8]) -> String {
    format!("ent:{}", hex::encode(blake3::hash(header).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_id_differs_for_the_same_owner_at_the_same_millisecond() {
        let owner = "svc-owner";
        let ts = 123_456_789;
        let id1 = derive_group_id(owner, ts, &[1u8; 16]);
        let id2 = derive_group_id(owner, ts, &[2u8; 16]);
        assert_ne!(id1, id2);
        assert!(id1.starts_with("conv:"));
    }

    #[test]
    fn entry_id_is_stable_for_identical_headers() {
        let header = b"header-bytes-for-test";
        let id1 = derive_entry_id(header);
        let id2 = derive_entry_id(header);
        assert_eq!(id1, id2);
        assert!(id1.starts_with("ent:"));
    }

    #[test]
    fn conversation_id_is_order_independent_and_identical_on_both_sides() {
        let a_side = derive_conversation_id("did:key:zA", "did:key:zB");
        let b_side = derive_conversation_id("did:key:zB", "did:key:zA");
        assert_eq!(a_side, b_side);
        assert!(a_side.starts_with("conv:"));
    }

    #[test]
    fn conversation_id_differs_for_a_different_pair() {
        let ab = derive_conversation_id("did:key:zA", "did:key:zB");
        let ac = derive_conversation_id("did:key:zA", "did:key:zC");
        assert_ne!(ab, ac);
    }

    #[test]
    fn message_id_changes_with_the_nonce_and_is_stable_otherwise() {
        let base = |nonce: &[u8; 16]| {
            derive_message_id("did:key:zA", "conv:xyz", 1_000, "text/plain", b"hello", nonce)
        };
        let id1 = base(&[1u8; 16]);
        let id2 = base(&[2u8; 16]);
        assert_ne!(id1, id2, "different nonce must derive a different message id");

        let id1_again = base(&[1u8; 16]);
        assert_eq!(id1, id1_again, "the same inputs must derive the same id");
    }

    #[test]
    fn message_id_changes_with_any_field() {
        let nonce = [9u8; 16];
        let base = derive_message_id("did:key:zA", "conv:xyz", 1_000, "text/plain", b"hi", &nonce);
        let diff_author =
            derive_message_id("did:key:zB", "conv:xyz", 1_000, "text/plain", b"hi", &nonce);
        let diff_conv =
            derive_message_id("did:key:zA", "conv:abc", 1_000, "text/plain", b"hi", &nonce);
        let diff_ts =
            derive_message_id("did:key:zA", "conv:xyz", 1_001, "text/plain", b"hi", &nonce);
        let diff_ct =
            derive_message_id("did:key:zA", "conv:xyz", 1_000, "text/html", b"hi", &nonce);
        let diff_body =
            derive_message_id("did:key:zA", "conv:xyz", 1_000, "text/plain", b"ho", &nonce);
        for other in [diff_author, diff_conv, diff_ts, diff_ct, diff_body] {
            assert_ne!(base, other);
        }
    }
}
