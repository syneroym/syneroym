#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Data-layer test guest component
//!
//! Exercises the `syneroym:data-layer/store` host interface end-to-end for
//! Slice 3A integration tests: schema lifecycle hooks (`init`/`migrate`),
//! CRUD, and host-injected `creator-id` verification.

use bindings::{
    Guest,
    exports::syneroym_test::data_layer_test::test_driver::Guest as TestDriverGuest,
    syneroym::data_layer::store::{
        self, CollectionSchema, IndexDefinition, IndexType, QueryOptions, RecordWriteValue,
    },
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "data-layer-test",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
        },
    });

    use super::DataLayerTestComponent;
    export!(DataLayerTestComponent);
}

struct DataLayerTestComponent;

impl Guest for DataLayerTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&CollectionSchema {
            name: "profiles".to_string(),
            indexes: vec![IndexDefinition {
                field_name: "age".to_string(),
                type_: IndexType::Numeric,
            }],
        })
        .map_err(|e| format!("{e:?}"))
    }

    fn migrate() -> Result<(), String> {
        store::execute_ddl("ALTER TABLE profiles ADD COLUMN nickname TEXT")
            .map_err(|e| format!("{e:?}"))
    }
}

impl TestDriverGuest for DataLayerTestComponent {
    fn run_crud_scenario(count: u32) -> Result<String, String> {
        for i in 0..count {
            let payload = format!(r#"{{"age": {i}}}"#).into_bytes();
            store::put("profiles", &RecordWriteValue { id: format!("p{i}"), payload })
                .map_err(|e| format!("{e:?}"))?;
        }
        let result = store::query(
            "profiles",
            &QueryOptions { filter: None, limit: Some(count), cursor: None },
        )
        .map_err(|e| format!("{e:?}"))?;
        Ok(result.records.len().to_string())
    }

    fn get_creator_id(id: String) -> Result<String, String> {
        let record = store::get("profiles", &id).map_err(|e| format!("{e:?}"))?;
        record.map(|r| r.creator_id).ok_or_else(|| "record not found".to_string())
    }
}
