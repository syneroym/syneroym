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
    signing::{
        Principal as GuestPrincipal, RecordDraft as GuestRecordDraft,
        SigningError as GuestSigningError, SigningIdentity as GuestSigningIdentity,
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
    signing_host::syneroym::signing::signing::{
        Principal as HostPrincipal, RecordDraft as HostRecordDraft,
        SigningError as HostSigningError, SigningIdentity as HostSigningIdentity,
    },
};

mod conversation;
mod data_layer;
mod proxy;
mod signing;
mod simple_errors;

pub(crate) use conversation::*;
pub(crate) use data_layer::*;
pub(crate) use proxy::*;
pub(crate) use signing::*;
pub(crate) use simple_errors::*;
