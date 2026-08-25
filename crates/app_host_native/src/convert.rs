//! Guest-vocabulary <-> host-vocabulary conversions. Both sides are
//! generated from the same `.wit`, so every one is a field-for-field copy;
//! this module exists so that copy happens in exactly one place.

use syneroym_app_host::types::{
    app_config::ConfigError as GuestConfigError,
    blob_store::BlobError as GuestBlobError,
    conversation::{
        ConversationError as GuestConversationError, ConversationKind as GuestConversationKind,
        ConversationSummary as GuestConversationSummary, DeliveryState as GuestDeliveryState,
        HistoryPage as GuestHistoryPage, MembershipEvent as GuestMembershipEvent,
        Message as GuestMessage,
    },
    data_layer::{
        CollectionSchema as GuestCollectionSchema, DataLayerError as GuestDataLayerError,
        IndexDefinition as GuestIndexDefinition, IndexType as GuestIndexType,
        Mutation as GuestMutation, PatchMutation as GuestPatchMutation,
        QueryOptions as GuestQueryOptions, QueryResult as GuestQueryResult,
        RawQueryResult as GuestRawQueryResult, RecordReadValue as GuestRecordReadValue,
        RecordWriteValue as GuestRecordWriteValue, SqlValue as GuestSqlValue,
    },
    messaging::MessagingError as GuestMessagingError,
    proxy::{
        CallOptions as GuestCallOptions, CallTarget as GuestCallTarget,
        CalleeError as GuestCalleeError, ProxyError as GuestProxyError,
    },
    vault::VaultError as GuestVaultError,
};
use syneroym_rpc::{
    ConversationDeliveryState as RpcDeliveryState, ConversationMessage as RpcMessage,
};
use syneroym_wit_interfaces::{
    conversation_host::syneroym::conversation::conversation::{
        ConversationError as HostConversationError, ConversationKind as HostConversationKind,
        ConversationSummary as HostConversationSummary, DeliveryState as HostDeliveryState,
        HistoryPage as HostHistoryPage, MembershipEvent as HostMembershipEvent,
        Message as HostMessage,
    },
    host::syneroym::{
        app_config::app_config::ConfigError as HostConfigError,
        blob_store::blob_store::BlobError as HostBlobError,
        data_layer::store::{
            CollectionSchema as HostCollectionSchema, DataLayerError as HostDataLayerError,
            IndexDefinition as HostIndexDefinition, IndexType as HostIndexType,
            Mutation as HostMutation, PatchMutation as HostPatchMutation,
            QueryOptions as HostQueryOptions, QueryResult as HostQueryResult,
            RawQueryResult as HostRawQueryResult, RecordReadValue as HostRecordReadValue,
            RecordWriteValue as HostRecordWriteValue, SqlValue as HostSqlValue,
        },
        messaging::host_api::MessagingError as HostMessagingError,
        proxy::proxy::{
            CallOptions as HostCallOptions, CallTarget as HostCallTarget,
            CalleeError as HostCalleeError, ProxyError as HostProxyError,
        },
        vault::vault::VaultError as HostVaultError,
    },
};

// ---- data-layer: in (guest -> host) ----

pub(crate) fn index_type_in(v: GuestIndexType) -> HostIndexType {
    match v {
        GuestIndexType::String => HostIndexType::String,
        GuestIndexType::Numeric => HostIndexType::Numeric,
        GuestIndexType::Boolean => HostIndexType::Boolean,
    }
}

pub(crate) fn index_definition_in(v: GuestIndexDefinition) -> HostIndexDefinition {
    HostIndexDefinition { field_name: v.field_name, type_: index_type_in(v.type_) }
}

pub(crate) fn collection_schema_in(v: GuestCollectionSchema) -> HostCollectionSchema {
    HostCollectionSchema {
        name: v.name,
        indexes: v.indexes.into_iter().map(index_definition_in).collect(),
    }
}

pub(crate) fn record_write_value_in(v: GuestRecordWriteValue) -> HostRecordWriteValue {
    HostRecordWriteValue { id: v.id, payload: v.payload }
}

pub(crate) fn patch_mutation_in(v: GuestPatchMutation) -> HostPatchMutation {
    HostPatchMutation { id: v.id, patch_json: v.patch_json }
}

pub(crate) fn mutation_in(v: GuestMutation) -> HostMutation {
    match v {
        GuestMutation::Put(w) => HostMutation::Put(record_write_value_in(w)),
        GuestMutation::Patch(p) => HostMutation::Patch(patch_mutation_in(p)),
        GuestMutation::Delete(id) => HostMutation::Delete(id),
    }
}

pub(crate) fn query_options_in(v: GuestQueryOptions) -> HostQueryOptions {
    HostQueryOptions { filter: v.filter, limit: v.limit, cursor: v.cursor }
}

pub(crate) fn sql_value_in(v: GuestSqlValue) -> HostSqlValue {
    match v {
        GuestSqlValue::Text(s) => HostSqlValue::Text(s),
        GuestSqlValue::Integer(i) => HostSqlValue::Integer(i),
        GuestSqlValue::Real(f) => HostSqlValue::Real(f),
        GuestSqlValue::Boolean(b) => HostSqlValue::Boolean(b),
        GuestSqlValue::Null => HostSqlValue::Null,
    }
}

// ---- data-layer: out (host -> guest) ----

pub(crate) fn record_read_value_out(v: HostRecordReadValue) -> GuestRecordReadValue {
    GuestRecordReadValue {
        id: v.id,
        payload: v.payload,
        creator_id: v.creator_id,
        created_at: v.created_at,
        updated_at: v.updated_at,
    }
}

pub(crate) fn query_result_out(v: HostQueryResult) -> GuestQueryResult {
    GuestQueryResult {
        records: v.records.into_iter().map(record_read_value_out).collect(),
        next_cursor: v.next_cursor,
    }
}

pub(crate) fn raw_query_result_out(v: HostRawQueryResult) -> GuestRawQueryResult {
    GuestRawQueryResult {
        columns: v.columns,
        rows: v.rows.into_iter().map(|row| row.into_iter().map(sql_value_out).collect()).collect(),
    }
}

fn sql_value_out(v: HostSqlValue) -> GuestSqlValue {
    match v {
        HostSqlValue::Text(s) => GuestSqlValue::Text(s),
        HostSqlValue::Integer(i) => GuestSqlValue::Integer(i),
        HostSqlValue::Real(f) => GuestSqlValue::Real(f),
        HostSqlValue::Boolean(b) => GuestSqlValue::Boolean(b),
        HostSqlValue::Null => GuestSqlValue::Null,
    }
}

pub(crate) fn data_layer_error_out(v: HostDataLayerError) -> GuestDataLayerError {
    match v {
        HostDataLayerError::PermissionDenied => GuestDataLayerError::PermissionDenied,
        HostDataLayerError::CollectionNotFound => GuestDataLayerError::CollectionNotFound,
        HostDataLayerError::SchemaViolation(s) => GuestDataLayerError::SchemaViolation(s),
        HostDataLayerError::QuotaExceeded => GuestDataLayerError::QuotaExceeded,
        HostDataLayerError::Internal(s) => GuestDataLayerError::Internal(s),
    }
}

// ---- blob-store: out (host -> guest) ----

pub(crate) fn blob_error_out(v: HostBlobError) -> GuestBlobError {
    match v {
        HostBlobError::NotFound => GuestBlobError::NotFound,
        HostBlobError::QuotaExceeded => GuestBlobError::QuotaExceeded,
        HostBlobError::Internal(s) => GuestBlobError::Internal(s),
    }
}

// ---- messaging: out (host -> guest) ----

pub(crate) fn msg_error_out(v: HostMessagingError) -> GuestMessagingError {
    match v {
        HostMessagingError::PermissionDenied => GuestMessagingError::PermissionDenied,
        HostMessagingError::Internal(s) => GuestMessagingError::Internal(s),
    }
}

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

// ---- proxy: in (guest -> host) ----

pub(crate) fn call_target_in(v: GuestCallTarget) -> HostCallTarget {
    match v {
        GuestCallTarget::Service(s) => HostCallTarget::Service(s),
        GuestCallTarget::Dependency(s) => HostCallTarget::Dependency(s),
    }
}

pub(crate) fn call_options_in(v: GuestCallOptions) -> HostCallOptions {
    HostCallOptions {
        protocol: v.protocol,
        idempotent: v.idempotent,
        timeout_ms: v.timeout_ms,
        routing_key: v.routing_key,
        idempotency_key: v.idempotency_key,
    }
}

// ---- proxy: out (host -> guest) ----

pub(crate) fn callee_error_out(v: HostCalleeError) -> GuestCalleeError {
    GuestCalleeError { code: v.code, message: v.message, data: v.data }
}

pub(crate) fn proxy_error_out(v: HostProxyError) -> GuestProxyError {
    match v {
        HostProxyError::ServiceNotFound(s) => GuestProxyError::ServiceNotFound(s),
        HostProxyError::DependencyNotBound(s) => GuestProxyError::DependencyNotBound(s),
        HostProxyError::UnsupportedProtocol(s) => GuestProxyError::UnsupportedProtocol(s),
        HostProxyError::UnsupportedTarget(s) => GuestProxyError::UnsupportedTarget(s),
        HostProxyError::PermissionDenied(s) => GuestProxyError::PermissionDenied(s),
        HostProxyError::Transport(s) => GuestProxyError::Transport(s),
        HostProxyError::TimedOut => GuestProxyError::TimedOut,
        HostProxyError::Callee(e) => GuestProxyError::Callee(callee_error_out(e)),
        HostProxyError::Internal(s) => GuestProxyError::Internal(s),
    }
}

// ---- app-config: out (host -> guest) ----

pub(crate) fn config_error_out(v: HostConfigError) -> GuestConfigError {
    match v {
        HostConfigError::Internal(s) => GuestConfigError::Internal(s),
    }
}

// ---- vault: out (host -> guest) ----

pub(crate) fn vault_error_out(v: HostVaultError) -> GuestVaultError {
    match v {
        HostVaultError::NotFound => GuestVaultError::NotFound,
        HostVaultError::PermissionDenied => GuestVaultError::PermissionDenied,
        HostVaultError::Internal(s) => GuestVaultError::Internal(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_definition_round_trips() {
        let g =
            GuestIndexDefinition { field_name: "seq".to_string(), type_: GuestIndexType::Numeric };
        let h = index_definition_in(g);
        assert_eq!(h.field_name, "seq");
        assert!(matches!(h.type_, HostIndexType::Numeric));
    }

    #[test]
    fn collection_schema_round_trips() {
        let g = GuestCollectionSchema {
            name: "messages".to_string(),
            indexes: vec![GuestIndexDefinition {
                field_name: "seq".to_string(),
                type_: GuestIndexType::String,
            }],
        };
        let h = collection_schema_in(g);
        assert_eq!(h.name, "messages");
        assert_eq!(h.indexes.len(), 1);
        assert_eq!(h.indexes[0].field_name, "seq");
    }

    #[test]
    fn record_write_value_round_trips() {
        let g = GuestRecordWriteValue { id: "m1".to_string(), payload: vec![1, 2, 3] };
        let h = record_write_value_in(g);
        assert_eq!(h.id, "m1");
        assert_eq!(h.payload, vec![1, 2, 3]);
    }

    #[test]
    fn patch_mutation_round_trips() {
        let g = GuestPatchMutation { id: "m1".to_string(), patch_json: vec![4, 5] };
        let h = patch_mutation_in(g);
        assert_eq!(h.id, "m1");
        assert_eq!(h.patch_json, vec![4, 5]);
    }

    #[test]
    fn mutation_round_trips_all_variants() {
        let put = mutation_in(GuestMutation::Put(GuestRecordWriteValue {
            id: "a".to_string(),
            payload: vec![1],
        }));
        assert!(matches!(put, HostMutation::Put(_)));

        let patch = mutation_in(GuestMutation::Patch(GuestPatchMutation {
            id: "b".to_string(),
            patch_json: vec![2],
        }));
        assert!(matches!(patch, HostMutation::Patch(_)));

        let delete = mutation_in(GuestMutation::Delete("c".to_string()));
        assert!(matches!(delete, HostMutation::Delete(id) if id == "c"));
    }

    #[test]
    fn query_options_round_trips() {
        let g = GuestQueryOptions {
            filter: Some("x = 1".to_string()),
            limit: Some(10),
            cursor: Some("cur".to_string()),
        };
        let h = query_options_in(g);
        assert_eq!(h.filter, Some("x = 1".to_string()));
        assert_eq!(h.limit, Some(10));
        assert_eq!(h.cursor, Some("cur".to_string()));
    }

    #[test]
    fn sql_value_round_trips_all_variants() {
        assert!(
            matches!(sql_value_in(GuestSqlValue::Text("t".to_string())), HostSqlValue::Text(s) if s == "t")
        );
        assert!(matches!(sql_value_in(GuestSqlValue::Integer(7)), HostSqlValue::Integer(7)));
        assert!(
            matches!(sql_value_in(GuestSqlValue::Real(1.5)), HostSqlValue::Real(f) if f == 1.5)
        );
        assert!(matches!(sql_value_in(GuestSqlValue::Boolean(true)), HostSqlValue::Boolean(true)));
        assert!(matches!(sql_value_in(GuestSqlValue::Null), HostSqlValue::Null));

        assert!(
            matches!(sql_value_out(HostSqlValue::Text("t".to_string())), GuestSqlValue::Text(s) if s == "t")
        );
        assert!(matches!(sql_value_out(HostSqlValue::Integer(7)), GuestSqlValue::Integer(7)));
        assert!(
            matches!(sql_value_out(HostSqlValue::Real(1.5)), GuestSqlValue::Real(f) if f == 1.5)
        );
        assert!(matches!(sql_value_out(HostSqlValue::Boolean(true)), GuestSqlValue::Boolean(true)));
        assert!(matches!(sql_value_out(HostSqlValue::Null), GuestSqlValue::Null));
    }

    #[test]
    fn record_read_value_round_trips() {
        let h = HostRecordReadValue {
            id: "m1".to_string(),
            payload: vec![9],
            creator_id: "did:key:z1".to_string(),
            created_at: 100,
            updated_at: 200,
        };
        let g = record_read_value_out(h);
        assert_eq!(g.id, "m1");
        assert_eq!(g.payload, vec![9]);
        assert_eq!(g.creator_id, "did:key:z1");
        assert_eq!(g.created_at, 100);
        assert_eq!(g.updated_at, 200);
    }

    #[test]
    fn query_result_round_trips() {
        let h = HostQueryResult {
            records: vec![HostRecordReadValue {
                id: "m1".to_string(),
                payload: vec![1],
                creator_id: "c".to_string(),
                created_at: 1,
                updated_at: 2,
            }],
            next_cursor: Some("cur".to_string()),
        };
        let g = query_result_out(h);
        assert_eq!(g.records.len(), 1);
        assert_eq!(g.next_cursor, Some("cur".to_string()));
    }

    #[test]
    fn raw_query_result_round_trips() {
        let h = HostRawQueryResult {
            columns: vec!["a".to_string(), "b".to_string()],
            rows: vec![vec![HostSqlValue::Integer(1), HostSqlValue::Text("x".to_string())]],
        };
        let g = raw_query_result_out(h);
        assert_eq!(g.columns, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(g.rows.len(), 1);
        assert_eq!(g.rows[0].len(), 2);
    }

    #[test]
    fn data_layer_error_round_trips_all_variants() {
        assert!(matches!(
            data_layer_error_out(HostDataLayerError::PermissionDenied),
            GuestDataLayerError::PermissionDenied
        ));
        assert!(matches!(
            data_layer_error_out(HostDataLayerError::CollectionNotFound),
            GuestDataLayerError::CollectionNotFound
        ));
        assert!(matches!(
            data_layer_error_out(HostDataLayerError::SchemaViolation("x".to_string())),
            GuestDataLayerError::SchemaViolation(s) if s == "x"
        ));
        assert!(matches!(
            data_layer_error_out(HostDataLayerError::QuotaExceeded),
            GuestDataLayerError::QuotaExceeded
        ));
        assert!(matches!(
            data_layer_error_out(HostDataLayerError::Internal("y".to_string())),
            GuestDataLayerError::Internal(s) if s == "y"
        ));
    }

    #[test]
    fn blob_error_round_trips_all_variants() {
        assert!(matches!(blob_error_out(HostBlobError::NotFound), GuestBlobError::NotFound));
        assert!(matches!(
            blob_error_out(HostBlobError::QuotaExceeded),
            GuestBlobError::QuotaExceeded
        ));
        assert!(matches!(
            blob_error_out(HostBlobError::Internal("z".to_string())),
            GuestBlobError::Internal(s) if s == "z"
        ));
    }

    #[test]
    fn messaging_error_round_trips_all_variants() {
        assert!(matches!(
            msg_error_out(HostMessagingError::PermissionDenied),
            GuestMessagingError::PermissionDenied
        ));
        assert!(matches!(
            msg_error_out(HostMessagingError::Internal("w".to_string())),
            GuestMessagingError::Internal(s) if s == "w"
        ));
    }

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

    #[test]
    fn call_target_and_options_round_trip() {
        let t = call_target_in(GuestCallTarget::Service("svc1".to_string()));
        assert!(matches!(t, HostCallTarget::Service(s) if s == "svc1"));

        let t2 = call_target_in(GuestCallTarget::Dependency("dep1".to_string()));
        assert!(matches!(t2, HostCallTarget::Dependency(s) if s == "dep1"));

        let opt = call_options_in(GuestCallOptions {
            protocol: Some("json-rpc/v1".to_string()),
            idempotent: true,
            timeout_ms: Some(5000),
            routing_key: Some("rk".to_string()),
            idempotency_key: Some("k1".to_string()),
        });
        assert_eq!(opt.protocol, Some("json-rpc/v1".to_string()));
        assert!(opt.idempotent);
        assert_eq!(opt.timeout_ms, Some(5000));
        assert_eq!(opt.routing_key, Some("rk".to_string()));
        assert_eq!(opt.idempotency_key, Some("k1".to_string()));
    }

    #[test]
    fn proxy_error_round_trips_all_variants() {
        assert!(matches!(
            proxy_error_out(HostProxyError::ServiceNotFound("s".to_string())),
            GuestProxyError::ServiceNotFound(s) if s == "s"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::DependencyNotBound("d".to_string())),
            GuestProxyError::DependencyNotBound(s) if s == "d"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::UnsupportedProtocol("p".to_string())),
            GuestProxyError::UnsupportedProtocol(s) if s == "p"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::UnsupportedTarget("t".to_string())),
            GuestProxyError::UnsupportedTarget(s) if s == "t"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::PermissionDenied("u".to_string())),
            GuestProxyError::PermissionDenied(s) if s == "u"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::Transport("tr".to_string())),
            GuestProxyError::Transport(s) if s == "tr"
        ));
        assert!(matches!(proxy_error_out(HostProxyError::TimedOut), GuestProxyError::TimedOut));
        assert!(matches!(
            proxy_error_out(HostProxyError::Callee(HostCalleeError {
                code: 1,
                message: "m".to_string(),
                data: None,
            })),
            GuestProxyError::Callee(e) if e.code == 1 && e.message == "m" && e.data.is_none()
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::Internal("i".to_string())),
            GuestProxyError::Internal(s) if s == "i"
        ));
    }

    #[test]
    fn config_error_round_trips_all_variants() {
        assert!(matches!(
            config_error_out(HostConfigError::Internal("i".to_string())),
            GuestConfigError::Internal(s) if s == "i"
        ));
    }

    #[test]
    fn vault_error_round_trips_all_variants() {
        assert!(matches!(vault_error_out(HostVaultError::NotFound), GuestVaultError::NotFound));
        assert!(matches!(
            vault_error_out(HostVaultError::PermissionDenied),
            GuestVaultError::PermissionDenied
        ));
        assert!(matches!(
            vault_error_out(HostVaultError::Internal("i".to_string())),
            GuestVaultError::Internal(s) if s == "i"
        ));
    }
}
