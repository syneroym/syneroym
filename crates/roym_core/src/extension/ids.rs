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

/// Every id that this manifest implies, with the place that makes it, in
/// manifest order. A place is the extension, the kind of thing (field, choice
/// or step), the schema's short name, the field name and the choice. The
/// schema version is not in it, so two versions of one schema that share a
/// field name share one place.
pub fn manifest_message_sources(manifest: &Manifest) -> Vec<(String, String)> {
    let ext = manifest.id.as_str();
    let mut out = Vec::new();
    for schema in &manifest.details {
        let short = schema_short(ext, &schema.schema);
        for field in &schema.fields {
            let field_place = format!("{ext}/field/{short}/{}", field.name);
            out.push((field_label_id(ext, &schema.schema, &field.name), field_place.clone()));
            for choice in field.choices.iter().flatten() {
                out.push((
                    choice_label_id(ext, &schema.schema, &field.name, choice),
                    format!("{field_place}/{choice}"),
                ));
            }
        }
    }
    for step in &manifest.steps {
        let name = step.step_type.strip_prefix(ext).and_then(|r| r.strip_prefix('/'));
        out.push((
            step_name_id(ext, &step.step_type),
            format!("{ext}/step/{}", name.unwrap_or(&step.step_type)),
        ));
    }
    out
}

/// Every id that this manifest implies, in manifest order.
pub fn manifest_message_ids(manifest: &Manifest) -> Vec<String> {
    manifest_message_sources(manifest).into_iter().map(|(id, _)| id).collect()
}
