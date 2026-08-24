//! The peer-facing verbs (`prekey-bundle`, `deliver`) and the outbound
//! call they travel over.

use std::time::Duration;

use rand::RngCore;
use syneroym_rpc::{
    CallOrigin, CallerContext, ConversationError, ProxyError, ProxyProtocol, ProxyRequest,
};

use crate::{
    ConversationService,
    crypto::{Envelope, PrekeyBundle},
    envelope::DeliveryPayload,
    ids::derive_conversation_id,
    store::{ConversationStore, StoredMessage, now_ms},
};

fn internal(e: impl std::fmt::Display) -> ConversationError {
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

    pub(crate) async fn push_group_entry(
        &self,
        store: &ConversationStore,
        svc: &str,
        peer_address: &str,
        entry: &crate::dag::WireEntry,
    ) -> Result<(), Disposition> {
        let ident = store
            .local_identity_or_generate(crate::crypto::generate_identity_bytes)
            .map_err(|_| Disposition::Retry)?;
        let sig_bytes: [u8; 32] = ident
            .sig_secret
            .as_slice()
            .try_into()
            .map_err(|_| Disposition::Terminal("corrupt local signing key".to_string()))?;
        let sk = ed25519_dalek::SigningKey::from_bytes(&sig_bytes);
        let mut nonce = [0u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let assertion =
            crate::dag::sign_peer_assertion(&sk, svc, &entry.conversation_id, now_ms(), &nonce);
        let req = crate::dag::GroupPushRequest {
            from: assertion,
            group: entry.conversation_id.clone(),
            entries: vec![entry.clone()],
        };
        let json = serde_json::to_value(&req)
            .map_err(|_| Disposition::Terminal("serialize error".to_string()))?;
        let ack_json = self
            .call_peer(
                svc,
                peer_address,
                "group-push",
                json,
                Some(format!("{}:{}", entry.entry_id, peer_address)),
                Some(Duration::from_secs(10)),
            )
            .await?;
        let _ack: crate::dag::GroupPushAck = serde_json::from_value(ack_json)
            .map_err(|_| Disposition::Terminal("invalid ack".to_string()))?;
        Ok(())
    }

    pub(crate) async fn deliver_group_one(
        &self,
        svc: &str,
        peer_address: &str,
        msg: &StoredMessage,
    ) -> Result<(), Disposition> {
        let store = self.store_for(svc).await.map_err(|_| Disposition::Retry)?;
        let entry_id = msg
            .entry_id
            .as_deref()
            .ok_or_else(|| Disposition::Terminal("message is missing entry_id".to_string()))?;
        let entry = store
            .wire_entry(entry_id)
            .map_err(|_| Disposition::Retry)?
            .ok_or_else(|| Disposition::Terminal("DAG entry not found".to_string()))?;
        self.push_group_entry(&store, svc, peer_address, &entry).await
    }

    pub(crate) async fn group_push_impl(
        &self,
        svc: &str,
        requester_did: &str,
        req: crate::dag::GroupPushRequest,
    ) -> Result<crate::dag::GroupPushAck, ConversationError> {
        if requester_did.is_empty() {
            return Err(ConversationError::PermissionDenied);
        }
        let store = self.store_for(svc).await.map_err(internal)?;
        let conv = store
            .get_conversation(&req.group)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != syneroym_rpc::ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        if req.from.address == svc {
            return Err(ConversationError::PermissionDenied);
        }

        // Anti-replay and freshness check on `PeerAssertion`, matching `group-sync`
        // below — an unbounded assertion here lets a captured push be replayed
        // without limit.
        let now = now_ms();
        let max_skew_ms = (store.config().max_clock_skew_secs as i64) * 1000;
        if (req.from.timestamp_ms - now).abs() > max_skew_ms {
            return Err(ConversationError::PermissionDenied);
        }

        let sender_sig_key =
            pinned_member_sig_key(&store, &conv.id, &req.from.address, req.from.sig_key)?;
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&sender_sig_key).map_err(internal)?;
        if !crate::dag::verify_peer_assertion(&vk, &req.group, &req.from) {
            return Err(ConversationError::PermissionDenied);
        }

        if req.entries.len() as u32 > store.config().conversation_max_sync_entries_per_call {
            return Err(ConversationError::QuotaExceeded);
        }

        let mut accepted = Vec::new();
        for entry in &req.entries {
            if entry.conversation_id != req.group {
                return Err(ConversationError::InvalidArgument(
                    "entry conversation_id does not match request group".to_string(),
                ));
            }
            let (ins, msg_opt) = crate::group::validate_and_insert(&store, svc, &conv, entry)?;
            if ins {
                accepted.push(entry.entry_id.clone());
            }
            if let Some(msg) = msg_opt {
                self.notify_message(svc, msg.into_wire()).await;
            }
        }

        Ok(crate::dag::GroupPushAck { accepted })
    }

    pub(crate) async fn group_sync_impl(
        &self,
        svc: &str,
        requester_did: &str,
        req: crate::dag::GroupSyncRequest,
    ) -> Result<crate::dag::GroupSyncResponse, ConversationError> {
        if requester_did.is_empty() {
            return Err(ConversationError::PermissionDenied);
        }
        let store = self.store_for(svc).await.map_err(internal)?;
        let conv = store
            .get_conversation(&req.group)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != syneroym_rpc::ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        if req.from.address == svc {
            return Err(ConversationError::PermissionDenied);
        }

        // Anti-replay and freshness check on PeerAssertion
        let now = now_ms();
        let max_skew_ms = (store.config().max_clock_skew_secs as i64) * 1000;
        if (req.from.timestamp_ms - now).abs() > max_skew_ms {
            return Err(ConversationError::PermissionDenied);
        }

        let sender_sig_key =
            pinned_member_sig_key(&store, &conv.id, &req.from.address, req.from.sig_key)?;
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&sender_sig_key).map_err(internal)?;
        if !crate::dag::verify_peer_assertion(&vk, &req.group, &req.from) {
            return Err(ConversationError::PermissionDenied);
        }

        let limit = req.limit.clamp(1, 100);
        let entries =
            store.entries_after_seq(&req.group, req.after_seq, limit + 1).map_err(internal)?;
        let has_more = entries.len() as u32 > limit;
        let mut out_entries = entries;
        if has_more {
            out_entries.pop();
        }
        let next_seq = out_entries.last().map(|e| e.seq).unwrap_or(req.after_seq);
        let seqs = out_entries.iter().map(|e| e.seq).collect();
        Ok(crate::dag::GroupSyncResponse {
            entries: out_entries.into_iter().map(crate::store::StoredDagEntry::into_wire).collect(),
            seqs,
            next_seq,
            has_more,
        })
    }

    /// Guest-facing `sync-now`: bounded by `conversation_sync_now_budget_ms`,
    /// which is itself kept inside `dispatch_epoch_timeout_secs` so a guest
    /// call never times out waiting on it. Always starts at the first
    /// member — a guest asking to sync now wants the members it can reach
    /// first served first, not a rotation.
    pub(crate) async fn sync_now_impl(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let budget_ms = store.config().conversation_sync_now_budget_ms;
        self.run_group_sync(service_id, conversation, budget_ms, Duration::from_secs(2), 0).await
    }

    /// The background periodic pass: its own, much larger budget
    /// (`conversation_background_sync_budget_ms`), a longer per-peer call
    /// timeout matching `push_group_entry`'s, and a rotating start offset
    /// so a budget that runs out partway through the roster does not
    /// starve the same tail members on every tick.
    pub(crate) async fn periodic_group_sync_pass(
        &self,
        service_id: &str,
        conversation: &str,
        start_offset: usize,
    ) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let budget_ms = store.config().conversation_background_sync_budget_ms;
        self.run_group_sync(
            service_id,
            conversation,
            budget_ms,
            Duration::from_secs(10),
            start_offset,
        )
        .await
    }

    async fn run_group_sync(
        &self,
        service_id: &str,
        conversation: &str,
        budget_ms: u64,
        per_peer_timeout: Duration,
        start_offset: usize,
    ) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != syneroym_rpc::ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        let mut members = store.current_members(conversation).map_err(internal)?;
        if !members.is_empty() {
            let offset = start_offset % members.len();
            members.rotate_left(offset);
        }
        let ident = store
            .local_identity_or_generate(crate::crypto::generate_identity_bytes)
            .map_err(internal)?;
        let sig_bytes: [u8; 32] =
            ident.sig_secret.as_slice().try_into().map_err(|_| {
                ConversationError::Internal("corrupt local signing key".to_string())
            })?;
        let sk = ed25519_dalek::SigningKey::from_bytes(&sig_bytes);

        let now = now_ms();
        let deadline = std::time::Instant::now() + Duration::from_millis(budget_ms);
        for m in members {
            if m.as_str() == service_id {
                continue;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            let cursor = store.sync_cursor(conversation, &m).map_err(internal)?;
            let mut nonce = [0u8; 16];
            rand::rng().fill_bytes(&mut nonce);
            let assertion =
                crate::dag::sign_peer_assertion(&sk, service_id, conversation, now, &nonce);
            let req_json = serde_json::to_value(&crate::dag::GroupSyncRequest {
                from: assertion,
                group: conversation.to_string(),
                after_seq: cursor,
                limit: 100,
            })
            .map_err(internal)?;
            let resp_json = match self
                .call_peer(service_id, &m, "group-sync", req_json, None, Some(per_peer_timeout))
                .await
            {
                Ok(j) => j,
                Err(_) => continue,
            };
            let resp: crate::dag::GroupSyncResponse = match serde_json::from_value(resp_json) {
                Ok(r) => r,
                Err(_) => continue,
            };
            // Advance the cursor to the highest seq that actually applied,
            // not blindly to `next_seq` — a page can contain an entry that
            // fails validation (e.g. still-arriving membership), and the
            // peer will keep offering that same page until it is resolved.
            let mut highest_applied_seq = cursor;
            for (seq, entry) in resp.seqs.into_iter().zip(resp.entries) {
                if entry.conversation_id != conversation {
                    continue;
                }
                let res = crate::group::validate_and_insert(&store, service_id, &conv, &entry);
                match res {
                    Ok((_, Some(msg))) => {
                        highest_applied_seq = highest_applied_seq.max(seq);
                        self.notify_message(service_id, msg.into_wire()).await;
                    }
                    Ok(_) => {
                        highest_applied_seq = highest_applied_seq.max(seq);
                    }
                    Err(e) => {
                        tracing::warn!(
                            service = service_id,
                            entry_id = entry.entry_id,
                            error = ?e,
                            "group sync validate_and_insert failed"
                        );
                    }
                }
            }
            let _ = store.set_sync_cursor(conversation, &m, highest_applied_seq, now);
        }
        self.apply_pending_entries(&store, service_id, conversation).await;
        Ok(())
    }
}

/// Resolves the key a `PeerAssertion` must verify under, and never trusts
/// the wire key *repeatedly*: a `group_members` row already holding a real
/// (DAG-confirmed, owner-signed) key always wins; failing that, a pinned
/// 1:1 session key; failing that, the wire key is accepted exactly once,
/// pinned into the placeholder `group_members` row it came from
/// (`pin_member_sig_key_if_placeholder`), and from then on treated the
/// same as any other confirmed key — a later request presenting a
/// *different* key for the same address fails signature verification
/// against the one already on file rather than being silently re-pinned.
/// This is what lets two members who share only this group (never a 1:1
/// conversation) verify each other's `group-push`/`group-sync` calls at
/// all, while keeping trust-on-first-use to the first call, not every call.
fn pinned_member_sig_key(
    store: &ConversationStore,
    conv_id: &str,
    address: &str,
    asserted_key: [u8; 32],
) -> Result<[u8; 32], ConversationError> {
    if let Ok(Some(k)) = store.member_sig_key(conv_id, address) {
        if k != [0u8; 32] {
            return Ok(k);
        }
        if let Ok(Some(sess)) = store.session(address) {
            return Ok(sess.pinned_sig_key);
        }
        let _ = store.pin_member_sig_key_if_placeholder(conv_id, address, &asserted_key);
        return Ok(asserted_key);
    }
    Err(ConversationError::PermissionDenied)
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

    /// Row 8: a `GroupKeyPayload` whose `owner` field does not match the
    /// verified sender of the envelope carrying it must be refused. Drives
    /// the real path (`peer_deliver_impl`), not just the store call it
    /// happens to use — a forged `owner` field is exactly what an
    /// unprivileged peer would send to try to seed a group it does not
    /// control.
    #[tokio::test]
    async fn group_key_payload_claiming_a_different_owner_than_the_signer_is_rejected() {
        use syneroym_rpc::ConversationHost;

        let dir = tempfile::tempdir().unwrap();
        let service = test_service(dir.path()).await;
        let crypto = X3dhDoubleRatchetCrypto::new();

        let bundle_bytes = service.prekey_bundle("victim", "attacker").await.unwrap();
        let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();

        let store_attacker = test_store();
        let mut session_attacker =
            crypto.begin_session(&store_attacker, "attacker", "victim", &bundle).await.unwrap();

        let identity = store_attacker
            .local_identity_or_generate(crate::crypto::generate_identity_bytes)
            .unwrap();
        let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&sig_bytes);

        // The payload's author is "attacker" (matches the session, so the
        // envelope itself is legitimately signed), but the embedded
        // GroupKeyPayload claims "some-other-owner" owns the group.
        let key_payload = crate::dag::GroupKeyPayload {
            group_id: "conv:forged".to_string(),
            epoch: 1,
            key: [5u8; 32],
            members: vec!["attacker".to_string(), "victim".to_string()],
            owner: "some-other-owner".to_string(),
        };
        let body = serde_json::to_vec(&key_payload).unwrap();
        let sig = envelope::sign(
            &signing_key,
            "msg:forged-key",
            &crate::ids::derive_conversation_id("victim", "attacker"),
            "attacker",
            1_000,
            crate::dag::GROUP_KEY_CONTENT_TYPE,
            &body,
        );
        let payload = DeliveryPayload {
            message_id: "msg:forged-key".to_string(),
            conversation_id: crate::ids::derive_conversation_id("victim", "attacker"),
            author: "attacker".to_string(),
            sender_timestamp_ms: 1_000,
            content_type: crate::dag::GROUP_KEY_CONTENT_TYPE.to_string(),
            body,
            signature: sig,
        };
        let env = crypto.encrypt(&mut session_attacker, &payload).unwrap();
        let env_bytes = serde_json::to_vec(&env).unwrap();

        let result = service.peer_deliver("victim", "attacker", env_bytes).await;
        assert!(
            matches!(result, Err(ConversationError::PermissionDenied)),
            "a group-key payload whose owner does not match the signer must be rejected, got \
             {result:?}"
        );
    }

    #[tokio::test]
    async fn group_push_with_unregistered_assertion_sender_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let service = test_service(dir.path()).await;
        let store = service.store_for("svc:receiver").await.unwrap();

        // Create group on receiver with only svc:owner as member
        {
            let conn = store.conn().lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            ConversationStore::get_or_create_group_shell(&tx, "conv:g1", "svc:owner", 1, 1000)
                .unwrap();
            let payload = crate::dag::MembershipPayload {
                action: "add".to_string(),
                subject_address: "svc:owner".to_string(),
                subject_sig_key: [1u8; 32],
                new_epoch: 1,
                member_list_hash: "hash".to_string(),
            };
            ConversationStore::apply_membership(&tx, "conv:g1", &payload).unwrap();
            tx.commit().unwrap();
        }

        // Stranger attempts group-push
        let sk = SigningKey::generate(&mut rand_core::OsRng);
        let entry = crate::dag::WireEntry {
            entry_id: "ent:1".to_string(),
            conversation_id: "conv:g1".to_string(),
            author: "svc:stranger".to_string(),
            sender_timestamp_ms: 1000,
            epoch: 1,
            kind: crate::dag::EntryKind::Message,
            parents: vec![],
            ciphertext: Some(vec![1]),
            nonce: Some([0u8; 12]),
            payload: None,
            signature: [0u8; 64],
        };
        let assertion =
            crate::dag::sign_peer_assertion(&sk, "svc:stranger", "conv:g1", 1000, &[0u8; 16]);
        let req = crate::dag::GroupPushRequest {
            from: assertion,
            group: "conv:g1".to_string(),
            entries: vec![entry],
        };

        let res = service.group_push_impl("svc:receiver", "svc:stranger", req).await;
        assert!(matches!(res, Err(ConversationError::PermissionDenied)));
    }
}
