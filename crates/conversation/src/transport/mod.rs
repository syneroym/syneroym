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

pub(super) fn internal(e: impl std::fmt::Display) -> ConversationError {
    ConversationError::Internal(e.to_string())
}

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
        timeout: Option<Duration>,
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
            timeout: Some(timeout.unwrap_or(Duration::from_secs(30))),
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
                    .call_peer(
                        svc,
                        peer_address,
                        "prekey-bundle",
                        serde_json::json!({}),
                        None,
                        None,
                    )
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

        let ack_json = self
            .call_peer(svc, peer_address, "deliver", env_json, Some(msg.id.clone()), None)
            .await?;
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

        let is_group_key = payload.content_type == crate::dag::GROUP_KEY_CONTENT_TYPE;
        let mut group_id_to_apply = None;

        let my_ident = store
            .local_identity_or_generate(crate::crypto::generate_identity_bytes)
            .map_err(internal)?;
        let my_sig_key: [u8; 32] = my_ident.sig_secret.as_slice().try_into().unwrap_or([0u8; 32]);
        let my_vk = ed25519_dalek::SigningKey::from_bytes(&my_sig_key).verifying_key().to_bytes();

        let parsed_group_key: Option<crate::dag::GroupKeyPayload> = if is_group_key {
            let key_payload: crate::dag::GroupKeyPayload = serde_json::from_slice(&payload.body)
                .map_err(|e| {
                    ConversationError::InvalidArgument(format!("invalid group key payload: {e}"))
                })?;
            // Sender must be the owner of the group declared in the payload
            if author != key_payload.owner {
                return Err(ConversationError::PermissionDenied);
            }
            // Recipient (this service) must be in the member roster distributed by the
            // owner
            if !key_payload.members.contains(&svc.to_string()) {
                return Err(ConversationError::PermissionDenied);
            }
            // Epoch must be >= 1
            if key_payload.epoch == 0 {
                return Err(ConversationError::InvalidArgument("invalid epoch".to_string()));
            }
            Some(key_payload)
        } else {
            None
        };

        store
            .queue()
            .transaction(|tx, _txq| {
                if let Some(key_payload) = &parsed_group_key {
                    let shell = crate::store::ConversationStore::get_or_create_group_shell(
                        tx,
                        &key_payload.group_id,
                        &key_payload.owner,
                        key_payload.epoch,
                        now,
                    )?;
                    // If group already existed, owner must match
                    if shell.owner_address.as_deref() != Some(&key_payload.owner) {
                        return Err(anyhow::anyhow!("group owner mismatch"));
                    }
                    // Epoch must not jump backwards or unreasonably ahead of shell's current epoch
                    if key_payload.epoch > shell.current_epoch + 100 {
                        return Err(anyhow::anyhow!("epoch jump too large"));
                    }
                    tx.execute(
                        "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) \
                         VALUES (?1, ?2, ?3, ?4) ON CONFLICT(conversation_id, epoch) DO NOTHING",
                        rusqlite::params![
                            key_payload.group_id,
                            key_payload.epoch as i64,
                            key_payload.key.as_slice(),
                            now,
                        ],
                    )?;
                    // Seed `group_members` so this service can send/verify before the
                    // corresponding membership DAG entries have synced — but with each
                    // row's *real* `joined_epoch`, not a hardcoded 1. The owner is the
                    // one exception: it is always the group's epoch-1 founder by
                    // construction (`create_group_impl`), regardless of which key
                    // message a receiver happens to learn it from. Getting another
                    // member's `joined_epoch` wrong here is exactly what made a
                    // genesis entry's `member_list_hash` disagree at every receiver
                    // that had already seen a later epoch's key message — the seeded
                    // row falsely counted as a member since epoch 1.
                    for m in &key_payload.members {
                        if m != svc && m != &author {
                            tx.execute(
                                "INSERT INTO group_members (conversation_id, member_address, \
                                 sig_key, joined_epoch, removed_epoch) VALUES (?1, ?2, \
                                 zeroblob(32), ?3, NULL) ON CONFLICT(conversation_id, \
                                 member_address) DO NOTHING",
                                rusqlite::params![
                                    key_payload.group_id,
                                    m,
                                    key_payload.epoch as i64
                                ],
                            )?;
                        }
                    }
                    tx.execute(
                        "INSERT INTO group_members (conversation_id, member_address, sig_key, \
                         joined_epoch, removed_epoch) VALUES (?1, ?2, ?3, 1, NULL) ON \
                         CONFLICT(conversation_id, member_address) DO UPDATE SET sig_key = \
                         excluded.sig_key",
                        rusqlite::params![
                            key_payload.group_id,
                            author,
                            session.peer_sig_key.as_slice(),
                        ],
                    )?;
                    tx.execute(
                        "INSERT INTO group_members (conversation_id, member_address, sig_key, \
                         joined_epoch, removed_epoch) VALUES (?1, ?2, ?3, ?4, NULL) ON \
                         CONFLICT(conversation_id, member_address) DO UPDATE SET sig_key = \
                         excluded.sig_key",
                        rusqlite::params![
                            key_payload.group_id,
                            svc,
                            my_vk.as_slice(),
                            key_payload.epoch as i64,
                        ],
                    )?;
                    tx.execute(
                        "UPDATE conversations SET current_epoch = MAX(current_epoch, ?1), \
                         last_activity = ?2 WHERE id = ?3",
                        rusqlite::params![key_payload.epoch as i64, now, key_payload.group_id],
                    )?;
                    group_id_to_apply = Some(key_payload.group_id.clone());
                }

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
                if is_group_key {
                    tx.execute(
                        "UPDATE messages SET system = 1 WHERE id = ?1",
                        rusqlite::params![payload.message_id],
                    )?;
                }
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

        if is_group_key {
            if let Some(gid) = group_id_to_apply {
                self.apply_pending_entries(&store, svc, &gid).await;
            }
        } else if let Ok(Some(stored)) = store.get_message(&payload.message_id) {
            self.notify_message(svc, stored.into_wire()).await;
        }

        Ok(DeliveryAck { message_id: payload.message_id })
    }
}

/// Mirrors `syneroym_router::proxy_outbox::disposition_of`: not
/// re-derived independently, adapted to this crate's own `Disposition`.
pub(super) fn classify(error: ProxyError) -> Disposition {
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

mod group_sync;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
