//! Guest-vocabulary <-> host-vocabulary conversions. Both sides are
//! generated from the same `.wit`, so every one is a field-for-field copy;
//! this module exists so that copy happens in exactly one place.

use syneroym_app_host::types::{
    blob_store::BlobError as GuestBlobError,
    data_layer::{
        CollectionSchema as GuestCollectionSchema, DataLayerError as GuestDataLayerError,
        IndexDefinition as GuestIndexDefinition, IndexType as GuestIndexType,
        Mutation as GuestMutation, PatchMutation as GuestPatchMutation,
        QueryOptions as GuestQueryOptions, QueryResult as GuestQueryResult,
        RawQueryResult as GuestRawQueryResult, RecordReadValue as GuestRecordReadValue,
        RecordWriteValue as GuestRecordWriteValue, SqlValue as GuestSqlValue,
    },
    messaging::MessagingError as GuestMessagingError,
};
use syneroym_wit_interfaces::host::syneroym::{
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
}
