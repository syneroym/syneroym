//! The signed message payload (`DeliveryPayload`) and its canonical byte
//! encoding. This is what a ratchet session encrypts and what the receiver
//! verifies — attribution is signature-based, never transport-based.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// What one ratchet session carries per message, before encryption and
/// after decryption. Signed as a whole (`sign`/`verify` below) under the
/// sender's own conversation signing key, independently of the session
/// encryption — attribution survives a relay, which a transport-identity
/// check never could.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeliveryPayload {
    pub message_id: String,
    pub conversation_id: String,
    /// The sender's own routing service id.
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub content_type: String,
    pub languages: Vec<String>,
    pub body: Vec<u8>,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub signature: [u8; 64],
}

/// `u64` count, then each tag as a `u64` length and its bytes. Lengths,
/// not separators, so no byte can move between two tags or into the body.
pub(crate) fn put_languages(out: &mut Vec<u8>, languages: &[String]) {
    out.extend_from_slice(&(languages.len() as u64).to_be_bytes());
    for lang in languages {
        out.extend_from_slice(&(lang.len() as u64).to_be_bytes());
        out.extend_from_slice(lang.as_bytes());
    }
}

/// The reverse of `put_languages`, reading from `bytes[*at..]` and moving
/// `at`. Fails on a short buffer, a count over MAX_MESSAGE_LANGUAGES,
/// an offset overflow or a tag that is not UTF-8.
pub(crate) fn take_languages(bytes: &[u8], at: &mut usize) -> Result<Vec<String>, String> {
    let count_end = at.checked_add(8).ok_or_else(|| "offset overflow".to_string())?;
    let count_bytes = bytes
        .get(*at..count_end)
        .ok_or_else(|| "unexpected end of buffer reading language count".to_string())?;
    let count_u64 = u64::from_be_bytes(
        count_bytes.try_into().map_err(|_| "slice conversion failed".to_string())?,
    );
    *at = count_end;

    if count_u64 > syneroym_text_lang::MAX_MESSAGE_LANGUAGES as u64 {
        return Err(format!(
            "language count {} exceeds maximum {}",
            count_u64,
            syneroym_text_lang::MAX_MESSAGE_LANGUAGES
        ));
    }
    let count = usize::try_from(count_u64).map_err(|_| "count overflow".to_string())?;

    let mut languages = Vec::with_capacity(count);
    for _ in 0..count {
        let len_end = at.checked_add(8).ok_or_else(|| "offset overflow".to_string())?;
        let len_bytes = bytes
            .get(*at..len_end)
            .ok_or_else(|| "unexpected end of buffer reading tag length".to_string())?;
        let tag_len_u64 = u64::from_be_bytes(
            len_bytes.try_into().map_err(|_| "slice conversion failed".to_string())?,
        );
        *at = len_end;

        if tag_len_u64 > syneroym_text_lang::MAX_TAG_LEN as u64 {
            return Err("tag length exceeds maximum".to_string());
        }
        let tag_len =
            usize::try_from(tag_len_u64).map_err(|_| "tag length overflow".to_string())?;

        let tag_end = at.checked_add(tag_len).ok_or_else(|| "offset overflow".to_string())?;
        let tag_bytes = bytes
            .get(*at..tag_end)
            .ok_or_else(|| "unexpected end of buffer reading tag".to_string())?;
        *at = tag_end;

        let tag = std::str::from_utf8(tag_bytes)
            .map_err(|_| "invalid UTF-8 in language tag".to_string())?
            .to_string();
        languages.push(tag);
    }
    Ok(languages)
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
    languages: &[String],
    body: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        32 + message_id.len()
            + conversation_id.len()
            + author.len()
            + content_type.len()
            + body.len()
            + 64,
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
    put_languages(&mut out, languages);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// Signs the canonical encoding of `payload`'s fields under `signing_key`,
/// returning the fields' own signature (not stored on `payload` -- the
/// caller assigns it).
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn sign(
    signing_key: &SigningKey,
    message_id: &str,
    conversation_id: &str,
    author: &str,
    sender_timestamp_ms: i64,
    content_type: &str,
    languages: &[String],
    body: &[u8],
) -> [u8; 64] {
    let bytes = canonical_bytes(
        message_id,
        conversation_id,
        author,
        sender_timestamp_ms,
        content_type,
        languages,
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
        &payload.languages,
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
        let signature = sign(
            &signing_key,
            "msg:1",
            "conv:1",
            "did:key:zAuthor",
            1_000,
            "text/plain",
            &[],
            b"hello",
        );
        (
            signing_key,
            DeliveryPayload {
                message_id: "msg:1".to_string(),
                conversation_id: "conv:1".to_string(),
                author: "did:key:zAuthor".to_string(),
                sender_timestamp_ms: 1_000,
                content_type: "text/plain".to_string(),
                languages: vec![],
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
    fn payload_with_languages_verifies_and_detects_tampering() {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let langs = vec!["hi".to_string(), "en".to_string()];
        let signature = sign(
            &signing_key,
            "msg:1",
            "conv:1",
            "did:key:zAuthor",
            1_000,
            "text/plain",
            &langs,
            b"hello",
        );
        let payload = DeliveryPayload {
            message_id: "msg:1".to_string(),
            conversation_id: "conv:1".to_string(),
            author: "did:key:zAuthor".to_string(),
            sender_timestamp_ms: 1_000,
            content_type: "text/plain".to_string(),
            languages: langs.clone(),
            body: b"hello".to_vec(),
            signature,
        };
        let vk = signing_key.verifying_key();
        assert!(verify(&vk, &payload));

        let mut changed_tag = payload.clone();
        changed_tag.languages = vec!["ta".to_string(), "en".to_string()];
        assert!(!verify(&vk, &changed_tag));

        let mut removed_tag = payload.clone();
        removed_tag.languages = vec!["hi".to_string()];
        assert!(!verify(&vk, &removed_tag));

        let mut swapped_order = payload.clone();
        swapped_order.languages = vec!["en".to_string(), "hi".to_string()];
        assert!(!verify(&vk, &swapped_order));
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
        let a = canonical_bytes("m", "c", "a", 0, "xy", &[], b"z");
        let b = canonical_bytes("m", "c", "a", 0, "x", &[], b"yz");
        assert_ne!(a, b, "the length prefix must prevent this exact ambiguity");

        let c = canonical_bytes("m", "c", "a", 0, "x", &["ab".to_string()], b"");
        let d = canonical_bytes("m", "c", "a", 0, "x", &["a".to_string(), "b".to_string()], b"");
        assert_ne!(c, d, "tag boundaries cannot collide");

        let e = canonical_bytes("m", "c", "a", 0, "x", &["a".to_string()], b"b");
        let f = canonical_bytes("m", "c", "a", 0, "x", &["ab".to_string()], b"");
        assert_ne!(e, f, "tag and body cannot collide");
    }

    #[test]
    fn put_and_take_languages_roundtrip_and_safety() {
        let tags = vec!["hi".to_string(), "en".to_string()];
        let mut buf = Vec::new();
        put_languages(&mut buf, &tags);

        let mut at = 0;
        let read = take_languages(&buf, &mut at).expect("roundtrip");
        assert_eq!(read, tags);
        assert_eq!(at, buf.len());

        let short_buf = buf[..buf.len() - 1].to_vec();
        at = 0;
        assert!(take_languages(&short_buf, &mut at).is_err());

        // Count over maximum: 4
        let mut bad_count_buf = Vec::new();
        bad_count_buf.extend_from_slice(&4u64.to_be_bytes());
        at = 0;
        assert!(take_languages(&bad_count_buf, &mut at).is_err());
    }

    #[test]
    fn wrong_verifying_key_fails() {
        let (_, payload) = payload();
        let other_key = SigningKey::generate(&mut rand_core::OsRng);
        assert!(!verify(&other_key.verifying_key(), &payload));
    }
}
