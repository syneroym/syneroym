//! Conversions between the two Rust type sets generated from
//! `syneroym:data-layer/store`: the `wasmtime::component::bindgen!` output
//! used by `HostState`'s `Host` trait impl (aliased `host_store` below) and
//! the `wit_bindgen::generate!` output already used throughout
//! `syneroym-data-layer` (`wit_store`). These are structurally identical but
//! distinct Rust types produced by different codegen backends -- calls
//! crossing this boundary need explicit field-by-field conversion. Neither
//! type is local to this crate, so `From` impls are not an option here
//! (orphan rules); these are plain conversion functions instead.

use syneroym_bindings::host::syneroym::data_layer::store as host_store;
use syneroym_data_layer::wit_store;

pub fn index_type_to_wit(ty: host_store::IndexType) -> wit_store::IndexType {
    match ty {
        host_store::IndexType::String => wit_store::IndexType::String,
        host_store::IndexType::Numeric => wit_store::IndexType::Numeric,
        host_store::IndexType::Boolean => wit_store::IndexType::Boolean,
    }
}

pub fn collection_schema_to_wit(
    schema: &host_store::CollectionSchema,
) -> wit_store::CollectionSchema {
    wit_store::CollectionSchema {
        name: schema.name.clone(),
        indexes: schema
            .indexes
            .iter()
            .map(|idx| wit_store::IndexDefinition {
                field_name: idx.field_name.clone(),
                type_: index_type_to_wit(idx.type_),
            })
            .collect(),
    }
}

pub fn record_write_value_to_wit(
    value: &host_store::RecordWriteValue,
) -> wit_store::RecordWriteValue {
    wit_store::RecordWriteValue { id: value.id.clone(), payload: value.payload.clone() }
}

pub fn query_options_to_wit(opts: &host_store::QueryOptions) -> wit_store::QueryOptions {
    wit_store::QueryOptions {
        filter: opts.filter.clone(),
        limit: opts.limit,
        cursor: opts.cursor.clone(),
    }
}

pub fn mutation_to_wit(mutation: &host_store::Mutation) -> wit_store::Mutation {
    match mutation {
        host_store::Mutation::Put(v) => wit_store::Mutation::Put(record_write_value_to_wit(v)),
        host_store::Mutation::Patch(p) => wit_store::Mutation::Patch(wit_store::PatchMutation {
            id: p.id.clone(),
            patch_json: p.patch_json.clone(),
        }),
        host_store::Mutation::Delete(id) => wit_store::Mutation::Delete(id.clone()),
    }
}

pub fn record_read_value_from_wit(
    value: wit_store::RecordReadValue,
) -> host_store::RecordReadValue {
    host_store::RecordReadValue {
        id: value.id,
        payload: value.payload,
        creator_id: value.creator_id,
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
}

pub fn query_result_from_wit(result: wit_store::QueryResult) -> host_store::QueryResult {
    host_store::QueryResult {
        records: result.records.into_iter().map(record_read_value_from_wit).collect(),
        next_cursor: result.next_cursor,
    }
}

pub fn data_layer_error_from_wit(err: wit_store::DataLayerError) -> host_store::DataLayerError {
    match err {
        wit_store::DataLayerError::PermissionDenied => host_store::DataLayerError::PermissionDenied,
        wit_store::DataLayerError::CollectionNotFound => {
            host_store::DataLayerError::CollectionNotFound
        }
        wit_store::DataLayerError::SchemaViolation(msg) => {
            host_store::DataLayerError::SchemaViolation(msg)
        }
        wit_store::DataLayerError::QuotaExceeded => host_store::DataLayerError::QuotaExceeded,
        wit_store::DataLayerError::Internal(msg) => host_store::DataLayerError::Internal(msg),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip_shapes_preserved() {
        let host_schema = host_store::CollectionSchema {
            name: "people".to_string(),
            indexes: vec![host_store::IndexDefinition {
                field_name: "age".to_string(),
                type_: host_store::IndexType::Numeric,
            }],
        };
        let wit_schema = collection_schema_to_wit(&host_schema);
        assert_eq!(wit_schema.name, "people");
        assert_eq!(wit_schema.indexes[0].field_name, "age");
        assert!(matches!(wit_schema.indexes[0].type_, wit_store::IndexType::Numeric));

        let wit_read = wit_store::RecordReadValue {
            id: "p1".to_string(),
            payload: vec![1, 2, 3],
            creator_id: "svc-a".to_string(),
            created_at: 100,
            updated_at: 200,
        };
        let host_read = record_read_value_from_wit(wit_read);
        assert_eq!(host_read.id, "p1");
        assert_eq!(host_read.creator_id, "svc-a");

        let err =
            data_layer_error_from_wit(wit_store::DataLayerError::SchemaViolation("x".to_string()));
        assert!(matches!(err, host_store::DataLayerError::SchemaViolation(msg) if msg == "x"));
    }
}
