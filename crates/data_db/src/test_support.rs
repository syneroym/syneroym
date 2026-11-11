use syneroym_wit_interfaces::host::syneroym::data_layer::store::{
    CollectionSchema, IndexDefinition,
};

pub(crate) fn plain_schema(name: &str) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes: vec![], text_fields: vec![] }
}

pub(crate) fn schema_with(
    name: &str,
    indexes: Vec<IndexDefinition>,
    text_fields: Vec<String>,
) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes, text_fields }
}
