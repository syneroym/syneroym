pub mod errors;
pub mod filter;
pub mod registry_store;
pub mod sqlite;
pub mod traits;

pub use sqlite::SqliteStorageProvider;
/// Re-export the wasmtime-host-generated WIT types: this crate runs only on
/// the host (never compiled to a WASM guest), so it speaks the same types
/// `HostState`'s `Host` trait impl uses, with no conversion layer between
/// them.
pub use syneroym_bindings::host::syneroym::data_layer::store as host_store;
pub use syneroym_bindings::vault::syneroym::vault::vault as wit_vault;
pub use traits::{ServiceStore, StorageProvider};

/// Placeholder service for the data layer, to be implemented in subsequent
/// slices.
#[derive(Debug, Clone)]
pub struct DataLayerService {
    // DB logic to be added in Slice 2A and 3A
}

impl DataLayerService {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for DataLayerService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests_crud;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_data_layer_service_instantiation() {
        let service = DataLayerService::new();
        let _default = DataLayerService::default();
        assert!(format!("{:?}", service).contains("DataLayerService"));
    }

    #[test]
    fn test_serde_derives_on_host_store_types() {
        // Test RecordWriteValue
        let val =
            host_store::RecordWriteValue { id: "test-id".to_string(), payload: vec![1, 2, 3] };
        let serialized = serde_json::to_string(&val).unwrap();
        let deserialized: host_store::RecordWriteValue = serde_json::from_str(&serialized).unwrap();
        assert_eq!(val.id, deserialized.id);
        assert_eq!(val.payload, deserialized.payload);

        // Test RecordReadValue
        let val = host_store::RecordReadValue {
            id: "test-id".to_string(),
            payload: vec![1, 2, 3],
            creator_id: "creator".to_string(),
            created_at: 100,
            updated_at: 200,
        };
        let serialized = serde_json::to_string(&val).unwrap();
        let deserialized: host_store::RecordReadValue = serde_json::from_str(&serialized).unwrap();
        assert_eq!(val.id, deserialized.id);
        assert_eq!(val.payload, deserialized.payload);
        assert_eq!(val.creator_id, deserialized.creator_id);
        assert_eq!(val.created_at, deserialized.created_at);
        assert_eq!(val.updated_at, deserialized.updated_at);

        // Test DataLayerError (Internal)
        let val = host_store::DataLayerError::Internal("test error".to_string());
        let serialized = serde_json::to_string(&val).unwrap();
        let deserialized: host_store::DataLayerError = serde_json::from_str(&serialized).unwrap();
        match (val, deserialized) {
            (
                host_store::DataLayerError::Internal(e1),
                host_store::DataLayerError::Internal(e2),
            ) => {
                assert_eq!(e1, e2);
            }
            _ => panic!("Expected DataLayerError::Internal"),
        }

        // Test DataLayerError (PermissionDenied)
        let val = host_store::DataLayerError::PermissionDenied;
        let serialized = serde_json::to_string(&val).unwrap();
        let deserialized: host_store::DataLayerError = serde_json::from_str(&serialized).unwrap();
        assert!(matches!(deserialized, host_store::DataLayerError::PermissionDenied));

        // Test Mutation (Put)
        let val = host_store::Mutation::Put(host_store::RecordWriteValue {
            id: "test-id".to_string(),
            payload: vec![1, 2, 3],
        });
        let serialized = serde_json::to_string(&val).unwrap();
        let deserialized: host_store::Mutation = serde_json::from_str(&serialized).unwrap();
        match (val, deserialized) {
            (host_store::Mutation::Put(w1), host_store::Mutation::Put(w2)) => {
                assert_eq!(w1.id, w2.id);
                assert_eq!(w1.payload, w2.payload);
            }
            _ => panic!("Expected Mutation::Put"),
        }
    }
}
