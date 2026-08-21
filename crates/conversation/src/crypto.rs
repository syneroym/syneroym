//! The key-agreement seam (`D-B4-1`, `D-B4-7`, ADR-0013 §6): the DAG,
//! ordering, sync, and storage above this module never depend on which
//! [`SessionCrypto`] implementation is wired in.
//!
//! **B4b: real X3DH + Double Ratchet, via `vodozemac`** (Apache-2.0,
//! matrix-org's Rust reimplementation of libolm -- license and API
//! verified against the crate's own source/docs before adoption, not from
//! memory). Replaces B4a's `StaticEcdhSessionCrypto` placeholder outright;
//! `ConversationConfig::allow_insecure_crypto` is gone with it.
//!
//! Two keys per service (`D-B4-8`): a fresh vodozemac [`Account`] (its own
//! Curve25519 identity + one-time/fallback keys, for the ratchet) and a
//! separate, independently generated ed25519 [`SigningKey`] (for
//! [`crate::envelope`]'s application-level attribution, which must survive
//! a B5 relay and therefore cannot be tied to any one session -- `D-B4-22`).
//! Neither is derived from the other, and neither is derived from the
//! node's own identity: "no cross-protocol key reuse" is the same reason
//! Signal keeps its identity key separate from its signed prekey.

use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rusqlite::Transaction;
use vodozemac::{
    Curve25519PublicKey,
    olm::{
        Account, AccountPickle, OlmMessage, Session as OlmSession, SessionConfig, SessionPickle,
    },
};

use crate::{
    envelope::DeliveryPayload,
    store::{ConversationStore, SessionRow, now_ms},
};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("crypto error: {0}")]
    Internal(String),
}

/// Self-consistent before any of it is trusted: `self_signature` covers
/// `sig_key || identity_key || one_time_key`, verified under `sig_key`
/// itself. `sig_key` is then pinned trust-on-first-use by the caller
/// (`D-B4-28`) -- this bundle alone proves internal consistency, never who
/// actually holds it. `one_time_key` is either a genuine one-time key
/// (consumed on first use, never reused) or, once the pool is empty,
/// vodozemac's own fallback key -- the same X3DH "signed prekey" role,
/// reused rather than a second signature scheme.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrekeyBundle {
    pub sig_key: [u8; 32],
    pub identity_key: [u8; 32],
    pub one_time_key: [u8; 32],
    #[serde(with = "crate::wire::fixed_bytes")]
    pub self_signature: [u8; 64],
}

/// An established (or about-to-be-established) session with one peer.
/// `local_sig_key` is this service's own application-level signing key,
/// carried here (not re-read from the store) so [`SessionCrypto::encrypt`]
/// can build a self-describing envelope without store access --
/// `encrypt`/`decrypt` are deliberately synchronous (`D-B4-9`: never on the
/// hot path is a store round trip acceptable for the ratchet step itself).
pub struct Session {
    pub peer_address: String,
    pub peer_sig_key: [u8; 32],
    pub local_sig_key: [u8; 32],
    inner: OlmSession,
    /// Set only when this `Session` was just constructed by
    /// [`SessionCrypto::session_for_envelope`] from a pre-key message.
    /// `Account::create_inbound_session` decrypts the first message as
    /// part of establishing the session (vodozemac's own API shape, not a
    /// choice this crate made) -- `decrypt` consumes this instead of
    /// re-decrypting a message the ratchet has already advanced past.
    pending_first_plaintext: Option<Vec<u8>>,
}

// Deliberately does not derive `Debug` on `inner`: a ratchet's live chain
// keys have no business in a log line.
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("peer_address", &self.peer_address).finish_non_exhaustive()
    }
}

/// The wire envelope one `deliver` call carries. `sender_sig_key` travels
/// unconditionally (not only on first contact) so `session_for_envelope`
/// can check it against an already-pinned key on every message, not just
/// the establishing one -- cheap, and it closes the gap a first-message-only
/// header would leave for a later, differently-signed message on an
/// already-established session. `message` is vodozemac's own `OlmMessage`
/// (`PreKey` on first contact, `Normal` after), serialized as-is.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    pub peer_address: String,
    pub sender_sig_key: [u8; 32],
    pub message: OlmMessage,
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
    fn encrypt(&self, s: &mut Session, p: &DeliveryPayload) -> Result<Envelope, CryptoError>;
    fn decrypt(&self, s: &mut Session, e: &Envelope) -> Result<DeliveryPayload, CryptoError>;
    async fn commit(&self, store: &ConversationStore, s: &Session) -> Result<(), CryptoError>;
    fn commit_in(&self, tx: &Transaction<'_>, s: &Session) -> Result<(), CryptoError>;
}

/// Generates a fresh (vodozemac `Account` pickle, ed25519 signing key)
/// pair for `ConversationStore::local_identity_or_generate` to persist
/// (`D-B4-8`).
#[must_use]
pub fn generate_identity_bytes() -> (Vec<u8>, Vec<u8>) {
    let account = Account::new();
    #[allow(clippy::expect_used)]
    let account_bytes =
        serde_json::to_vec(&account.pickle()).expect("AccountPickle always serializes");
    let sig = SigningKey::generate(&mut rand_core::OsRng);
    (account_bytes, sig.to_bytes().to_vec())
}

fn load_account(store: &ConversationStore) -> Result<Account, CryptoError> {
    let row = store
        .local_identity_or_generate(generate_identity_bytes)
        .map_err(|e| CryptoError::Internal(format!("local identity unavailable: {e}")))?;
    let pickle: AccountPickle = serde_json::from_slice(&row.account_state)
        .map_err(|e| CryptoError::Internal(format!("corrupt local account: {e}")))?;
    Ok(Account::from_pickle(pickle))
}

fn load_signing_key(store: &ConversationStore) -> Result<SigningKey, CryptoError> {
    let row = store
        .local_identity_or_generate(generate_identity_bytes)
        .map_err(|e| CryptoError::Internal(format!("local identity unavailable: {e}")))?;
    let sig_bytes: [u8; 32] = row
        .sig_secret
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Internal("corrupt local signing secret".to_string()))?;
    Ok(SigningKey::from_bytes(&sig_bytes))
}

/// Persists a mutated `Account` immediately -- called right after anything
/// that consumes or generates key material, so a crash can never
/// re-publish (one-time keys) or re-consume (an inbound session's prekey)
/// something already spent.
fn save_account(store: &ConversationStore, account: &Account) -> Result<(), CryptoError> {
    let bytes = serde_json::to_vec(&account.pickle())
        .map_err(|e| CryptoError::Internal(format!("cannot persist local account: {e}")))?;
    store.save_local_account(&bytes).map_err(|e| CryptoError::Internal(e.to_string()))
}

fn sign_bundle_fields(
    signing_key: &SigningKey,
    identity_key: &[u8; 32],
    one_time_key: &[u8; 32],
    sig_key: &[u8; 32],
) -> [u8; 64] {
    let mut to_sign = sig_key.to_vec();
    to_sign.extend_from_slice(identity_key);
    to_sign.extend_from_slice(one_time_key);
    signing_key.sign(&to_sign).to_bytes()
}

fn session_from_row(row: SessionRow, local_sig_key: [u8; 32]) -> Result<Session, CryptoError> {
    let pickle: SessionPickle = serde_json::from_slice(&row.state)
        .map_err(|e| CryptoError::Internal(format!("corrupt session state: {e}")))?;
    Ok(Session {
        peer_address: row.peer_address,
        peer_sig_key: row.pinned_sig_key,
        local_sig_key,
        inner: OlmSession::from_pickle(pickle),
        pending_first_plaintext: None,
    })
}

#[derive(Debug, Default)]
pub struct X3dhDoubleRatchetCrypto;

#[async_trait::async_trait]
impl SessionCrypto for X3dhDoubleRatchetCrypto {
    async fn prekey_bundle(&self, store: &ConversationStore) -> Result<PrekeyBundle, CryptoError> {
        let mut account = load_account(store)?;
        let signing_key = load_signing_key(store)?;

        // Replenish lazily: generate a fresh batch only when the pool is
        // empty, so a bundle handed out earlier and not yet consumed by
        // the peer keeps being served identically on a repeat request (a
        // retry must not burn a fresh key for a request the peer never
        // actually used, and must not change what a peer already fetched
        // once).
        let mut mutated = false;
        if account.one_time_keys().is_empty() {
            account.generate_one_time_keys(1);
            mutated = true;
        }
        if account.fallback_key().is_empty() {
            account.generate_fallback_key();
            mutated = true;
        }
        if mutated {
            save_account(store, &account)?;
        }

        let one_time_key = account
            .one_time_keys()
            .values()
            .next()
            .copied()
            .or_else(|| account.fallback_key().values().next().copied())
            .ok_or_else(|| CryptoError::Internal("no prekey available to publish".to_string()))?
            .to_bytes();

        let identity_key = account.identity_keys().curve25519.to_bytes();
        let sig_key = signing_key.verifying_key().to_bytes();
        let self_signature =
            sign_bundle_fields(&signing_key, &identity_key, &one_time_key, &sig_key);
        Ok(PrekeyBundle { sig_key, identity_key, one_time_key, self_signature })
    }

    async fn begin_session(
        &self,
        store: &ConversationStore,
        peer_address: &str,
        bundle: &PrekeyBundle,
    ) -> Result<Session, CryptoError> {
        let mut to_verify = bundle.sig_key.to_vec();
        to_verify.extend_from_slice(&bundle.identity_key);
        to_verify.extend_from_slice(&bundle.one_time_key);
        let vk = VerifyingKey::from_bytes(&bundle.sig_key)
            .map_err(|e| CryptoError::Internal(format!("bad bundle signing key: {e}")))?;
        let sig = Signature::from_bytes(&bundle.self_signature);
        vk.verify(&to_verify, &sig).map_err(|_| {
            CryptoError::PermissionDenied("prekey bundle self-signature invalid".to_string())
        })?;

        let account = load_account(store)?;
        let signing_key = load_signing_key(store)?;
        let identity_key = Curve25519PublicKey::from_bytes(bundle.identity_key);
        let one_time_key = Curve25519PublicKey::from_bytes(bundle.one_time_key);
        let inner = account
            .create_outbound_session(SessionConfig::default(), identity_key, one_time_key)
            .map_err(|e| {
                CryptoError::Internal(format!("cannot establish outbound session: {e}"))
            })?;

        Ok(Session {
            peer_address: peer_address.to_string(),
            peer_sig_key: bundle.sig_key,
            local_sig_key: signing_key.verifying_key().to_bytes(),
            inner,
            pending_first_plaintext: None,
        })
    }

    async fn session_for_envelope(
        &self,
        store: &ConversationStore,
        env: &Envelope,
    ) -> Result<Session, CryptoError> {
        let signing_key = load_signing_key(store)?;
        let local_sig_key = signing_key.verifying_key().to_bytes();

        if let Some(row) =
            store.session(&env.peer_address).map_err(|e| CryptoError::Internal(e.to_string()))?
        {
            // `D-B4-28`: a later message presenting a different signing
            // key for a pinned address is a hard failure, never a silent
            // re-pin.
            if row.pinned_sig_key != env.sender_sig_key {
                return Err(CryptoError::PermissionDenied(
                    "peer presented a different signing key than the one pinned for this address"
                        .to_string(),
                ));
            }
            return session_from_row(row, local_sig_key);
        }

        // No existing session: only a pre-key message can establish one.
        let OlmMessage::PreKey(pre_key) = &env.message else {
            return Err(CryptoError::PermissionDenied(
                "no session exists for this address and the envelope is not a pre-key message"
                    .to_string(),
            ));
        };
        let mut account = load_account(store)?;
        let result = account
            .create_inbound_session(SessionConfig::default(), pre_key.identity_key(), pre_key)
            .map_err(|e| {
                CryptoError::PermissionDenied(format!("cannot establish inbound session: {e}"))
            })?;
        // Durable before anything else touches it: the one-time key
        // `create_inbound_session` just consumed must not be reusable
        // after a restart.
        save_account(store, &account)?;

        Ok(Session {
            peer_address: env.peer_address.clone(),
            peer_sig_key: env.sender_sig_key,
            local_sig_key,
            inner: result.session,
            pending_first_plaintext: Some(result.plaintext),
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
        let signing_key = load_signing_key(store)?;
        session_from_row(row, signing_key.verifying_key().to_bytes()).map(Some)
    }

    fn encrypt(&self, s: &mut Session, p: &DeliveryPayload) -> Result<Envelope, CryptoError> {
        let plaintext = serde_json::to_vec(p)
            .map_err(|e| CryptoError::Internal(format!("cannot encode payload: {e}")))?;
        let message = s
            .inner
            .encrypt(plaintext)
            .map_err(|e| CryptoError::Internal(format!("encryption failed: {e}")))?;
        Ok(Envelope {
            peer_address: s.peer_address.clone(),
            sender_sig_key: s.local_sig_key,
            message,
        })
    }

    fn decrypt(&self, s: &mut Session, e: &Envelope) -> Result<DeliveryPayload, CryptoError> {
        let plaintext = if let Some(pt) = s.pending_first_plaintext.take() {
            pt
        } else {
            s.inner.decrypt(&e.message).map_err(|_| {
                CryptoError::PermissionDenied("envelope failed to decrypt".to_string())
            })?
        };
        serde_json::from_slice(&plaintext)
            .map_err(|e| CryptoError::Internal(format!("undecodable payload: {e}")))
    }

    async fn commit(&self, store: &ConversationStore, s: &Session) -> Result<(), CryptoError> {
        let state = serde_json::to_vec(&s.inner.pickle())
            .map_err(|e| CryptoError::Internal(format!("cannot persist session: {e}")))?;
        let row = SessionRow {
            peer_address: s.peer_address.clone(),
            pinned_sig_key: s.peer_sig_key,
            state,
        };
        store.upsert_session(&row, now_ms()).map_err(|e| CryptoError::Internal(e.to_string()))
    }

    fn commit_in(&self, tx: &Transaction<'_>, s: &Session) -> Result<(), CryptoError> {
        let state = serde_json::to_vec(&s.inner.pickle())
            .map_err(|e| CryptoError::Internal(format!("cannot persist session: {e}")))?;
        let row = SessionRow {
            peer_address: s.peer_address.clone(),
            pinned_sig_key: s.peer_sig_key,
            state,
        };
        ConversationStore::upsert_session_conn(tx, &row, now_ms())
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
        let crypto = X3dhDoubleRatchetCrypto;
        let bundle = crypto.prekey_bundle(&store_a).await.unwrap();

        let store_b = store();
        assert!(crypto.begin_session(&store_b, "did:key:zA", &bundle).await.is_ok());
    }

    #[tokio::test]
    async fn a_tampered_bundle_signature_is_refused() {
        let store_a = store();
        let crypto = X3dhDoubleRatchetCrypto;
        let mut bundle = crypto.prekey_bundle(&store_a).await.unwrap();
        bundle.identity_key[0] ^= 0xFF;

        let store_b = store();
        assert!(crypto.begin_session(&store_b, "did:key:zA", &bundle).await.is_err());
    }

    #[tokio::test]
    async fn both_sides_establish_a_session_and_round_trip_a_payload() {
        let crypto = X3dhDoubleRatchetCrypto;
        let store_a = store();
        let store_b = store();

        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a = crypto.begin_session(&store_a, "b-address", &bundle_b).await.unwrap();

        let env = crypto.encrypt(&mut session_a, &payload()).unwrap();
        assert_eq!(env.peer_address, "b-address");

        // B receives the envelope and establishes its own inbound session
        // from the pre-key message.
        let mut session_b = crypto.session_for_envelope(&store_b, &env).await.unwrap();
        let decrypted = crypto.decrypt(&mut session_b, &env).unwrap();
        assert_eq!(decrypted, payload());
    }

    #[tokio::test]
    async fn a_second_message_on_an_established_session_round_trips_too() {
        let crypto = X3dhDoubleRatchetCrypto;
        let store_a = store();
        let store_b = store();

        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a = crypto.begin_session(&store_a, "b-address", &bundle_b).await.unwrap();
        let env1 = crypto.encrypt(&mut session_a, &payload()).unwrap();
        crypto.commit(&store_a, &session_a).await.unwrap();

        let mut session_b = crypto.session_for_envelope(&store_b, &env1).await.unwrap();
        assert_eq!(crypto.decrypt(&mut session_b, &env1).unwrap(), payload());
        crypto.commit(&store_b, &session_b).await.unwrap();

        // A second message, from A's own re-loaded (committed) session --
        // the ratchet must have advanced past the first message, and B's
        // own re-loaded session must still be able to decrypt it.
        let mut session_a_2 = crypto.session_for(&store_a, "b-address").await.unwrap().unwrap();
        let mut second_payload = payload();
        second_payload.message_id = "msg:2".to_string();
        let env2 = crypto.encrypt(&mut session_a_2, &second_payload).unwrap();
        crypto.commit(&store_a, &session_a_2).await.unwrap();

        let mut session_b_2 = crypto.session_for_envelope(&store_b, &env2).await.unwrap();
        assert_eq!(crypto.decrypt(&mut session_b_2, &env2).unwrap(), second_payload);
    }

    /// `D-B4-28`: a peer re-presenting a different signing key for an
    /// already-pinned address must hard-fail, never silently re-pin.
    #[tokio::test]
    async fn a_re_presented_different_signing_key_is_refused_not_re_pinned() {
        let crypto = X3dhDoubleRatchetCrypto;
        let store_a = store();
        let store_b = store();
        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a = crypto.begin_session(&store_a, "a-address", &bundle_b).await.unwrap();
        let env = crypto.encrypt(&mut session_a, &payload()).unwrap();

        // B receives A's first message and pins A's signing key for
        // "a-address".
        let mut session_b = crypto.session_for_envelope(&store_b, &env).await.unwrap();
        crypto.decrypt(&mut session_b, &env).unwrap();
        crypto.commit(&store_b, &session_b).await.unwrap();

        // A different, genuinely independent identity now claims to be
        // "a-address" -- an impersonation attempt, not a legitimate key
        // rotation.
        let store_attacker = store();
        let bundle_b_for_attacker = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_attacker = crypto
            .begin_session(&store_attacker, "a-address", &bundle_b_for_attacker)
            .await
            .unwrap();
        let mut forged_env = crypto.encrypt(&mut session_attacker, &payload()).unwrap();
        forged_env.peer_address = "a-address".to_string();

        let result = crypto.session_for_envelope(&store_b, &forged_env).await;
        assert!(result.is_err(), "a changed signing key for a pinned address must be refused");
    }

    /// The dropped-ack case (`D-B4-18`'s ratchet-commit ordering): a
    /// delivery attempt that never commits must not have advanced the
    /// *persisted* ratchet, so the next retry -- reloading the same
    /// committed state -- re-derives the identical message the receiver
    /// can still decrypt.
    #[tokio::test]
    async fn an_uncommitted_encrypt_does_not_advance_the_persisted_ratchet() {
        let crypto = X3dhDoubleRatchetCrypto;
        let store_a = store();
        let store_b = store();
        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();

        // Establish and commit a session (models a conversation already
        // past its first message).
        let mut session_a = crypto.begin_session(&store_a, "b-address", &bundle_b).await.unwrap();
        let env0 = crypto.encrypt(&mut session_a, &payload()).unwrap();
        crypto.commit(&store_a, &session_a).await.unwrap();
        let mut session_b = crypto.session_for_envelope(&store_b, &env0).await.unwrap();
        crypto.decrypt(&mut session_b, &env0).unwrap();
        crypto.commit(&store_b, &session_b).await.unwrap();

        // Attempt 1: reload the committed session, encrypt, but never
        // commit (simulating a dropped ack / crash before commit).
        let mut attempt_1 = crypto.session_for(&store_a, "b-address").await.unwrap().unwrap();
        let mut msg2 = payload();
        msg2.message_id = "msg:2".to_string();
        let env_attempt_1 = crypto.encrypt(&mut attempt_1, &msg2).unwrap();

        // Attempt 2 (the retry): reloads from disk again -- since attempt
        // 1 was never committed, this starts from the *same* chain
        // position and must produce something the receiver's own
        // once-committed session can still decrypt.
        let mut attempt_2 = crypto.session_for(&store_a, "b-address").await.unwrap().unwrap();
        let env_attempt_2 = crypto.encrypt(&mut attempt_2, &msg2).unwrap();
        crypto.commit(&store_a, &attempt_2).await.unwrap();

        let mut session_b_2 = crypto.session_for_envelope(&store_b, &env_attempt_2).await.unwrap();
        assert_eq!(crypto.decrypt(&mut session_b_2, &env_attempt_2).unwrap(), msg2);
        let _ = env_attempt_1; // never delivered -- exactly what "uncommitted" means
    }
}
