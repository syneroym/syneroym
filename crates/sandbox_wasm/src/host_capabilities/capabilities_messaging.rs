use super::*;

impl websocket::Host for HostState {
    async fn send(
        &mut self,
        conn: String,
        frame: Vec<u8>,
        kind: WitFrameKind,
    ) -> Result<(), String> {
        let k = match kind {
            WitFrameKind::Text => AppFrameKind::Text,
            WitFrameKind::Binary => AppFrameKind::Binary,
        };
        self.websocket_senders.send(&self.component_id, &conn, frame, k).await
    }
}

/// WIT `syneroym:conversation` <-> `syneroym-rpc`'s plain
/// `ConversationHost` types -- the same split `data-layer`'s own `Host`
/// impl already draws between WIT shapes and `syneroym-data-db`'s.
mod conversation_wire {
    use syneroym_rpc as rpc;
    use syneroym_wit_interfaces::conversation_host::syneroym::conversation::conversation as wit;

    pub(super) fn map_error(e: rpc::ConversationError) -> wit::ConversationError {
        match e {
            rpc::ConversationError::PermissionDenied => wit::ConversationError::PermissionDenied,
            rpc::ConversationError::NotFound => wit::ConversationError::NotFound,
            rpc::ConversationError::InvalidArgument(m) => {
                wit::ConversationError::InvalidArgument(m)
            }
            rpc::ConversationError::Unreachable(m) => wit::ConversationError::Unreachable(m),
            rpc::ConversationError::QuotaExceeded => wit::ConversationError::QuotaExceeded,
            rpc::ConversationError::Internal(m) => wit::ConversationError::Internal(m),
        }
    }

    pub(super) fn no_capability() -> wit::ConversationError {
        wit::ConversationError::Internal("no conversation capability on this node".to_string())
    }

    fn map_kind(k: rpc::ConversationKind) -> wit::ConversationKind {
        match k {
            rpc::ConversationKind::Direct => wit::ConversationKind::Direct,
            rpc::ConversationKind::Group => wit::ConversationKind::Group,
        }
    }

    pub(super) fn map_state(s: rpc::ConversationDeliveryState) -> wit::DeliveryState {
        match s {
            rpc::ConversationDeliveryState::Pending => wit::DeliveryState::Pending,
            rpc::ConversationDeliveryState::Delivered => wit::DeliveryState::Delivered,
            rpc::ConversationDeliveryState::Failed => wit::DeliveryState::Failed,
        }
    }

    pub(super) fn map_summary(s: rpc::ConversationSummary) -> wit::ConversationSummary {
        wit::ConversationSummary {
            id: s.id,
            kind: map_kind(s.kind),
            participants: s.participants,
            peer_address: s.peer_address,
            created_at: s.created_at,
            last_activity_at: s.last_activity_at,
            message_count: s.message_count,
            name: s.name,
            restored: s.restored,
        }
    }

    pub(super) fn map_message(m: rpc::ConversationMessage) -> wit::Message {
        wit::Message {
            id: m.id,
            conversation: m.conversation,
            author: m.author,
            sender_timestamp: m.sender_timestamp,
            received_at: m.received_at,
            content_type: m.content_type,
            body: m.body,
            state: map_state(m.state),
            verified: m.verified,
            last_error: m.last_error,
            outgoing: m.outgoing,
            deleted_at: m.deleted_at,
            restored: m.restored,
            visible_seq: m.visible_seq,
            refused: m.refused,
        }
    }

    pub(super) fn map_name_event(n: rpc::ConversationNameEvent) -> wit::NameEvent {
        wit::NameEvent { entry: n.entry, name: n.name, sender_timestamp: n.sender_timestamp }
    }

    pub(super) fn map_history_item(i: rpc::ConversationHistoryItem) -> wit::HistoryItem {
        match i {
            rpc::ConversationHistoryItem::Message(m) => wit::HistoryItem::Message(map_message(m)),
            rpc::ConversationHistoryItem::Membership(e) => {
                wit::HistoryItem::Membership(map_membership_event(e))
            }
            rpc::ConversationHistoryItem::GroupName(n) => {
                wit::HistoryItem::GroupName(map_name_event(n))
            }
        }
    }

    pub(super) fn map_history(p: rpc::ConversationHistoryPage) -> wit::HistoryPage {
        wit::HistoryPage {
            items: p.items.into_iter().map(map_history_item).collect(),
            next_cursor: p.next_cursor,
        }
    }

    pub(super) fn map_change_page(p: rpc::ConversationChangePage) -> wit::ChangePage {
        wit::ChangePage {
            messages: p.messages.into_iter().map(map_message).collect(),
            last_seq: p.last_seq,
        }
    }

    pub(super) fn map_transcript(t: rpc::ConversationTranscript) -> wit::Transcript {
        wit::Transcript { digest: t.digest, rows: t.rows }
    }

    pub(super) fn map_export_chunk(c: rpc::ConversationExportChunk) -> wit::ExportChunk {
        wit::ExportChunk { data: c.data, next_cursor: c.next_cursor }
    }

    pub(super) fn map_membership_event(
        e: rpc::ConversationMembershipEvent,
    ) -> wit::MembershipEvent {
        wit::MembershipEvent {
            entry: e.entry,
            action: e.action,
            subject: e.subject,
            epoch: e.epoch,
            sender_timestamp: e.sender_timestamp,
        }
    }

    pub(super) fn map_group_info(g: rpc::ConversationGroupInfo) -> wit::GroupInfo {
        wit::GroupInfo {
            owner: g.owner,
            is_owner: g.is_owner,
            is_member: g.is_member,
            members: g.members,
            epoch: g.epoch,
            key_epoch: g.key_epoch,
            key_stored_at: g.key_stored_at,
            name: g.name,
            restored: g.restored,
        }
    }
}

impl wit_conversation::Host for HostState {
    async fn open_direct(
        &mut self,
        peer_address: String,
    ) -> Result<String, wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.open_direct(&self.component_id, &peer_address)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn conversations(
        &mut self,
    ) -> Result<Vec<wit_conversation::ConversationSummary>, wit_conversation::ConversationError>
    {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.conversations(&self.component_id)
            .await
            .map(|v| v.into_iter().map(conversation_wire::map_summary).collect())
            .map_err(conversation_wire::map_error)
    }

    async fn send(
        &mut self,
        conversation: String,
        content_type: String,
        body: Vec<u8>,
    ) -> Result<String, wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.send(&self.component_id, &conversation, &content_type, body)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn history(
        &mut self,
        conversation: String,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<wit_conversation::HistoryPage, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.history(&self.component_id, &conversation, limit, cursor)
            .await
            .map(conversation_wire::map_history)
            .map_err(conversation_wire::map_error)
    }

    async fn delivery_status(
        &mut self,
        message: String,
    ) -> Result<wit_conversation::DeliveryState, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.delivery_status(&self.component_id, &message)
            .await
            .map(conversation_wire::map_state)
            .map_err(conversation_wire::map_error)
    }

    async fn outbox(
        &mut self,
    ) -> Result<Vec<wit_conversation::Message>, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.outbox(&self.component_id)
            .await
            .map(|v| v.into_iter().map(conversation_wire::map_message).collect())
            .map_err(conversation_wire::map_error)
    }

    async fn retry(&mut self, message: String) -> Result<(), wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.retry(&self.component_id, &message).await.map_err(conversation_wire::map_error)
    }

    async fn create_group(&mut self) -> Result<String, wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.create_group(&self.component_id).await.map_err(conversation_wire::map_error)
    }

    async fn add_member(
        &mut self,
        conversation: String,
        member_address: String,
    ) -> Result<(), wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.add_member(&self.component_id, &conversation, &member_address)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn remove_member(
        &mut self,
        conversation: String,
        member_address: String,
    ) -> Result<(), wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.remove_member(&self.component_id, &conversation, &member_address)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn members(
        &mut self,
        conversation: String,
    ) -> Result<Vec<String>, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.members(&self.component_id, &conversation).await.map_err(conversation_wire::map_error)
    }

    async fn sync_now(
        &mut self,
        conversation: String,
    ) -> Result<(), wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.sync_now(&self.component_id, &conversation).await.map_err(conversation_wire::map_error)
    }

    async fn get_group_info(
        &mut self,
        conversation: String,
    ) -> Result<wit_conversation::GroupInfo, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.group_info(&self.component_id, &conversation)
            .await
            .map(conversation_wire::map_group_info)
            .map_err(conversation_wire::map_error)
    }

    async fn get_message(
        &mut self,
        message: String,
    ) -> Result<wit_conversation::Message, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.get_message(&self.component_id, &message)
            .await
            .map(conversation_wire::map_message)
            .map_err(conversation_wire::map_error)
    }

    async fn delete_message(
        &mut self,
        message: String,
        ask_others: bool,
    ) -> Result<(), wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.delete_message(&self.component_id, &message, ask_others)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn readmit(
        &mut self,
        conversation: String,
        reasons: Vec<String>,
    ) -> Result<u32, wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.readmit(&self.component_id, &conversation, reasons)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn changes(
        &mut self,
        conversation: String,
        after_seq: u64,
        limit: u32,
    ) -> Result<wit_conversation::ChangePage, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.changes(&self.component_id, &conversation, after_seq, limit)
            .await
            .map(conversation_wire::map_change_page)
            .map_err(conversation_wire::map_error)
    }

    async fn search(
        &mut self,
        query: String,
        conversation: Option<String>,
        limit: u32,
    ) -> Result<Vec<wit_conversation::Message>, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.search(&self.component_id, &query, conversation.as_deref(), limit)
            .await
            .map(|v| v.into_iter().map(conversation_wire::map_message).collect())
            .map_err(conversation_wire::map_error)
    }

    async fn set_group_name(
        &mut self,
        conversation: String,
        name: String,
    ) -> Result<(), wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.set_group_name(&self.component_id, &conversation, &name)
            .await
            .map_err(conversation_wire::map_error)
    }

    async fn transcript_digest(
        &mut self,
        conversation: String,
    ) -> Result<wit_conversation::Transcript, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.transcript_digest(&self.component_id, &conversation)
            .await
            .map(conversation_wire::map_transcript)
            .map_err(conversation_wire::map_error)
    }

    async fn export_history(
        &mut self,
        cursor: Option<String>,
    ) -> Result<wit_conversation::ExportChunk, wit_conversation::ConversationError> {
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.export_history(&self.component_id, cursor)
            .await
            .map(conversation_wire::map_export_chunk)
            .map_err(conversation_wire::map_error)
    }

    async fn import_history(
        &mut self,
        data: Vec<u8>,
    ) -> Result<u32, wit_conversation::ConversationError> {
        if self.read_only {
            return Err(conversation_wire::map_error(RpcConversationError::PermissionDenied));
        }
        let conv = self.conversation.upgrade().ok_or_else(conversation_wire::no_capability)?;
        conv.import_history(&self.component_id, data).await.map_err(conversation_wire::map_error)
    }
}
