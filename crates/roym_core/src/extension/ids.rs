//! Message ids derived from a manifest. The Hub derives the same ids with the
//! same rules, so a manifest never has to name them.
//!
//! The schema version is not part of an id. A new schema version that changes
//! what a field means must give the field a new name.

use super::Manifest;

fn dashed(s: &str) -> String {
    s.replace('_', "-")
}

/// `scheduling/slot@1` becomes `slot` (for the extension `scheduling`).
fn schema_short<'a>(ext_id: &str, schema: &'a str) -> &'a str {
    let name = schema.strip_prefix(ext_id).and_then(|r| r.strip_prefix('/')).unwrap_or(schema);
    name.split('@').next().unwrap_or(name)
}

/// The label of a field of a `details` schema.
pub fn field_label_id(ext_id: &str, schema: &str, field: &str) -> String {
    format!("ext-{}-{}-{}", dashed(ext_id), dashed(schema_short(ext_id, schema)), dashed(field))
}

/// The label of one choice of a field of a `details` schema.
pub fn choice_label_id(ext_id: &str, schema: &str, field: &str, choice: &str) -> String {
    format!("{}-{}", field_label_id(ext_id, schema, field), dashed(choice))
}

/// The name of a step type.
pub fn step_name_id(ext_id: &str, step_type: &str) -> String {
    let name =
        step_type.strip_prefix(ext_id).and_then(|r| r.strip_prefix('/')).unwrap_or(step_type);
    format!("ext-{}-step-{}", dashed(ext_id), dashed(name))
}

/// Every id that this manifest implies, in manifest order.
pub fn manifest_message_ids(manifest: &Manifest) -> Vec<String> {
    let mut ids = Vec::new();
    for schema in &manifest.details {
        for field in &schema.fields {
            ids.push(field_label_id(&manifest.id, &schema.schema, &field.name));
            for choice in field.choices.iter().flatten() {
                ids.push(choice_label_id(&manifest.id, &schema.schema, &field.name, choice));
            }
        }
    }
    for step in &manifest.steps {
        ids.push(step_name_id(&manifest.id, &step.step_type));
    }
    ids
}
