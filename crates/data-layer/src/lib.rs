pub mod registry_store;

/// Re-export generated WIT types for guest compatibility and ease of use.
pub use syneroym_bindings::data_layer::syneroym::data_layer::store as wit_store;
pub use syneroym_bindings::vault::syneroym::vault::vault as wit_vault;

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
mod tests {
    use super::*;

    #[test]
    fn test_data_layer_service_instantiation() {
        let service = DataLayerService::new();
        let _default = DataLayerService::default();
        assert!(format!("{:?}", service).contains("DataLayerService"));
    }

    #[test]
    fn test_serde_derives_on_wit_types() {
        fn assert_serde<T: serde::Serialize + for<'de> serde::Deserialize<'de>>() {}

        assert_serde::<wit_store::RecordWriteValue>();
        assert_serde::<wit_store::RecordReadValue>();
        assert_serde::<wit_store::CollectionSchema>();
        assert_serde::<wit_store::IndexDefinition>();
        assert_serde::<wit_store::IndexType>();
        assert_serde::<wit_store::QueryOptions>();
        assert_serde::<wit_store::QueryResult>();
        assert_serde::<wit_store::PatchMutation>();
        assert_serde::<wit_store::Mutation>();
        assert_serde::<wit_store::DataLayerError>();
    }
}
