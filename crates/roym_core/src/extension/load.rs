//! The rules a manifest must meet to load.

use std::collections::{HashMap, HashSet};

use super::{Hook, MAX_STEP_TYPE_LEN, Manifest, ManifestKind, ids, schema};
use crate::{
    deal::{MAX_EXTENSION_ID_LEN, ResourceKind},
    listing, services,
};

/// Why a manifest does not load.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// The JSON does not fit the manifest format.
    #[error("manifest does not fit the format: {0}")]
    Format(String),
    /// A rule of the format is broken. `id` names the extension.
    #[error("extension '{id}': {rule}")]
    Rule { id: String, rule: String },
}

impl ManifestError {
    pub(super) fn rule(id: &str, rule: impl Into<String>) -> Self {
        Self::Rule { id: id.to_string(), rule: rule.into() }
    }
}

pub(super) fn check_manifest(m: &Manifest) -> Result<(), ManifestError> {
    let fail = |rule: String| ManifestError::rule(&m.id, rule);
    if !listing::valid_token(&m.id, MAX_EXTENSION_ID_LEN) {
        return Err(fail(format!("id must be [a-z0-9-]{{1,{MAX_EXTENSION_ID_LEN}}}")));
    }
    if m.version == 0 {
        return Err(fail("version must be at least 1".to_string()));
    }
    match m.kind {
        ManifestKind::Workflow => check_workflow(m).map_err(fail),
        ManifestKind::Insight => check_insight(m).map_err(fail),
    }
}

fn check_insight(m: &Manifest) -> Result<(), String> {
    let has_workflow_part = !m.resource_kinds.is_empty()
        || !m.hooks.is_empty()
        || !m.details.is_empty()
        || !m.steps.is_empty();
    if has_workflow_part {
        return Err("an insight has only reads".to_string());
    }
    if m.reads.is_empty() {
        return Err("an insight must say what it reads".to_string());
    }
    if let Some(read) = m.reads.iter().find(|r| !listing::valid_token(r, MAX_EXTENSION_ID_LEN)) {
        return Err(format!("read '{read}' is not a code"));
    }
    Ok(())
}

fn check_workflow(m: &Manifest) -> Result<(), String> {
    if !m.reads.is_empty() {
        return Err("only an insight has reads".to_string());
    }
    check_hooks(m)?;
    check_schemas(m)?;
    check_steps(m)
}

fn check_hooks(m: &Manifest) -> Result<(), String> {
    let distinct: HashSet<_> = m.hooks.iter().collect();
    if distinct.len() != m.hooks.len() {
        return Err("hooks lists a hook twice".to_string());
    }
    let held = [Hook::Reserve, Hook::Confirm, Hook::Release]
        .iter()
        .filter(|h| distinct.contains(h))
        .count();
    if held != 0 && held != 3 {
        return Err("hooks must have all of reserve, confirm and release, or none".to_string());
    }
    if distinct.contains(&Hook::Prepare) && held == 0 {
        return Err("hooks: prepare needs reserve, confirm and release".to_string());
    }
    check_hook_service(m)?;
    if m.resource_kinds.contains(&ResourceKind::ServiceInSlot) && m.hooks.is_empty() {
        return Err("service-in-slot needs the reserve, confirm and release hooks".to_string());
    }
    Ok(())
}

/// `service` is there exactly when there are hooks, and it names a Roym
/// service that can answer them.
fn check_hook_service(m: &Manifest) -> Result<(), String> {
    match (&m.service, m.hooks.is_empty()) {
        (None, true) => Ok(()),
        (None, false) => Err("hooks need a service that answers them".to_string()),
        (Some(_), true) => Err("a service is only for an extension with hooks".to_string()),
        (Some(name), false) => match services::by_name(name) {
            Some(service) if service != services::WEB => Ok(()),
            _ => Err(format!("service '{name}' is not a Roym service that can answer hooks")),
        },
    }
}

/// Splits `<id>/<name>` and checks that `<name>` is a code.
fn name_after_prefix<'a>(ext_id: &str, full: &'a str) -> Result<&'a str, String> {
    let name = full
        .strip_prefix(ext_id)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| format!("'{full}' must start with '{ext_id}/'"))?;
    if !listing::valid_token(name, MAX_EXTENSION_ID_LEN) {
        return Err(format!("the name in '{full}' must be [a-z0-9-]{{1,{MAX_EXTENSION_ID_LEN}}}"));
    }
    Ok(name)
}

fn check_schemas(m: &Manifest) -> Result<(), String> {
    let mut names = HashSet::new();
    for s in &m.details {
        let (base, version) = s
            .schema
            .rsplit_once('@')
            .ok_or_else(|| format!("schema '{}' must end with @<version>", s.schema))?;
        if version.parse::<u32>().ok().filter(|v| *v >= 1).is_none() {
            return Err(format!("schema '{}' must end with @<version>", s.schema));
        }
        name_after_prefix(&m.id, base)?;
        if !names.insert(s.schema.as_str()) {
            return Err(format!("schema '{}' is listed twice", s.schema));
        }
        check_field_list(&s.fields)?;
    }
    Ok(())
}

fn check_steps(m: &Manifest) -> Result<(), String> {
    let mut types = HashSet::new();
    for step in &m.steps {
        if step.step_type.len() > MAX_STEP_TYPE_LEN {
            return Err(format!("step type '{}' is too long", step.step_type));
        }
        name_after_prefix(&m.id, &step.step_type)?;
        if !types.insert(step.step_type.as_str()) {
            return Err(format!("step type '{}' is listed twice", step.step_type));
        }
        check_field_list(&step.fields)?;
    }
    Ok(())
}

fn check_field_list(fields: &[super::Field]) -> Result<(), String> {
    let mut names = HashSet::new();
    for field in fields {
        schema::check_definition(field)?;
        if !names.insert(field.name.as_str()) {
            return Err(format!("field '{}' is listed twice", field.name));
        }
    }
    Ok(())
}

/// Two different places must never give one message id. The same place in two
/// versions of a schema or of an extension gives the same id on purpose.
pub(super) fn check_message_ids(manifests: &[Manifest]) -> Result<(), ManifestError> {
    let mut seen: HashMap<String, String> = HashMap::new();
    for m in manifests {
        for (id, place) in ids::manifest_message_sources(m) {
            match seen.get(&id) {
                Some(first) if *first != place => {
                    return Err(ManifestError::rule(
                        &m.id,
                        format!("message id '{id}' is made twice ({first} and {place})"),
                    ));
                }
                Some(_) => {}
                None => {
                    seen.insert(id, place);
                }
            }
        }
    }
    Ok(())
}
