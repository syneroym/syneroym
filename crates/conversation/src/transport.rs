//! The peer-facing verbs (`prekey-bundle`, `deliver`) and the outbound
//! call they travel over.

use std::time::Duration;

use syneroym_rpc::{
    CallOrigin, CallerContext, ConversationError, ProxyError, ProxyProtocol, ProxyRequest,
};

use crate::{
    ConversationService,
    crypto::{Envelope, PrekeyBundle},
    envelope::DeliveryPayload,
    ids::derive_conversation_id,
    store::{StoredMessage, now_ms},
};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DeliveryAck {
    pub message_id: String,
}

/// What one delivery attempt's failure actually means for the outbox item.
/// Mirrors `syneroym_router::proxy_outbox::Disposition`; not re-derived
/// independently, adapted to this crate's own variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// The peer is not reachable right now — re-defer without charging
    /// the attempt budget. Unreachable-peer retries back off on
    /// `claim_count`, not `attempts`, so the exponential curve applies
    /// without touching the poison-pill budget.
    Unreachable,
    /// A settled refusal: no certificate, malformed envelope, permission
    /// denied. Never retried.
    Terminal(String),
    /// A real transport failure — the attempt budget applies.
    Retry,
    /// The receiver committed the message but its result was not retained
    /// (error-channel delivery confirmation). Marks the item delivered
    /// rather than failed — a replay would produce the same answer, so
    /// the row can safely be completed.
    Delivered,
}

impl ConversationService {
    /// Builds the delivery worker's outbound `CallerContext` with
    /// `proof: None` and `CallOrigin::Native`, and refuses up front if
    /// this service holds no unexpired instance certificate or recorded
    /// owner. Both are required for `invoke_remote_at`'s
    /// instance-certificate branch to present this *service's* identity
    /// to the peer rather than silently falling back to the node's own
    /// key.
    fn check_outbound_identity(&self, svc: &str) -> Result<(), Disposition> {
        let Some(cert) = self.registry.instance_cert(svc) else {
            return Err(Disposition::Terminal(
                "no instance certificate for this service".to_string(),
            ));
        };
        if cert.is_expired() {
            return Err(Disposition::Terminal("instance certificate has expired".to_string()));
        }
        if self.registry.owner_of(svc).is_none() {
            return Err(Disposition::Terminal("no recorded owner for this service".to_string()));
        }
        Ok(())
    }

    pub(crate) async fn call_peer(
        &self,
        svc: &str,
        peer_address: &str,
        method: &str,
        params: serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<serde_json::Value, Disposition> {
        self.check_outbound_identity(svc)?;
        let Some(proxy) = self.current_service_proxy().upgrade() else {
            return Err(Disposition::Retry);
        };
        let caller = CallerContext::service_system(svc);
        let request = ProxyRequest {
            target_service: peer_address.to_string(),
            interface: "conversation".to_string(),
            method: method.to_string(),
            params,
            caller,
            origin: CallOrigin::Native { service_id: Some(svc.to_string()) },
            protocol: ProxyProtocol::default(),
            idempotent: idempotency_key.is_some(),
            idempotency_key,
            timeout: Some(Duration::from_secs(30)),
        };
        proxy.invoke(request).await.map_err(classify)
    }

    /// The sending side of one delivery attempt. Never called on the hot
    /// dispatch path — only from the outbox worker.
    pub(crate) async fn deliver_one(
        &self,
        svc: &str,
        peer_address: &str,
        msg: &StoredMessage,
    ) -> Result<(), Disposition> {
        let store = self.store_for(svc).await.map_err(|_| Disposition::Retry)?;

        let existing_session = self
            .crypto
            .session_for(&store, svc, peer_address)
            .await
            .map_err(|_| Disposition::Retry)?;
        let mut session = match existing_session {
            Some(session) => session,
            None => {
                let bundle_json = match self
                    .call_peer(svc, peer_address, "prekey-bundle", serde_json::json!({}), None)
                    .await
                {
                    Ok(json) => json,
                    Err(Disposition::Delivered) => return Err(Disposition::Retry),
                    Err(other) => return Err(other),
                };
                let bundle: PrekeyBundle = serde_json::from_value(bundle_json).map_err(|_| {
                    Disposition::Terminal("peer returned an undecodable prekey bundle".to_string())
                })?;
                self.crypto.begin_session(&store, svc, peer_address, &bundle).await.map_err(
                    |_| {
                        Disposition::Terminal(
                            "could not establish a session from the peer's bundle".to_string(),
                        )
                    },
                )?
            }
        };

        let payload = DeliveryPayload {
            message_id: msg.id.clone(),
            conversation_id: msg.conversation_id.clone(),
            author: msg.author.clone(),
            sender_timestamp_ms: msg.sender_timestamp_ms,
            content_type: msg.content_type.clone(),
            body: msg.body.clone(),
            signature: msg.signature,
        };
        let env = self
            .crypto
            .encrypt(&mut session, &payload)
            .map_err(|_| Disposition::Terminal("could not encrypt outbound payload".to_string()))?;
        let env_json = serde_json::to_value(&env)
            .map_err(|_| Disposition::Terminal("could not serialize envelope".to_string()))?;

        let ack_json =
            self.call_peer(svc, peer_address, "deliver", env_json, Some(msg.id.clone())).await?;
        let _ack: DeliveryAck = serde_json::from_value(ack_json).map_err(|_| {
            Disposition::Terminal("peer returned an undecodable delivery ack".to_string())
        })?;

        // Ratchet-commit ordering: only after a real `Ok` from the peer,
        // so a failed call leaves the sender able to retry under the same
        // key rather than a step ahead of a receiver that never saw it.
        self.crypto.commit(&store, &session).await.map_err(|_| Disposition::Retry)?;
        Ok(())
    }

    /// The receiving side — reached only from `dispatch_conversation`,
    /// never from a guest.
    pub(crate) async fn peer_deliver_impl(
        &self,
        svc: &str,
        requester_did: &str,
        env: Envelope,
    ) -> Result<DeliveryAck, ConversationError> {
        if requester_did.is_empty() {
            return Err(ConversationError::PermissionDenied);
        }
        let store =
            self.store_for(svc).await.map_err(|e| ConversationError::Internal(e.to_string()))?;

        // Pre-ratchet quota check: verify message limits BEFORE calling
        // `session_for_envelope`. On first contact, `session_for_envelope`
        // consumes and durably spends a one-time key in `local_identity`.
        // If a quota check fails afterward, the session row is rolled back
        // but the one-time key remains spent — causing future retries to
        // fail with MissingOneTimeKey.
        let conv_id = derive_conversation_id(svc, &env.sender_address);
        let max_messages = store.config().max_messages_per_conversation;
        let count = store
            .message_count(&conv_id)
            .map_err(|e| ConversationError::Internal(e.to_string()))?;
        if count >= max_messages {
            return Err(ConversationError::QuotaExceeded);
        }

        let mut session = self
            .crypto
            .session_for_envelope(&store, &env)
            .await
            .map_err(|_| ConversationError::PermissionDenied)?;
        let payload = self
            .crypto
            .decrypt(&mut session, &env)
            .map_err(|_| ConversationError::PermissionDenied)?;

        let author = payload.author.clone();

        // The same-service exemption in the capability gate lets a guest
        // reach this arm on its own service id, but it cannot sign as a
        // peer — a service cannot deliver a message to itself.
        if author == svc {
            return Err(ConversationError::PermissionDenied);
        }

        // The decrypted payload's claimed author must match the peer whose
        // key is pinned in this session. Without this check, a peer C can
        // send a validly-signed envelope claiming author = A and the
        // signature check passes (under C's own key, which is pinned for
        // C's slot) — giving C the ability to forge messages attributed
        // to A.
        if author != session.peer_address {
            return Err(ConversationError::PermissionDenied);
        }

        if !crate::envelope::verify(
            &ed25519_dalek::VerifyingKey::from_bytes(&session.peer_sig_key)
                .map_err(|_| ConversationError::PermissionDenied)?,
            &payload,
        ) {
            return Err(ConversationError::PermissionDenied);
        }
        let now = now_ms();
        let max_skew_ms = (self.max_clock_skew_secs as i64).saturating_mul(1000);
        if payload.sender_timestamp_ms > now.saturating_add(max_skew_ms) {
            return Err(ConversationError::InvalidArgument(
                "sender timestamp implausibly far in the future".to_string(),
            ));
        }
        let expected_conv_id = derive_conversation_id(svc, &author);
        if payload.conversation_id != expected_conv_id {
            return Err(ConversationError::InvalidArgument(
                "conversation id does not match the verified author".to_string(),
            ));
        }

        // Apply per-conversation bounds on the receive path. The same
        // limits `send` enforces for outgoing messages must hold for
        // incoming ones — an unchecked peer can otherwise write unbounded
        // rows and bytes into this service's store.
        let max_body = store.config().max_body_bytes;
        if payload.body.len() as u32 > max_body {
            return Err(ConversationError::QuotaExceeded);
        }

        store
            .queue()
            .transaction(|tx, _txq| {
                store.insert_incoming_if_absent(
                    tx,
                    &payload.conversation_id,
                    &payload.message_id,
                    &author,
                    payload.sender_timestamp_ms,
                    &payload.content_type,
                    &payload.body,
                    &payload.signature,
                    now,
                    store.config().max_messages_per_conversation,
                )?;
                self.crypto
                    .commit_in(tx, &session)
                    .map_err(|e| anyhow::anyhow!("session commit failed: {e}"))?;
                Ok(())
            })
            .map_err(|e| {
                if e.downcast_ref::<crate::store::StoreError>().is_some() {
                    ConversationError::QuotaExceeded
                } else {
                    ConversationError::Internal(e.to_string())
                }
            })?;

        if let Ok(Some(stored)) = store.get_message(&payload.message_id) {
            self.notify_message(svc, stored.into_wire()).await;
        }

        Ok(DeliveryAck { message_id: payload.message_id })
    }
}

/// Mirrors `syneroym_router::proxy_outbox::disposition_of`: not
/// re-derived independently, adapted to this crate's own `Disposition`.
fn classify(error: ProxyError) -> Disposition {
    use syneroym_async_queue::{CALL_ALREADY_RUNNING_RPC_CODE, CALL_RESULT_NOT_RETAINED_RPC_CODE};
    use syneroym_rpc::SERVICE_NOT_FOUND_RPC_CODE;
    match error {
        // "I already ran this, but its result was too large to keep." That
        // is a delivery, reported through the error channel. Treating it
        // as a failure would dead-letter an item that landed.
        ProxyError::Callee { code, .. } if code == CALL_RESULT_NOT_RETAINED_RPC_CODE => {
            Disposition::Delivered
        }
        // Transient in-flight collision or the callee service was briefly
        // absent — retry rather than permanently failing.
        ProxyError::Callee { code, .. }
            if code == CALL_ALREADY_RUNNING_RPC_CODE || code == SERVICE_NOT_FOUND_RPC_CODE =>
        {
            Disposition::Retry
        }
        ProxyError::ServiceNotFound(_) | ProxyError::Timeout(_) | ProxyError::Transport(_) => {
            Disposition::Unreachable
        }
        ProxyError::PermissionDenied(_)
        | ProxyError::UnsupportedTarget(_)
        | ProxyError::UnsupportedProtocol(_)
        | ProxyError::Callee { .. } => Disposition::Terminal("callee refused".to_string()),
        ProxyError::Internal(_) => Disposition::Retry,
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use syneroym_async_queue::QueueConfig;
    use syneroym_core::config::RetryPolicy;

    use super::*;
    use crate::{
        crypto::{SessionCrypto, X3dhDoubleRatchetCrypto},
        envelope,
        store::{ConversationConfig, ConversationStore},
    };

    fn test_store() -> ConversationStore {
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

    async fn test_service(dir: &std::path::Path) -> std::sync::Arc<ConversationService> {
        let storage_provider: std::sync::Arc<dyn syneroym_data_db::traits::StorageProvider> =
            std::sync::Arc::new(
                syneroym_data_db::SqliteStorageProvider::new(dir.join("data"), false).unwrap(),
            );
        let key_store = std::sync::Arc::new(syneroym_data_keystore::KeyStore::new());
        let registry = syneroym_core::local_registry::EndpointRegistry::new(std::sync::Arc::new(
            syneroym_core::storage::MockStorage::new(),
        ))
        .await
        .unwrap();
        ConversationService::new(
            storage_provider,
            key_store,
            registry,
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
            crate::ConversationConfig::default(),
        )
        .unwrap()
    }

    /// A peer whose `payload.author` does not match the address the inbound
    /// session is keyed on must be refused — this is what prevents peer C
    /// from attributing a message to peer A.
    #[tokio::test]
    async fn a_mismatched_author_in_the_payload_is_refused() {
        use syneroym_rpc::ConversationHost;

        let dir = tempfile::tempdir().unwrap();
        let service = test_service(dir.path()).await;
        let crypto = X3dhDoubleRatchetCrypto::new();

        let bundle_bytes = service.prekey_bundle("b-address", "a-address").await.unwrap();
        let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();

        let store_a = test_store();
        let mut session_a =
            crypto.begin_session(&store_a, "a-address", "b-address", &bundle).await.unwrap();

        // Forge an envelope: session is from "a-address" but author claims "c-address".
        // Sign with store_a's legitimate signing key so that envelope::verify
        // passes under the pinned key, isolating the author == session.peer_address
        // guard.
        let identity =
            store_a.local_identity_or_generate(crate::crypto::generate_identity_bytes).unwrap();
        let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&sig_bytes);
        let forged_sig = envelope::sign(
            &signing_key,
            "msg:forged",
            &crate::ids::derive_conversation_id("b-address", "c-address"),
            "c-address",
            1_000,
            "text/plain",
            b"hi",
        );
        let forged_payload = DeliveryPayload {
            message_id: "msg:forged".to_string(),
            conversation_id: crate::ids::derive_conversation_id("b-address", "c-address"),
            author: "c-address".to_string(),
            sender_timestamp_ms: 1_000,
            content_type: "text/plain".to_string(),
            body: b"hi".to_vec(),
            signature: forged_sig,
        };
        let env = crypto.encrypt(&mut session_a, &forged_payload).unwrap();
        let env_bytes = serde_json::to_vec(&env).unwrap();

        let result = service.peer_deliver("b-address", "a-address", env_bytes).await;
        assert!(
            matches!(result, Err(ConversationError::PermissionDenied)),
            "mismatched author must be rejected with PermissionDenied, got {result:?}"
        );
    }

    /// A sender that claims `author == svc` (the same-service exemption
    /// path) must be refused.
    #[tokio::test]
    async fn self_injection_via_same_service_is_refused() {
        use syneroym_rpc::ConversationHost;

        let dir = tempfile::tempdir().unwrap();
        let service = test_service(dir.path()).await;
        let crypto = X3dhDoubleRatchetCrypto::new();

        let bundle_bytes = service.prekey_bundle("b-address", "b-address").await.unwrap();
        let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();

        let store_a = test_store();
        let mut session_a =
            crypto.begin_session(&store_a, "b-address", "b-address", &bundle).await.unwrap();

        // Forge: sign a payload where author == the receiver's own address.
        // Sign with store_a's legitimate signing key so envelope::verify
        // passes under the pinned key, isolating the author == svc guard.
        let identity =
            store_a.local_identity_or_generate(crate::crypto::generate_identity_bytes).unwrap();
        let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&sig_bytes);
        let self_sig = envelope::sign(
            &signing_key,
            "msg:self",
            &crate::ids::derive_conversation_id("b-address", "b-address"),
            "b-address", // author == receiver
            1_000,
            "text/plain",
            b"hi",
        );
        let self_payload = DeliveryPayload {
            message_id: "msg:self".to_string(),
            conversation_id: crate::ids::derive_conversation_id("b-address", "b-address"),
            author: "b-address".to_string(),
            sender_timestamp_ms: 1_000,
            content_type: "text/plain".to_string(),
            body: b"hi".to_vec(),
            signature: self_sig,
        };
        let env = crypto.encrypt(&mut session_a, &self_payload).unwrap();
        let env_bytes = serde_json::to_vec(&env).unwrap();

        let result = service.peer_deliver("b-address", "b-address", env_bytes).await;
        assert!(
            matches!(result, Err(ConversationError::PermissionDenied)),
            "self-injection must be rejected with PermissionDenied, got {result:?}"
        );
    }
}
