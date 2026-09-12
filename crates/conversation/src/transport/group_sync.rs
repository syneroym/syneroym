use std::time::Duration;

use rand::RngCore;
use syneroym_rpc::ConversationError;

use super::{Disposition, internal};
use crate::{
    ConversationService,
    store::{ConversationStore, StoredMessage, now_ms},
};

impl ConversationService {
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
            // Once any entry fails, stop advancing: a later entry in the
            // same page succeeding must not push the cursor past the
            // failure, or it is never retried again -- entries only fail
            // transiently here (a dependency, like a member's real join
            // epoch, that a still-in-flight earlier entry will resolve).
            let mut highest_applied_seq = cursor;
            let mut saw_failure = false;
            for (seq, entry) in resp.seqs.into_iter().zip(resp.entries) {
                if entry.conversation_id != conversation {
                    continue;
                }
                let res = crate::group::validate_and_insert(&store, service_id, &conv, &entry);
                match res {
                    Ok((_, Some(msg))) => {
                        if !saw_failure {
                            highest_applied_seq = highest_applied_seq.max(seq);
                        }
                        self.notify_message(service_id, msg.into_wire()).await;
                    }
                    Ok(_) => {
                        if !saw_failure {
                            highest_applied_seq = highest_applied_seq.max(seq);
                        }
                    }
                    Err(e) => {
                        saw_failure = true;
                        tracing::warn!(
                            service = service_id,
                            entry_id = entry.entry_id,
                            error = ?e,
                            epoch = entry.epoch,
                            kind = ?entry.kind,
                            payload = ?entry.payload,
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
