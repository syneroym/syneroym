//! The type vocabulary both builds share: the wit-bindgen *guest* types,
//! which compile for `wasm32-wasip2` and for the host alike (the host build
//! links them against stub imports it never calls).

pub mod data_layer {
    pub use syneroym_wit_interfaces::data_layer::syneroym::data_layer::store::{
        CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
        QueryOptions, QueryResult, RawQueryResult, RecordReadValue, RecordWriteValue, SqlValue,
    };
}

pub mod blob_store {
    pub use syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store::BlobError;
}

pub mod messaging {
    pub use syneroym_wit_interfaces::messaging::syneroym::messaging::host_api::MessagingError;
}

pub mod conversation {
    pub use syneroym_wit_interfaces::conversation::syneroym::conversation::conversation::{
        ConversationError, ConversationKind, ConversationSummary, DeliveryState, HistoryPage,
        MembershipEvent, Message,
    };
}

pub mod proxy {
    pub use syneroym_wit_interfaces::proxy::syneroym::proxy::proxy::{
        CallOptions, CallTarget, CalleeError, ProxyError,
    };
}

pub mod signing {
    pub use syneroym_wit_interfaces::signing::syneroym::signing::signing::{
        Principal, RecordDraft, SigningError, SigningIdentity,
    };
}

pub mod app_config {
    pub use syneroym_wit_interfaces::app_config::syneroym::app_config::app_config::ConfigError;
}

pub mod vault {
    pub use syneroym_wit_interfaces::vault::syneroym::vault::vault::VaultError;
}

/// Mirrors `syneroym:http/incoming-handler@0.1.0`'s records field for
/// field, **in the same order** -- the dynamic `Val::Record` the WASM
/// build marshals from these must match the declared field order.
///
/// Hand-written rather than WIT-generated, unlike every other module here:
/// these records are declared inside an interface a component *exports*,
/// so there is no import-direction interface to generate a shared guest
/// view from, and creating one would add a types-only import instance no
/// linker in the tree registers.
pub mod http {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CallerAuth {
        Delegated,
        Ucan,
        SelfAsserted,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CallerIdentity {
        pub did: String,
        pub auth: CallerAuth,
        pub app_instance: Option<String>,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct HttpRequest {
        pub method: String,
        pub path: String,
        pub query: String,
        pub route: String,
        pub path_params: Vec<(String, String)>,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
        pub caller: Option<CallerIdentity>,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct HttpResponse {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    /// Mirrors `syneroym:http/websocket-types@0.1.0`'s `frame-kind`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FrameKind {
        Text,
        Binary,
    }
}
