use super::*;

// ---- conversation: out (host -> guest) ----

pub(crate) fn conversation_error_out(v: HostConversationError) -> GuestConversationError {
    match v {
        HostConversationError::PermissionDenied => GuestConversationError::PermissionDenied,
        HostConversationError::NotFound => GuestConversationError::NotFound,
        HostConversationError::InvalidArgument(s) => GuestConversationError::InvalidArgument(s),
        HostConversationError::Unreachable(s) => GuestConversationError::Unreachable(s),
        HostConversationError::QuotaExceeded => GuestConversationError::QuotaExceeded,
        HostConversationError::Internal(s) => GuestConversationError::Internal(s),
    }
}

pub(crate) fn delivery_state_out(v: HostDeliveryState) -> GuestDeliveryState {
    match v {
        HostDeliveryState::Pending => GuestDeliveryState::Pending,
        HostDeliveryState::Delivered => GuestDeliveryState::Delivered,
        HostDeliveryState::Failed => GuestDeliveryState::Failed,
    }
}

fn conversation_kind_out(v: HostConversationKind) -> GuestConversationKind {
    match v {
        HostConversationKind::Direct => GuestConversationKind::Direct,
        HostConversationKind::Group => GuestConversationKind::Group,
    }
}

pub(crate) fn conversation_summary_out(v: HostConversationSummary) -> GuestConversationSummary {
    GuestConversationSummary {
        id: v.id,
        kind: conversation_kind_out(v.kind),
        participants: v.participants,
        created_at: v.created_at,
        last_activity_at: v.last_activity_at,
    }
}

pub(crate) fn message_out(v: HostMessage) -> GuestMessage {
    GuestMessage {
        id: v.id,
        conversation: v.conversation,
        author: v.author,
        sender_timestamp: v.sender_timestamp,
        received_at: v.received_at,
        content_type: v.content_type,
        body: v.body,
        state: delivery_state_out(v.state),
        verified: v.verified,
        last_error: v.last_error,
    }
}

pub(crate) fn history_page_out(v: HostHistoryPage) -> GuestHistoryPage {
    GuestHistoryPage {
        messages: v.messages.into_iter().map(message_out).collect(),
        next_cursor: v.next_cursor,
    }
}

pub(crate) fn membership_event_out(v: HostMembershipEvent) -> GuestMembershipEvent {
    GuestMembershipEvent {
        entry: v.entry,
        action: v.action,
        subject: v.subject,
        epoch: v.epoch,
        sender_timestamp: v.sender_timestamp,
    }
}

// ---- conversation: `syneroym-rpc`'s plain `ConversationMessage`/
// `ConversationDeliveryState` -> the guest WIT shape, for
// `NativeHostFactory`'s `ConversationNotifier` impl (factory.rs), which
// receives the `syneroym-rpc` shape and must hand `ConversationSink` the
// guest one.

pub(crate) fn rpc_delivery_state_to_guest(v: RpcDeliveryState) -> GuestDeliveryState {
    match v {
        RpcDeliveryState::Pending => GuestDeliveryState::Pending,
        RpcDeliveryState::Delivered => GuestDeliveryState::Delivered,
        RpcDeliveryState::Failed => GuestDeliveryState::Failed,
    }
}

pub(crate) fn rpc_message_to_guest(v: RpcMessage) -> GuestMessage {
    GuestMessage {
        id: v.id,
        conversation: v.conversation,
        author: v.author,
        sender_timestamp: v.sender_timestamp,
        received_at: v.received_at,
        content_type: v.content_type,
        body: v.body,
        state: rpc_delivery_state_to_guest(v.state),
        verified: v.verified,
        last_error: v.last_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_error_round_trips_all_variants() {
        assert!(matches!(
            conversation_error_out(HostConversationError::PermissionDenied),
            GuestConversationError::PermissionDenied
        ));
        assert!(matches!(
            conversation_error_out(HostConversationError::NotFound),
            GuestConversationError::NotFound
        ));
        assert!(matches!(
            conversation_error_out(HostConversationError::InvalidArgument("a".to_string())),
            GuestConversationError::InvalidArgument(s) if s == "a"
        ));
        assert!(matches!(
            conversation_error_out(HostConversationError::Unreachable("b".to_string())),
            GuestConversationError::Unreachable(s) if s == "b"
        ));
        assert!(matches!(
            conversation_error_out(HostConversationError::QuotaExceeded),
            GuestConversationError::QuotaExceeded
        ));
        assert!(matches!(
            conversation_error_out(HostConversationError::Internal("c".to_string())),
            GuestConversationError::Internal(s) if s == "c"
        ));
    }

    #[test]
    fn delivery_state_round_trips_all_variants() {
        assert!(matches!(
            delivery_state_out(HostDeliveryState::Pending),
            GuestDeliveryState::Pending
        ));
        assert!(matches!(
            delivery_state_out(HostDeliveryState::Delivered),
            GuestDeliveryState::Delivered
        ));
        assert!(matches!(
            delivery_state_out(HostDeliveryState::Failed),
            GuestDeliveryState::Failed
        ));
    }

    #[test]
    fn conversation_kind_round_trips_via_summary() {
        let direct = conversation_summary_out(HostConversationSummary {
            id: "conv:1".to_string(),
            kind: HostConversationKind::Direct,
            participants: vec!["a".to_string(), "b".to_string()],
            created_at: 1,
            last_activity_at: 2,
        });
        assert!(matches!(direct.kind, GuestConversationKind::Direct));
        assert_eq!(direct.participants, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn message_round_trips() {
        let h = HostMessage {
            id: "msg:1".to_string(),
            conversation: "conv:1".to_string(),
            author: "did:key:zA".to_string(),
            sender_timestamp: 1_000,
            received_at: 1_001,
            content_type: "text/plain".to_string(),
            body: vec![1, 2, 3],
            state: HostDeliveryState::Delivered,
            verified: true,
            last_error: None,
        };
        let g = message_out(h);
        assert_eq!(g.id, "msg:1");
        assert_eq!(g.body, vec![1, 2, 3]);
        assert!(matches!(g.state, GuestDeliveryState::Delivered));
        assert!(g.verified);
    }

    #[test]
    fn history_page_round_trips() {
        let h = HostHistoryPage {
            messages: vec![HostMessage {
                id: "msg:1".to_string(),
                conversation: "conv:1".to_string(),
                author: "did:key:zA".to_string(),
                sender_timestamp: 1,
                received_at: 1,
                content_type: "text/plain".to_string(),
                body: vec![],
                state: HostDeliveryState::Pending,
                verified: true,
                last_error: None,
            }],
            next_cursor: Some("cursor-1".to_string()),
        };
        let g = history_page_out(h);
        assert_eq!(g.messages.len(), 1);
        assert_eq!(g.next_cursor, Some("cursor-1".to_string()));
    }

    #[test]
    fn rpc_message_to_guest_round_trips() {
        let rpc_msg = RpcMessage {
            id: "msg:1".to_string(),
            conversation: "conv:1".to_string(),
            author: "did:key:zA".to_string(),
            sender_timestamp: 1,
            received_at: 1,
            content_type: "text/plain".to_string(),
            body: vec![9],
            state: RpcDeliveryState::Failed,
            verified: false,
            last_error: Some("gave up".to_string()),
        };
        let g = rpc_message_to_guest(rpc_msg);
        assert_eq!(g.body, vec![9]);
        assert!(matches!(g.state, GuestDeliveryState::Failed));
        assert_eq!(g.last_error, Some("gave up".to_string()));
    }
}
