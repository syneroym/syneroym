//! Implementation of `ConversationHost` for `ConversationService`.

use syneroym_rpc::{
    ConversationChangePage, ConversationDeliveryState, ConversationError, ConversationExportChunk,
    ConversationGroupInfo, ConversationHistoryPage, ConversationHost, ConversationKind,
    ConversationMessage, ConversationSummary,
};

use crate::{
    ConversationService, crypto,
    dag::{
        DELETION_REQUEST_CONTENT_TYPE, GroupPushRequest, GroupSyncRequest, deletion_request_body,
    },
    ids::derive_conversation_id,
    internal, store,
    store::StoredMessage,
};

#[async_trait::async_trait]
impl ConversationHost for ConversationService {
    async fn open_direct(
        &self,
        service_id: &str,
        peer_address: &str,
    ) -> Result<String, ConversationError> {
        if peer_address.is_empty() || peer_address == service_id {
            return Err(ConversationError::InvalidArgument(
                "peer address must be non-empty and not this service's own address".to_string(),
            ));
        }
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv_id = derive_conversation_id(service_id, peer_address);
        let id = store
            .get_or_create_direct(peer_address, &conv_id, store::now_ms())
            .map_err(internal)?;
        store.mark_direct_opened(&id).map_err(internal)?;
        Ok(id)
    }

    async fn conversations(
        &self,
        service_id: &str,
    ) -> Result<Vec<ConversationSummary>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let rows = store.list_conversations().map_err(internal)?;
        let mut summaries = Vec::new();
        for r in rows {
            let participants = if r.kind == ConversationKind::Group {
                store.current_members(&r.id).unwrap_or_default()
            } else {
                let mut p = vec![service_id.to_string()];
                if let Some(peer) = r.peer_address {
                    p.push(peer);
                }
                p.sort();
                p
            };
            let count = store.message_count(&r.id).unwrap_or(0);
            summaries.push(ConversationSummary {
                id: r.id,
                kind: r.kind,
                participants,
                created_at: r.created_at_ms,
                last_activity_at: r.last_activity_ms,
                message_count: count,
                name: r.name,
                restored: r.restored,
            });
        }
        Ok(summaries)
    }

    async fn send(
        &self,
        service_id: &str,
        conversation: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<String, ConversationError> {
        // Reject the host's own reserved content types so apps cannot send
        // system messages and have them treated as ordinary visible rows.
        if content_type == crate::dag::DELETION_REQUEST_CONTENT_TYPE
            || content_type == crate::dag::REFUSAL_NOTICE_CONTENT_TYPE
        {
            return Err(ConversationError::InvalidArgument(
                "this content type is reserved".to_string(),
            ));
        }
        let store = self.store_for(service_id).await.map_err(internal)?;
        if body.len() as u32 > store.config().max_body_bytes {
            return Err(ConversationError::QuotaExceeded);
        }
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind == ConversationKind::Group {
            return self.send_group(service_id, &store, &conv, content_type, &body).await;
        }
        let peer_address = conv.peer_address.ok_or_else(|| {
            ConversationError::Internal(
                "direct conversation is missing its peer address".to_string(),
            )
        })?;
        self.enqueue_direct(&store, service_id, &peer_address, content_type, &body, false).await
    }

    async fn history(
        &self,
        service_id: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<ConversationHistoryPage, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        store.history(conversation, limit, cursor.as_deref()).map_err(internal)
    }

    async fn delivery_status(
        &self,
        service_id: &str,
        message: &str,
    ) -> Result<ConversationDeliveryState, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        store
            .get_message(message)
            .map_err(internal)?
            .map(|m| m.state)
            .ok_or(ConversationError::NotFound)
    }

    async fn outbox(
        &self,
        service_id: &str,
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        Ok(store
            .outbox_messages()
            .map_err(internal)?
            .into_iter()
            .map(StoredMessage::into_wire)
            .collect())
    }

    async fn retry(&self, service_id: &str, message: &str) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let msg =
            store.get_message(message).map_err(internal)?.ok_or(ConversationError::NotFound)?;
        if msg.restored {
            return Err(ConversationError::InvalidArgument(
                "cannot retry a restored message".to_string(),
            ));
        }
        if msg.state != ConversationDeliveryState::Failed {
            return Err(ConversationError::InvalidArgument(
                "only a failed message can be retried".to_string(),
            ));
        }
        let conv = store
            .get_conversation(&msg.conversation_id)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        let now = store::now_ms();
        store.restart_pending(message, now).map_err(internal)?;

        if conv.kind == ConversationKind::Group {
            let failed_members = {
                let conn = store
                    .conn()
                    .lock()
                    .map_err(|_| ConversationError::Internal("store lock poisoned".to_string()))?;
                let mut stmt = conn
                    .prepare(
                        "SELECT member_address FROM message_recipients WHERE message_id = ?1 AND \
                         state = 'failed'",
                    )
                    .map_err(internal)?;
                let mut rows = stmt.query(rusqlite::params![message]).map_err(internal)?;
                let mut failed = Vec::new();
                while let Some(r) = rows.next().map_err(internal)? {
                    failed.push(r.get::<_, String>(0).map_err(internal)?);
                }
                failed
            };

            for m in failed_members {
                store
                    .set_recipient_state(message, &m, ConversationDeliveryState::Pending, None)
                    .map_err(internal)?;
                let payload = serde_json::to_vec(&store::OutboxItem {
                    message_id: message.to_string(),
                    peer_address: m.clone(),
                    group: Some(conv.id.clone()),
                })
                .map_err(internal)?;
                store
                    .queue()
                    .enqueue(&conv.id, &format!("{message}:{m}"), &payload, now)
                    .map_err(internal)?;
            }
        } else {
            let peer_address = conv.peer_address.ok_or_else(|| {
                ConversationError::Internal(
                    "direct conversation is missing its peer address".to_string(),
                )
            })?;
            let payload = serde_json::to_vec(&store::OutboxItem {
                message_id: message.to_string(),
                peer_address,
                group: None,
            })
            .map_err(internal)?;
            store
                .queue()
                .enqueue(&msg.conversation_id, message, &payload, now)
                .map_err(internal)?;
        }
        Ok(())
    }

    async fn create_group(&self, service_id: &str) -> Result<String, ConversationError> {
        self.create_group_impl(service_id).await
    }

    async fn add_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError> {
        self.change_membership_impl(service_id, conversation, member_address, "add").await
    }

    async fn remove_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError> {
        self.change_membership_impl(service_id, conversation, member_address, "remove").await
    }

    async fn members(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<Vec<String>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        store.current_members(conversation).map_err(internal)
    }

    async fn sync_now(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<(), ConversationError> {
        self.sync_now_impl(service_id, conversation).await
    }

    async fn group_info(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<ConversationGroupInfo, ConversationError> {
        self.group_info_impl(service_id, conversation).await
    }

    async fn get_message(
        &self,
        service_id: &str,
        message: &str,
    ) -> Result<ConversationMessage, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        store
            .get_message(message)
            .map_err(internal)?
            .filter(|m| !m.system && (m.outgoing || m.admission == "accepted"))
            .map(StoredMessage::into_wire)
            .ok_or(ConversationError::NotFound)
    }

    async fn delete_message(
        &self,
        service_id: &str,
        message: &str,
        ask_others: bool,
    ) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let msg = match store.get_message(message).map_err(internal)? {
            Some(m) => m,
            None => {
                if store.has_dag_entry(message).map_err(internal)? {
                    return Err(ConversationError::InvalidArgument(
                        "this row records a group change and cannot be deleted".to_string(),
                    ));
                }
                return Err(ConversationError::NotFound);
            }
        };
        if msg.system {
            return Err(ConversationError::NotFound);
        }
        // Only the owner can delete outgoing messages; for incoming messages
        // only accepted ones are visible to the app, so it makes sense to
        // reject held/undecided/dropped rows with NotFound (the app never saw
        // them) rather than deleting the body of a row that was never indexed.
        if !msg.outgoing && msg.admission != "accepted" {
            return Err(ConversationError::NotFound);
        }
        let conv = store
            .get_conversation(&msg.conversation_id)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        let was_pending = msg.outgoing && msg.state == ConversationDeliveryState::Pending;
        let now = store::now_ms();
        store.delete_message(&conv.id, message, now).map_err(internal)?;

        if ask_others && !was_pending {
            let body = deletion_request_body(message);
            if conv.kind == ConversationKind::Group {
                let _ = self
                    .send_group(service_id, &store, &conv, DELETION_REQUEST_CONTENT_TYPE, &body)
                    .await;
            } else if let Some(peer) = conv.peer_address {
                let _ = self
                    .enqueue_direct(
                        &store,
                        service_id,
                        &peer,
                        DELETION_REQUEST_CONTENT_TYPE,
                        &body,
                        true,
                    )
                    .await;
            }
        }
        Ok(())
    }

    async fn readmit(
        &self,
        service_id: &str,
        conversation: &str,
        reasons: Vec<String>,
    ) -> Result<u32, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        store.readmit(conversation, &reasons).map_err(internal)
    }

    async fn changes(
        &self,
        service_id: &str,
        conversation: &str,
        after_seq: u64,
        limit: u32,
    ) -> Result<ConversationChangePage, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        store.changes(conversation, after_seq, limit).map_err(internal)
    }

    async fn search(
        &self,
        service_id: &str,
        query: &str,
        conversation: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let q = query.to_string();
        let c = conversation.map(str::to_string);
        let msgs = tokio::task::spawn_blocking(move || store.search(&q, c.as_deref(), limit))
            .await
            .map_err(|e| internal(anyhow::anyhow!("spawn_blocking failed: {e}")))?
            .map_err(internal)?;
        Ok(msgs.into_iter().map(StoredMessage::into_wire).collect())
    }

    async fn set_group_name(
        &self,
        service_id: &str,
        conversation: &str,
        name: &str,
    ) -> Result<(), ConversationError> {
        self.set_group_name_impl(service_id, conversation, name).await
    }

    async fn transcript_digest(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<String, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let c = conversation.to_string();
        tokio::task::spawn_blocking(move || store.transcript_digest(&c))
            .await
            .map_err(|e| internal(anyhow::anyhow!("spawn_blocking failed: {e}")))?
            .map_err(internal)
    }

    async fn export_history(
        &self,
        service_id: &str,
        cursor: Option<String>,
    ) -> Result<ConversationExportChunk, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        tokio::task::spawn_blocking(move || store.export_history(cursor))
            .await
            .map_err(|e| internal(anyhow::anyhow!("spawn_blocking failed: {e}")))?
            .map_err(internal)
    }

    async fn import_history(
        &self,
        service_id: &str,
        data: Vec<u8>,
    ) -> Result<u32, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        tokio::task::spawn_blocking(move || store.import_history(&data))
            .await
            .map_err(|e| internal(anyhow::anyhow!("spawn_blocking failed: {e}")))?
            .map_err(internal)
    }

    async fn group_push(
        &self,
        service_id: &str,
        requester_did: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError> {
        let req: GroupPushRequest = serde_json::from_slice(&payload).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable group-push payload: {e}"))
        })?;
        let ack = self.group_push_impl(service_id, requester_did, req).await?;
        serde_json::to_vec(&ack).map_err(internal)
    }

    async fn group_sync(
        &self,
        service_id: &str,
        requester_did: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError> {
        let req: GroupSyncRequest = serde_json::from_slice(&payload).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable group-sync payload: {e}"))
        })?;
        let resp = self.group_sync_impl(service_id, requester_did, req).await?;
        serde_json::to_vec(&resp).map_err(internal)
    }

    async fn prekey_bundle(
        &self,
        service_id: &str,
        requester_did: &str,
    ) -> Result<Vec<u8>, ConversationError> {
        if requester_did.is_empty() {
            return Err(ConversationError::PermissionDenied);
        }
        let store = self.store_for(service_id).await.map_err(internal)?;
        if !store.record_prekey_request(requester_did, store::now_ms()).map_err(internal)? {
            return Err(ConversationError::PermissionDenied);
        }
        let bundle = self
            .crypto
            .prekey_bundle(&store)
            .await
            .map_err(|e| ConversationError::Internal(e.to_string()))?;
        serde_json::to_vec(&bundle).map_err(internal)
    }

    async fn peer_deliver(
        &self,
        service_id: &str,
        requester_did: &str,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError> {
        let env: crypto::Envelope = serde_json::from_slice(&envelope).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable envelope: {e}"))
        })?;
        let ack = self.peer_deliver_impl(service_id, requester_did, env).await?;
        serde_json::to_vec(&ack).map_err(internal)
    }
}
