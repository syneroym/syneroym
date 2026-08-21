//! The key-agreement seam (ADR-0013 §6): the DAG, ordering, sync, and
//! storage above this module never depend on which [`SessionCrypto`]
//! implementation is wired in.
//!
//! Real X3DH + Double Ratchet, via `vodozemac` (Apache-2.0, matrix-org's
//! Rust reimplementation of libolm — license and API verified against the
//! crate's own source/docs before adoption, not from memory). Replaces the
//! earlier `StaticEcdhSessionCrypto` placeholder outright;
//! `ConversationConfig::allow_insecure_crypto` is gone with it.
//!
//! Two keys per service: a fresh vodozemac [`Account`] (its own Curve25519
//! identity + one-time/fallback keys, for the ratchet) and a separate,
//! independently generated ed25519 [`SigningKey`] (for
//! [`crate::envelope`]'s application-level attribution, which must survive
//! a relay and therefore cannot be tied to any one session — ADR-0013 §6).
//! Neither is derived from the other, and neither is derived from the
//! node's own identity: "no cross-protocol key reuse" is the same reason
//! Signal keeps its identity key separate from its signed prekey.

use std::{fmt, sync::Arc};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rusqlite::Transaction;
use tokio::sync::Mutex as AsyncMutex;
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
/// itself. `sig_key` is then pinned trust-on-first-use by the caller —
/// this bundle alone proves internal consistency, never who actually holds
/// it. `one_time_key` is either a genuine one-time key (consumed on first
/// use, never reused) or, once the pool is empty, vodozemac's own fallback
/// key — the same X3DH "signed prekey" role, reused rather than a second
/// signature scheme.
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
/// can build a self-describing envelope without store access —
/// `encrypt`/`decrypt` are deliberately synchronous (never on the hot
/// path is a store round trip acceptable for the ratchet step itself).
/// `local_address` is this service's own routing id, embedded in the
/// outbound envelope so the receiver can key its inbound session on the
/// actual sender rather than on a field the sender does not control.
pub struct Session {
    pub peer_address: String,
    pub local_address: String,
    pub peer_sig_key: [u8; 32],
    pub local_sig_key: [u8; 32],
    inner: OlmSession,
    /// Set only when this `Session` was just constructed by
    /// [`SessionCrypto::session_for_envelope`] from a pre-key message.
    /// `Account::create_inbound_session` decrypts the first message as
    /// part of establishing the session (vodozemac's own API shape, not a
    /// choice this crate made) — `decrypt` consumes this instead of
    /// re-decrypting a message the ratchet has already advanced past.
    pending_first_plaintext: Option<Vec<u8>>,
}

// Deliberately does not derive `Debug` on `inner`: a ratchet's live chain
// keys have no business in a log line.
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("peer_address", &self.peer_address)
            .field("local_address", &self.local_address)
            .finish_non_exhaustive()
    }
}

/// The wire envelope one `deliver` call carries. `sender_sig_key` travels
/// unconditionally (not only on first contact) so `session_for_envelope`
/// can check it against an already-pinned key on every message, not just
/// the establishing one — cheap, and it closes the gap a first-message-only
/// header would leave for a later, differently-signed message on an
/// already-established session.
///
/// `sender_address` is the sender's own routing service id. The receiver
/// keys its inbound session table on this field, so every peer gets its
/// own session slot rather than all sharing the receiver's own address.
/// It is distinct from the signed `payload.author` (which also carries the
/// sender's id but is only available after decryption) because session
/// lookup must happen before decryption.
///
/// `message` is vodozemac's own `OlmMessage` (`PreKey` on first contact,
/// `Normal` after), serialized as-is.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    pub peer_address: String,
    pub sender_address: String,
    pub sender_sig_key: [u8; 32],
    pub message: OlmMessage,
}

#[async_trait::async_trait]
pub trait SessionCrypto: Send + Sync + fmt::Debug {
    async fn prekey_bundle(&self, store: &ConversationStore) -> Result<PrekeyBundle, CryptoError>;
    async fn begin_session(
        &self,
        store: &ConversationStore,
        local_address: &str,
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
        local_address: &str,
        peer_address: &str,
    ) -> Result<Option<Session>, CryptoError>;
    fn encrypt(&self, s: &mut Session, p: &DeliveryPayload) -> Result<Envelope, CryptoError>;
    fn decrypt(&self, s: &mut Session, e: &Envelope) -> Result<DeliveryPayload, CryptoError>;
    async fn commit(&self, store: &ConversationStore, s: &Session) -> Result<(), CryptoError>;
    fn commit_in(&self, tx: &Transaction<'_>, s: &Session) -> Result<(), CryptoError>;
}

/// Generates a fresh (vodozemac `Account` pickle, ed25519 signing key)
/// pair for `ConversationStore::local_identity_or_generate` to persist.
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

/// Persists a mutated `Account` immediately — called right after anything
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

fn session_from_row(
    row: SessionRow,
    local_address: String,
    local_sig_key: [u8; 32],
) -> Result<Session, CryptoError> {
    let pickle: SessionPickle = serde_json::from_slice(&row.state)
        .map_err(|e| CryptoError::Internal(format!("corrupt session state: {e}")))?;
    Ok(Session {
        peer_address: row.peer_address,
        local_address,
        peer_sig_key: row.pinned_sig_key,
        local_sig_key,
        inner: OlmSession::from_pickle(pickle),
        pending_first_plaintext: None,
    })
}

/// Serialises read-modify-write cycles on the local vodozemac `Account`.
/// This prevents concurrent `prekey_bundle` and `session_for_envelope`
/// operations from racing on the local account state.
pub struct X3dhDoubleRatchetCrypto {
    crypto_lock: Arc<AsyncMutex<()>>,
}

impl fmt::Debug for X3dhDoubleRatchetCrypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("X3dhDoubleRatchetCrypto").finish_non_exhaustive()
    }
}

impl X3dhDoubleRatchetCrypto {
    #[must_use]
    pub fn new() -> Self {
        Self { crypto_lock: Arc::new(AsyncMutex::new(())) }
    }
}

impl Default for X3dhDoubleRatchetCrypto {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SessionCrypto for X3dhDoubleRatchetCrypto {
    async fn prekey_bundle(&self, store: &ConversationStore) -> Result<PrekeyBundle, CryptoError> {
        let _guard = self.crypto_lock.lock().await;

        let mut account = load_account(store)?;
        let signing_key = load_signing_key(store)?;

        // Replenish one key when the published pool is empty. In vodozemac,
        // `mark_keys_as_published` marks all currently unpublished keys,
        // so generating 1 key per request ensures every generated key is
        // published and served without accumulating unused keys in the
        // account pickle.
        if account.one_time_keys().is_empty() {
            account.generate_one_time_keys(1);
        }
        if account.fallback_key().is_empty() {
            account.generate_fallback_key();
        }

        let one_time_key = account
            .one_time_keys()
            .values()
            .next()
            .copied()
            .or_else(|| account.fallback_key().values().next().copied())
            .ok_or_else(|| CryptoError::Internal("no prekey available to publish".to_string()))?
            .to_bytes();

        // Mark as published so the next caller receives a different key.
        // Always mutated here; always persisted.
        account.mark_keys_as_published();
        save_account(store, &account)?;

        let identity_key = account.identity_keys().curve25519.to_bytes();
        let sig_key = signing_key.verifying_key().to_bytes();
        let self_signature =
            sign_bundle_fields(&signing_key, &identity_key, &one_time_key, &sig_key);
        Ok(PrekeyBundle { sig_key, identity_key, one_time_key, self_signature })
    }

    async fn begin_session(
        &self,
        store: &ConversationStore,
        local_address: &str,
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
            local_address: local_address.to_string(),
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
        let _guard = self.crypto_lock.lock().await;

        let signing_key = load_signing_key(store)?;
        let local_sig_key = signing_key.verifying_key().to_bytes();

        // Inbound sessions are keyed on the sender's own address, carried
        // in the envelope as `sender_address`. This ensures every peer
        // gets its own slot in the session table, independent of the
        // receiver's own address.
        if let Some(row) =
            store.session(&env.sender_address).map_err(|e| CryptoError::Internal(e.to_string()))?
        {
            // A later message presenting a different signing key for a
            // pinned address is a hard failure, never a silent re-pin.
            if row.pinned_sig_key != env.sender_sig_key {
                return Err(CryptoError::PermissionDenied(
                    "peer presented a different signing key than the one pinned for this address"
                        .to_string(),
                ));
            }
            return session_from_row(row, env.peer_address.clone(), local_sig_key);
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
            peer_address: env.sender_address.clone(),
            local_address: env.peer_address.clone(),
            peer_sig_key: env.sender_sig_key,
            local_sig_key,
            inner: result.session,
            pending_first_plaintext: Some(result.plaintext),
        })
    }

    async fn session_for(
        &self,
        store: &ConversationStore,
        local_address: &str,
        peer_address: &str,
    ) -> Result<Option<Session>, CryptoError> {
        let Some(row) =
            store.session(peer_address).map_err(|e| CryptoError::Internal(e.to_string()))?
        else {
            return Ok(None);
        };
        let signing_key = load_signing_key(store)?;
        session_from_row(row, local_address.to_string(), signing_key.verifying_key().to_bytes())
            .map(Some)
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
            sender_address: s.local_address.clone(),
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
        let crypto = X3dhDoubleRatchetCrypto::new();
        let bundle = crypto.prekey_bundle(&store_a).await.unwrap();

        let store_b = store();
        assert!(crypto.begin_session(&store_b, "b-address", "did:key:zA", &bundle).await.is_ok());
    }

    #[tokio::test]
    async fn a_tampered_bundle_signature_is_refused() {
        let store_a = store();
        let crypto = X3dhDoubleRatchetCrypto::new();
        let mut bundle = crypto.prekey_bundle(&store_a).await.unwrap();
        bundle.identity_key[0] ^= 0xFF;

        let store_b = store();
        assert!(crypto.begin_session(&store_b, "b-address", "did:key:zA", &bundle).await.is_err());
    }

    #[tokio::test]
    async fn both_sides_establish_a_session_and_round_trip_a_payload() {
        let crypto = X3dhDoubleRatchetCrypto::new();
        let store_a = store();
        let store_b = store();

        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a =
            crypto.begin_session(&store_a, "a-address", "b-address", &bundle_b).await.unwrap();

        let env = crypto.encrypt(&mut session_a, &payload()).unwrap();
        assert_eq!(env.peer_address, "b-address");
        assert_eq!(env.sender_address, "a-address");

        // B receives the envelope and establishes its own inbound session.
        let mut session_b = crypto.session_for_envelope(&store_b, &env).await.unwrap();
        let decrypted = crypto.decrypt(&mut session_b, &env).unwrap();
        assert_eq!(decrypted, payload());
    }

    #[tokio::test]
    async fn a_second_message_on_an_established_session_round_trips_too() {
        let crypto = X3dhDoubleRatchetCrypto::new();
        let store_a = store();
        let store_b = store();

        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a =
            crypto.begin_session(&store_a, "a-address", "b-address", &bundle_b).await.unwrap();
        let env1 = crypto.encrypt(&mut session_a, &payload()).unwrap();
        crypto.commit(&store_a, &session_a).await.unwrap();

        let mut session_b = crypto.session_for_envelope(&store_b, &env1).await.unwrap();
        assert_eq!(crypto.decrypt(&mut session_b, &env1).unwrap(), payload());
        crypto.commit(&store_b, &session_b).await.unwrap();

        // A second message, from A's own re-loaded (committed) session.
        let mut session_a_2 =
            crypto.session_for(&store_a, "a-address", "b-address").await.unwrap().unwrap();
        let mut second_payload = payload();
        second_payload.message_id = "msg:2".to_string();
        let env2 = crypto.encrypt(&mut session_a_2, &second_payload).unwrap();
        crypto.commit(&store_a, &session_a_2).await.unwrap();

        let mut session_b_2 = crypto.session_for_envelope(&store_b, &env2).await.unwrap();
        assert_eq!(crypto.decrypt(&mut session_b_2, &env2).unwrap(), second_payload);
    }

    /// A peer re-presenting a different signing key for an already-pinned
    /// address must hard-fail, never silently re-pin.
    #[tokio::test]
    async fn a_re_presented_different_signing_key_is_refused_not_re_pinned() {
        let crypto = X3dhDoubleRatchetCrypto::new();
        let store_a = store();
        let store_b = store();
        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a =
            crypto.begin_session(&store_a, "a-address", "b-address", &bundle_b).await.unwrap();
        let env = crypto.encrypt(&mut session_a, &payload()).unwrap();

        // B receives A's first message and pins A's signing key for "a-address".
        let mut session_b = crypto.session_for_envelope(&store_b, &env).await.unwrap();
        crypto.decrypt(&mut session_b, &env).unwrap();
        crypto.commit(&store_b, &session_b).await.unwrap();

        // A different, genuinely independent identity now claims to be
        // "a-address" — an impersonation attempt, not a key rotation.
        let store_attacker = store();
        let bundle_b_for_attacker = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_attacker = crypto
            .begin_session(&store_attacker, "a-address", "b-address", &bundle_b_for_attacker)
            .await
            .unwrap();
        let mut forged_env = crypto.encrypt(&mut session_attacker, &payload()).unwrap();
        forged_env.peer_address = "b-address".to_string();
        // Attacker claims sender_address = "a-address" to hit the pinned slot.
        forged_env.sender_address = "a-address".to_string();

        let result = crypto.session_for_envelope(&store_b, &forged_env).await;
        assert!(result.is_err(), "a changed signing key for a pinned address must be refused");
    }

    /// Two independent senders (A and C) both reaching the same receiver
    /// (B) must each get their own inbound session slot.
    #[tokio::test]
    async fn two_senders_each_get_an_independent_inbound_session_on_the_receiver() {
        let crypto = X3dhDoubleRatchetCrypto::new();
        let store_a = store();
        let store_b = store();
        let store_c = store();

        // A fetches B's bundle and sends.
        let bundle_b_for_a = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_a = crypto
            .begin_session(&store_a, "a-address", "b-address", &bundle_b_for_a)
            .await
            .unwrap();
        let env_from_a = crypto.encrypt(&mut session_a, &payload()).unwrap();

        // C fetches a fresh bundle from B and sends.
        let bundle_b_for_c = crypto.prekey_bundle(&store_b).await.unwrap();
        let mut session_c = crypto
            .begin_session(&store_c, "c-address", "b-address", &bundle_b_for_c)
            .await
            .unwrap();
        let env_from_c = crypto.encrypt(&mut session_c, &payload()).unwrap();

        // B receives both — neither must block the other.
        let mut sess_b_from_a = crypto.session_for_envelope(&store_b, &env_from_a).await.unwrap();
        let payload_a = crypto.decrypt(&mut sess_b_from_a, &env_from_a).unwrap();
        crypto.commit(&store_b, &sess_b_from_a).await.unwrap();

        let mut sess_b_from_c = crypto.session_for_envelope(&store_b, &env_from_c).await.unwrap();
        let payload_c = crypto.decrypt(&mut sess_b_from_c, &env_from_c).unwrap();
        crypto.commit(&store_b, &sess_b_from_c).await.unwrap();

        assert_eq!(payload_a, payload());
        assert_eq!(payload_c, payload());
        // Each must be stored under its own sender's address.
        assert!(store_b.session("a-address").unwrap().is_some());
        assert!(store_b.session("c-address").unwrap().is_some());
    }

    /// The dropped-ack case: a delivery attempt that never commits must
    /// not have advanced the persisted ratchet, so the next retry —
    /// reloading the same committed state — re-derives the identical
    /// message the receiver can still decrypt.
    #[tokio::test]
    async fn an_uncommitted_encrypt_does_not_advance_the_persisted_ratchet() {
        let crypto = X3dhDoubleRatchetCrypto::new();
        let store_a = store();
        let store_b = store();
        let bundle_b = crypto.prekey_bundle(&store_b).await.unwrap();

        // Establish and commit a session (models a conversation already
        // past its first message).
        let mut session_a =
            crypto.begin_session(&store_a, "a-address", "b-address", &bundle_b).await.unwrap();
        let env0 = crypto.encrypt(&mut session_a, &payload()).unwrap();
        crypto.commit(&store_a, &session_a).await.unwrap();
        let mut session_b = crypto.session_for_envelope(&store_b, &env0).await.unwrap();
        crypto.decrypt(&mut session_b, &env0).unwrap();
        crypto.commit(&store_b, &session_b).await.unwrap();

        // Attempt 1: reload the committed session, encrypt, but never
        // commit (simulating a dropped ack / crash before commit).
        let mut attempt_1 =
            crypto.session_for(&store_a, "a-address", "b-address").await.unwrap().unwrap();
        let mut msg2 = payload();
        msg2.message_id = "msg:2".to_string();
        let env_attempt_1 = crypto.encrypt(&mut attempt_1, &msg2).unwrap();

        // Attempt 2 (the retry): reloads from disk again — since attempt
        // 1 was never committed, this starts from the same chain position.
        let mut attempt_2 =
            crypto.session_for(&store_a, "a-address", "b-address").await.unwrap().unwrap();
        let env_attempt_2 = crypto.encrypt(&mut attempt_2, &msg2).unwrap();
        crypto.commit(&store_a, &attempt_2).await.unwrap();

        let mut session_b_2 = crypto.session_for_envelope(&store_b, &env_attempt_2).await.unwrap();
        assert_eq!(crypto.decrypt(&mut session_b_2, &env_attempt_2).unwrap(), msg2);
        let _ = env_attempt_1; // never delivered — exactly what "uncommitted" means
    }

    /// `prekey_bundle` must serve a distinct key to each consecutive caller
    /// — calling `mark_keys_as_published` after each serve is what makes
    /// this hold.
    #[tokio::test]
    async fn consecutive_prekey_bundle_calls_serve_distinct_keys() {
        let crypto = X3dhDoubleRatchetCrypto::new();
        let store_b = store();
        let b1 = crypto.prekey_bundle(&store_b).await.unwrap();
        let b2 = crypto.prekey_bundle(&store_b).await.unwrap();
        assert_ne!(b1.one_time_key, b2.one_time_key, "each caller must get a distinct OTK");
    }
}
