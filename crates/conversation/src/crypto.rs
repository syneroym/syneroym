//! The key-agreement seam (`D-B4-1`, `D-B4-7`, ADR-0013 §6): the DAG,
//! ordering, sync, and storage above this module never depend on which
//! [`SessionCrypto`] implementation is wired in.
//!
//! **B4a ships [`StaticEcdhSessionCrypto`] only** -- one static ECDH-P256 +
//! AES-256-GCM session per peer, no forward secrecy, no key rotation, no
//! real X3DH one-time-prekey consumption. It exists so the storage, outbox,
//! transport, and dispatch wiring around it can be built and tested before
//! B4b's real X3DH + Double Ratchet lands and deletes this type outright.
//! [`ConversationConfig::allow_insecure_crypto`]-gated at the service
//! boundary (`lib.rs`), refused unless a caller opts in explicitly -- no
//! config file can set it, only this workspace's own tests do.

use std::fmt;

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use p256::{
    PublicKey, SecretKey,
    ecdh::diffie_hellman,
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
};
use rand::RngCore;
use rusqlite::Transaction;
use sha2::Sha256;

use crate::{envelope::DeliveryPayload, store::ConversationStore};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("crypto error: {0}")]
    Internal(String),
}

/// Self-consistent before any of it is trusted: `self_signature` covers
/// `sig_key || dh_identity`, verified under `sig_key` itself. `sig_key` is
/// then pinned trust-on-first-use by the caller (`D-B4-28`) -- this bundle
/// alone proves internal consistency, never who actually holds it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrekeyBundle {
    pub sig_key: [u8; 32],
    pub dh_identity: Vec<u8>,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub self_signature: [u8; 64],
}

/// An established (or about-to-be-established) session with one peer.
/// `local_sig_key`/`local_dh_identity` are this service's own public keys,
/// carried here (not re-read from the store) so [`SessionCrypto::encrypt`]
/// can build a self-describing envelope header without store access --
/// `encrypt`/`decrypt` are deliberately synchronous (`D-B4-9`: never on the
/// hot path is a store round trip acceptable for the ratchet step itself).
#[derive(Debug, Clone)]
pub struct Session {
    pub peer_address: String,
    pub peer_sig_key: [u8; 32],
    pub local_sig_key: [u8; 32],
    pub local_dh_identity: Vec<u8>,
    /// Opaque, per-implementation secret material -- `StaticEcdhSessionCrypto`
    /// stores the raw derived AES-256 key here; a real ratchet would store
    /// its chain-key state.
    pub state: Vec<u8>,
}

/// The wire envelope one `deliver` call carries. `header` is opaque outside
/// this module -- `StaticEcdhSessionCrypto`'s own `StaticHeader`, sent in
/// the clear so a session can be established (or the peer's claimed
/// identity checked against the pinned one) before decryption is even
/// attempted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    pub peer_address: String,
    pub header: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[async_trait::async_trait]
pub trait SessionCrypto: Send + Sync + fmt::Debug {
    async fn prekey_bundle(&self, store: &ConversationStore) -> Result<PrekeyBundle, CryptoError>;
    async fn begin_session(
        &self,
        store: &ConversationStore,
        peer_address: &str,
        bundle: &PrekeyBundle,
    ) -> Result<Session, CryptoError>;
    async fn session_for_envelope(
        &self,
        store: &ConversationStore,
        env: &Envelope,
    ) -> Result<Session, CryptoError>;
    async fn session_for(
        &self,
        store: &ConversationStore,
        peer_address: &str,
    ) -> Result<Option<Session>, CryptoError>;
    fn encrypt(&self, s: &Session, p: &DeliveryPayload) -> Result<Envelope, CryptoError>;
    fn decrypt(&self, s: &Session, e: &Envelope) -> Result<DeliveryPayload, CryptoError>;
    async fn commit(&self, store: &ConversationStore, s: &Session) -> Result<(), CryptoError>;
    fn commit_in(&self, tx: &Transaction<'_>, s: &Session) -> Result<(), CryptoError>;
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StaticHeader {
    sender_dh_identity: Vec<u8>,
    sender_sig_key: [u8; 32],
}

/// NOT A RATCHET. See the module doc.
#[derive(Debug, Default)]
pub struct StaticEcdhSessionCrypto;

/// Generates a fresh (DH secret, ed25519 signing key) pair, as raw bytes
/// for `ConversationStore::local_identity_or_generate` to persist
/// (`D-B4-8`). Shared between `StaticEcdhSessionCrypto` (session keys) and
/// `lib.rs`'s `send` (the message-signing key) -- both read the same row.
#[must_use]
pub fn generate_identity_bytes() -> (Vec<u8>, Vec<u8>) {
    let dh = SecretKey::random(&mut rand_core::OsRng);
    let sig = SigningKey::generate(&mut rand_core::OsRng);
    (dh.to_bytes().to_vec(), sig.to_bytes().to_vec())
}

impl StaticEcdhSessionCrypto {
    fn local_keys(store: &ConversationStore) -> Result<(SecretKey, SigningKey), CryptoError> {
        let row = store
            .local_identity_or_generate(generate_identity_bytes)
            .map_err(|e| CryptoError::Internal(format!("local identity unavailable: {e}")))?;
        let dh_secret = SecretKey::from_slice(&row.dh_secret)
            .map_err(|e| CryptoError::Internal(format!("corrupt local DH secret: {e}")))?;
        let sig_bytes: [u8; 32] = row
            .sig_secret
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Internal("corrupt local signing secret".to_string()))?;
        let signing_key = SigningKey::from_bytes(&sig_bytes);
        Ok((dh_secret, signing_key))
    }

    fn derive_aes_key(shared: &[u8]) -> [u8; 32] {
        let hk = Hkdf::<Sha256>::new(None, shared);
        let mut out = [0u8; 32];
        #[allow(clippy::expect_used)]
        hk.expand(b"syneroym:conversation:v1:static-ecdh", &mut out)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        out
    }

    fn shared_secret(local: &SecretKey, peer_pub_bytes: &[u8]) -> Result<[u8; 32], CryptoError> {
        let encoded = p256::EncodedPoint::from_bytes(peer_pub_bytes)
            .map_err(|e| CryptoError::Internal(format!("bad peer DH public key: {e}")))?;
        let peer_pub = Option::<PublicKey>::from(PublicKey::from_encoded_point(&encoded))
            .ok_or_else(|| CryptoError::Internal("bad peer DH public key".to_string()))?;
        let shared = diffie_hellman(local.to_nonzero_scalar(), peer_pub.as_affine());
        Ok(Self::derive_aes_key(shared.raw_secret_bytes().as_slice()))
    }
}

#[async_trait::async_trait]
impl SessionCrypto for StaticEcdhSessionCrypto {
    async fn prekey_bundle(&self, store: &ConversationStore) -> Result<PrekeyBundle, CryptoError> {
        let (dh_secret, signing_key) = Self::local_keys(store)?;
        let dh_identity = dh_secret.public_key().to_encoded_point(true).as_bytes().to_vec();
        let sig_key = signing_key.verifying_key().to_bytes();
        let mut to_sign = sig_key.to_vec();
        to_sign.extend_from_slice(&dh_identity);
        let self_signature = signing_key.sign(&to_sign).to_bytes();
        Ok(PrekeyBundle { sig_key, dh_identity, self_signature })
    }

    async fn begin_session(
        &self,
        store: &ConversationStore,
        peer_address: &str,
        bundle: &PrekeyBundle,
    ) -> Result<Session, CryptoError> {
        let mut to_verify = bundle.sig_key.to_vec();
        to_verify.extend_from_slice(&bundle.dh_identity);
        let vk = VerifyingKey::from_bytes(&bundle.sig_key)
            .map_err(|e| CryptoError::Internal(format!("bad bundle signing key: {e}")))?;
        let sig = Signature::from_bytes(&bundle.self_signature);
        vk.verify(&to_verify, &sig).map_err(|_| {
            CryptoError::PermissionDenied("prekey bundle self-signature invalid".to_string())
        })?;

        let (dh_secret, signing_key) = Self::local_keys(store)?;
        let aes_key = Self::shared_secret(&dh_secret, &bundle.dh_identity)?;
        Ok(Session {
            peer_address: peer_address.to_string(),
            peer_sig_key: bundle.sig_key,
            local_sig_key: signing_key.verifying_key().to_bytes(),
            local_dh_identity: dh_secret.public_key().to_encoded_point(true).as_bytes().to_vec(),
            state: aes_key.to_vec(),
        })
    }

    async fn session_for_envelope(
        &self,
        store: &ConversationStore,
        env: &Envelope,
    ) -> Result<Session, CryptoError> {
        let header: StaticHeader = serde_json::from_slice(&env.header)
            .map_err(|e| CryptoError::Internal(format!("unreadable envelope header: {e}")))?;
        let (dh_secret, signing_key) = Self::local_keys(store)?;
        let local_sig_key = signing_key.verifying_key().to_bytes();
        let local_dh_identity = dh_secret.public_key().to_encoded_point(true).as_bytes().to_vec();

        if let Some(existing) =
            store.session(&env.peer_address).map_err(|e| CryptoError::Internal(e.to_string()))?
        {
            // `D-B4-28`: a later bundle presenting a different key for a
            // pinned address is a hard failure, never a silent re-pin.
            if existing.pinned_sig_key != header.sender_sig_key {
                return Err(CryptoError::PermissionDenied(
                    "peer presented a different signing key than the one pinned for this address"
                        .to_string(),
                ));
            }
            return Ok(Session {
                peer_address: env.peer_address.clone(),
                peer_sig_key: existing.pinned_sig_key,
                local_sig_key,
                local_dh_identity,
                state: existing.state,
            });
        }

        let aes_key = Self::shared_secret(&dh_secret, &header.sender_dh_identity)?;
        Ok(Session {
            peer_address: env.peer_address.clone(),
            peer_sig_key: header.sender_sig_key,
            local_sig_key,
            local_dh_identity,
            state: aes_key.to_vec(),
        })
    }

    async fn session_for(
        &self,
        store: &ConversationStore,
        peer_address: &str,
    ) -> Result<Option<Session>, CryptoError> {
        let Some(row) =
            store.session(peer_address).map_err(|e| CryptoError::Internal(e.to_string()))?
        else {
            return Ok(None);
        };
        let (dh_secret, signing_key) = Self::local_keys(store)?;
        Ok(Some(Session {
            peer_address: row.peer_address,
            peer_sig_key: row.pinned_sig_key,
            local_sig_key: signing_key.verifying_key().to_bytes(),
            local_dh_identity: dh_secret.public_key().to_encoded_point(true).as_bytes().to_vec(),
            state: row.state,
        }))
    }

    // `try_into`, not `into`: `s.state` is a `Vec<u8>` read back from the
    // store, so its length is not statically known here even though this
    // implementation always writes exactly 32 bytes -- keeping the
    // fallible path turns corrupted state into a graceful `Internal` error
    // instead of a panic.
    #[allow(clippy::unnecessary_fallible_conversions)]
    fn encrypt(&self, s: &Session, p: &DeliveryPayload) -> Result<Envelope, CryptoError> {
        let key: &Key<Aes256Gcm> = s
            .state
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Internal("corrupt session key".to_string()))?;
        let cipher = Aes256Gcm::new(key);
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = serde_json::to_vec(p)
            .map_err(|e| CryptoError::Internal(format!("cannot encode payload: {e}")))?;
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| CryptoError::Internal(format!("encryption failed: {e}")))?;
        let header = StaticHeader {
            sender_dh_identity: s.local_dh_identity.clone(),
            sender_sig_key: s.local_sig_key,
        };
        let header_bytes = serde_json::to_vec(&header)
            .map_err(|e| CryptoError::Internal(format!("cannot encode header: {e}")))?;
        Ok(Envelope {
            peer_address: s.peer_address.clone(),
            header: header_bytes,
            nonce: nonce_bytes.to_vec(),
            ciphertext,
        })
    }

    #[allow(clippy::unnecessary_fallible_conversions)]
    fn decrypt(&self, s: &Session, e: &Envelope) -> Result<DeliveryPayload, CryptoError> {
        let key: &Key<Aes256Gcm> = s
            .state
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Internal("corrupt session key".to_string()))?;
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&e.nonce);
        let plaintext = cipher
            .decrypt(nonce, e.ciphertext.as_ref())
            .map_err(|_| CryptoError::PermissionDenied("envelope failed to decrypt".to_string()))?;
        serde_json::from_slice(&plaintext)
            .map_err(|e| CryptoError::Internal(format!("undecodable payload: {e}")))
    }

    async fn commit(&self, store: &ConversationStore, s: &Session) -> Result<(), CryptoError> {
        let row = crate::store::SessionRow {
            peer_address: s.peer_address.clone(),
            pinned_sig_key: s.peer_sig_key,
            state: s.state.clone(),
        };
        store
            .upsert_session(&row, crate::store::now_ms())
            .map_err(|e| CryptoError::Internal(e.to_string()))
    }

    fn commit_in(&self, tx: &Transaction<'_>, s: &Session) -> Result<(), CryptoError> {
        let row = crate::store::SessionRow {
            peer_address: s.peer_address.clone(),
            pinned_sig_key: s.peer_sig_key,
            state: s.state.clone(),
        };
        crate::store::ConversationStore::upsert_session_conn(tx, &row, crate::store::now_ms())
            .map_err(|e| CryptoError::Internal(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use syneroym_async_queue::QueueConfig;
    use syneroym_core::config::RetryPolicy;

    use super::*;
    use crate::store::ConversationConfig;

    fn store() -> ConversationStore {
        let dir = tempfile::tempdir().unwrap();
        let path = Box::leak(Box::new(dir)).path();
        ConversationStore::open_encrypted(
            path,
            None,
            QueueConfig {
                retry: RetryPolicy {
                    max_attempts: 5,
                    initial_backoff_ms: 10,
                    backoff_multiplier: 2.0,
                    max_backoff_ms: 1000,
                },
                visibility_timeout_ms: 5000,
                dlq_max_rows: 100,
                max_pending_rows: 1000,
            },
            ConversationConfig::default(),
        )
        .unwrap()
    }

    fn payload() -> DeliveryPayload {
        DeliveryPayload {
            message_id: "msg:1".to_string(),
            conversation_id: "conv:1".to_string(),
            author: "did:key:zA".to_string(),
            sender_timestamp_ms: 1_000,
            content_type: "text/plain".to_string(),
            body: b"hello".to_vec(),
            signature: [0u8; 64],
        }
    }

    #[tokio::test]
    async fn a_prekey_bundle_self_signature_verifies() {
        let store_a = store();
        let crypto = StaticEcdhSessionCrypto;
        let bundle = crypto.prekey_bundle(&store_a).await.unwrap();

        let store_b = store();
        // begin_session on the *other* side must accept a genuinely
        // self-consistent bundle.
        assert!(crypto.begin_session(&store_b, "did:key:zA", &bundle).await.is_ok());
    }

    #[tokio::test]
    async fn a_tampered_bundle_signature_is_refused() {
        let store_a = store();
        let crypto = StaticEcdhSessionCrypto;
        let mut bundle = crypto.prekey_bundle(&store_a).await.unwrap();
        bundle.dh_identity[0] ^= 0xFF;

        let store_b = store();
        assert!(crypto.begin_session(&store_b, "did:key:zA", &bundle).await.is_err());
    }

    #[tokio::test]
    async fn both_sides_derive_the_same_session_and_round_trip_a_payload() {
        let crypto = StaticEcdhSessionCrypto;
        let store_a = store();
        let store_b = store();

        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let session_a = crypto.begin_session(&store_a, "b-address", &bundle_b).await.unwrap();

        let env = crypto.encrypt(&session_a, &payload()).unwrap();
        assert_eq!(env.peer_address, "b-address");

        // B receives the envelope and derives its own session from the
        // header -- must land on the same AES key A used.
        let session_b = crypto.session_for_envelope(&store_b, &env).await.unwrap();
        let decrypted = crypto.decrypt(&session_b, &env).unwrap();
        assert_eq!(decrypted, payload());
    }

    /// `D-B4-28`: a peer re-presenting a different signing key for an
    /// already-pinned address must hard-fail, never silently re-pin.
    #[tokio::test]
    async fn a_re_presented_different_signing_key_is_refused_not_re_pinned() {
        let crypto = StaticEcdhSessionCrypto;
        let store_a = store();
        let store_b = store();
        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let session_a = crypto.begin_session(&store_a, "a-address", &bundle_b).await.unwrap();
        let env = crypto.encrypt(&session_a, &payload()).unwrap();

        // B receives A's first message and pins A's signing key for
        // "a-address".
        let session_b = crypto.session_for_envelope(&store_b, &env).await.unwrap();
        crypto.commit(&store_b, &session_b).await.unwrap();

        // A second, genuinely different identity (its own local keys, not
        // A's) now claims to be "a-address" -- an impersonation attempt,
        // not a legitimate key rotation.
        let store_attacker = store();
        let bundle_b_for_attacker = crypto.prekey_bundle(&store_b).await.unwrap();
        let session_attacker = crypto
            .begin_session(&store_attacker, "a-address", &bundle_b_for_attacker)
            .await
            .unwrap();
        let mut forged_env = crypto.encrypt(&session_attacker, &payload()).unwrap();
        forged_env.peer_address = "a-address".to_string();

        let result = crypto.session_for_envelope(&store_b, &forged_env).await;
        assert!(result.is_err(), "a changed signing key for a pinned address must be refused");
    }

    /// The dropped-ack case (`D-B4-18`'s ratchet-commit ordering note): a
    /// delivery attempt that never commits must still be able to encrypt
    /// the *same* message again next attempt and have the receiver decrypt
    /// it -- for this static, non-evolving key, that holds trivially, but
    /// it is asserted so a future ratchet-backed implementation is held to
    /// the same observable behavior.
    #[tokio::test]
    async fn a_session_never_committed_can_still_encrypt_a_retry_the_receiver_can_decrypt() {
        let crypto = StaticEcdhSessionCrypto;
        let store_a = store();
        let store_b = store();
        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let session_a = crypto.begin_session(&store_a, "b-address", &bundle_b).await.unwrap();

        // First attempt: encrypt, but never commit (simulating a dropped
        // ack / crash before commit).
        let env1 = crypto.encrypt(&session_a, &payload()).unwrap();
        // Second attempt, from a session re-derived exactly as `deliver`
        // would (no prior commit means `session_for` finds nothing, so the
        // caller would begin_session again in real code -- here we just
        // reuse `session_a` to model "the same session state").
        let env2 = crypto.encrypt(&session_a, &payload()).unwrap();

        let session_b = crypto.session_for_envelope(&store_b, &env1).await.unwrap();
        assert_eq!(crypto.decrypt(&session_b, &env1).unwrap(), payload());
        assert_eq!(crypto.decrypt(&session_b, &env2).unwrap(), payload());
    }
}
