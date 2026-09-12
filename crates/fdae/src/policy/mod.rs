//! The FDAE policy document: typed deserialization target for the `fdae/v1`
//! schema (ADR-0017 §1) plus the semantic validation the JSON Schema can't
//! express.

use std::collections::{BTreeMap, BTreeSet};

pub mod types;

#[cfg(test)]
mod tests;

use types::MAX_PATH_HOPS;
pub use types::{
    CondOp, Condition, Definition, FieldsPolicy, Operator, Permission, Policy, PolicyError,
    Relation,
};

const FDAE_V1_SCHEMA: &str = include_str!("../../schema/fdae-v1.json");

/// Parses and fully validates an `fdae/v1` policy document: JSON-Schema
/// validation, then typed deserialization, then the semantic checks the
/// schema can't express (relation shapes, path/relation resolution,
/// `principal_column` coverage, acyclic `includes`).
pub fn parse_and_validate(doc: &str) -> Result<Policy, PolicyError> {
    let raw: serde_json::Value =
        serde_json::from_str(doc).map_err(|e| PolicyError::Schema(e.to_string()))?;

    // `expect_used` is workspace warn-level; both calls below parse this
    // crate's own embedded, unit-tested `fdae-v1.json`, never caller input,
    // so failure here means the crate itself is broken, not that a bad
    // policy was supplied.
    #[allow(clippy::expect_used)]
    let schema: serde_json::Value =
        serde_json::from_str(FDAE_V1_SCHEMA).expect("embedded fdae-v1.json is valid JSON");
    #[allow(clippy::expect_used)]
    let validator =
        jsonschema::validator_for(&schema).expect("embedded fdae-v1.json is a valid schema");
    validator.validate(&raw).map_err(|e| PolicyError::Schema(e.to_string()))?;

    let policy: Policy =
        serde_json::from_value(raw).map_err(|e| PolicyError::Schema(e.to_string()))?;

    validate_semantics(&policy)?;
    Ok(policy)
}

fn validate_semantics(policy: &Policy) -> Result<(), PolicyError> {
    if policy.version != "fdae/v1" {
        return Err(PolicyError::UnsupportedVersion(policy.version.clone()));
    }
    validate_no_collection_ambiguity(policy)?;
    for (type_name, def) in &policy.definitions {
        validate_relations(policy, type_name, def)?;
        validate_permissions(policy, type_name, def)?;
    }
    Ok(())
}

/// The compiler resolves a query's `collection` string against either a
/// definition's key or its `table`, case-insensitively -- matching SQLite's
/// own identifier resolution (`compile::find_definition`) -- taking the
/// first match. If two definitions' keys/tables collide under that same
/// case-insensitive rule, that resolution would silently pick one and mask
/// the other with no error -- reject the ambiguity at parse time instead.
fn validate_no_collection_ambiguity(policy: &Policy) -> Result<(), PolicyError> {
    let mut owners: BTreeMap<String, &str> = BTreeMap::new();
    for (type_name, def) in &policy.definitions {
        for name in [type_name.as_str(), def.table.as_str()] {
            let fold = name.to_ascii_lowercase();
            match owners.get(fold.as_str()) {
                Some(owner) if *owner != type_name.as_str() => {
                    return Err(PolicyError::Semantic(format!(
                        "'{name}' resolves ambiguously to both definition '{owner}' and \
                         '{type_name}' -- a definition's key or table must not collide with \
                         another's, even case-insensitively"
                    )));
                }
                Some(_) => {}
                None => {
                    owners.insert(fold, type_name.as_str());
                }
            }
        }
    }
    Ok(())
}

fn validate_relations(
    policy: &Policy,
    type_name: &str,
    def: &Definition,
) -> Result<(), PolicyError> {
    for (rel_name, rel) in &def.relations {
        validate_relation_shape(type_name, rel_name, rel)?;
        if rel.service.is_none() && !policy.definitions.contains_key(&rel.target) {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' relation '{rel_name}' targets unknown object type '{}'",
                rel.target
            )));
        }
    }
    Ok(())
}

fn validate_relation_shape(
    type_name: &str,
    rel_name: &str,
    rel: &Relation,
) -> Result<(), PolicyError> {
    let has_join = rel.join_column.is_some();
    let is_recursive_shape = rel.from_key.is_some() || rel.to_key.is_some();
    let is_remote = rel.service.is_some();
    // A join-based hop (local or remote) and a recursive self-join are the
    // only two shapes -- `join_column` and `service` may coexist
    // (`join_column` names the *local* column checked against the remote
    // fetch's returned id-set, exactly the role it plays for a local
    // relation's `EXISTS (SELECT ... FROM target_table)`), but neither may
    // combine with `recursive`.
    let shape_count = [has_join, is_recursive_shape].into_iter().filter(|shape| *shape).count();
    if shape_count != 1 {
        return Err(PolicyError::Semantic(format!(
            "definition '{type_name}' relation '{rel_name}' must be exactly one of: join-based \
             (join_column, optionally with service for a remote target), or recursive (from_key + \
             to_key)"
        )));
    }
    if is_recursive_shape {
        if rel.from_key.is_none() || rel.to_key.is_none() {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' relation '{rel_name}' is a recursive self-join and \
                 requires both from_key and to_key"
            )));
        }
        if !rel.recursive {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' relation '{rel_name}' declares from_key/to_key but not \
                 recursive: true"
            )));
        }
        if is_remote {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' relation '{rel_name}' is recursive; a remote \
                 (service-qualified) relation cannot also be recursive -- B3 does not support an \
                 iterative cross-node transitive closure"
            )));
        }
    } else if rel.recursive {
        return Err(PolicyError::Semantic(format!(
            "definition '{type_name}' relation '{rel_name}' sets recursive: true without \
             from_key/to_key"
        )));
    }
    if is_remote && rel.expected_asserter_did.is_none() {
        return Err(PolicyError::Semantic(format!(
            "definition '{type_name}' relation '{rel_name}' names a remote service but has no \
             expected_asserter_did -- required so a fetched RelationshipProof is verified against \
             a policy-declared trust anchor, not its own self-declared signer"
        )));
    }
    Ok(())
}

fn validate_permissions(
    policy: &Policy,
    type_name: &str,
    def: &Definition,
) -> Result<(), PolicyError> {
    for (perm_name, perm) in &def.permissions {
        for path in &perm.paths {
            validate_path(policy, type_name, perm_name, path)?;
        }
        for included in &perm.includes {
            if !def.permissions.contains_key(included) {
                return Err(PolicyError::Semantic(format!(
                    "definition '{type_name}' permission '{perm_name}' includes unknown \
                     permission '{included}'"
                )));
            }
        }
        // `fields.allow` is accepted by the schema/model but not enforced
        // by the compiler (CLS only derives masked_fields from
        // `deny`-list entries -- an allow-list can't be reduced to a
        // field-name-to-strip list without knowing a record's full key
        // set). Silently ignoring it would give the policy author the
        // opposite of what they declared: every field returned instead of
        // only the allowed ones. Reject it here so that's a loud parse-time
        // error, not a silent full-exposure no-op.
        if let Some(fields) = &perm.fields
            && fields.allow.is_some()
        {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' permission '{perm_name}' declares fields.allow, which \
                 this slice does not enforce (only fields.deny is compiled) -- express the \
                 restriction as fields.deny instead"
            )));
        }
        // `compile_cls`/`strip_masked_fields` treat every `fields.deny`
        // entry as a flat top-level JSON key (a plain `Map::remove`, no
        // path parsing). A dotted entry like "profile.ssn" would silently
        // mask nothing -- the key never exists at the top level -- while
        // reading as if nested-field masking were supported. Reject it
        // loudly instead of letting it round-trip as a no-op.
        if let Some(fields) = &perm.fields
            && let Some(deny) = &fields.deny
            && let Some(dotted) = deny.iter().find(|f| f.contains('.'))
        {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' permission '{perm_name}' declares fields.deny entry \
                 '{dotted}', which looks like a nested field path -- this slice only masks flat \
                 top-level keys, so a dotted entry would silently mask nothing"
            )));
        }
    }
    if let Some(default) = &def.default
        && !def.permissions.contains_key(default)
    {
        return Err(PolicyError::Semantic(format!(
            "definition '{type_name}' default '{default}' is not a declared permission"
        )));
    }
    validate_includes_acyclic(type_name, def)
}

fn validate_includes_acyclic(type_name: &str, def: &Definition) -> Result<(), PolicyError> {
    fn visit<'a>(
        type_name: &str,
        def: &'a Definition,
        node: &'a str,
        visiting: &mut BTreeSet<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Result<(), PolicyError> {
        if done.contains(node) {
            return Ok(());
        }
        if !visiting.insert(node) {
            return Err(PolicyError::Semantic(format!(
                "definition '{type_name}' has a cyclic 'includes' chain through permission \
                 '{node}'"
            )));
        }
        if let Some(perm) = def.permissions.get(node) {
            for included in &perm.includes {
                visit(type_name, def, included, visiting, done)?;
            }
        }
        visiting.remove(node);
        done.insert(node);
        Ok(())
    }

    let mut done = BTreeSet::new();
    for perm_name in def.permissions.keys() {
        let mut visiting = BTreeSet::new();
        visit(type_name, def, perm_name, &mut visiting, &mut done)?;
    }
    Ok(())
}

fn validate_path(
    policy: &Policy,
    start_type: &str,
    perm_name: &str,
    path: &[String],
) -> Result<(), PolicyError> {
    let Some((terminal, rel_names)) = path.split_last() else {
        return Err(PolicyError::Semantic(format!(
            "definition '{start_type}' permission '{perm_name}' has an empty path"
        )));
    };
    if terminal != "caller" && terminal != "anchor" {
        return Err(PolicyError::Semantic(format!(
            "definition '{start_type}' permission '{perm_name}' path ends in unknown terminal \
             '{terminal}' (expected 'caller' or 'anchor')"
        )));
    }
    if rel_names.len() > MAX_PATH_HOPS {
        return Err(PolicyError::Semantic(format!(
            "definition '{start_type}' permission '{perm_name}' path has {} relation hops, \
             exceeding the {MAX_PATH_HOPS} maximum",
            rel_names.len()
        )));
    }

    let mut current_type: &str = start_type;
    for (i, rel_name) in rel_names.iter().enumerate() {
        let current_def = policy.definitions.get(current_type).ok_or_else(|| {
            PolicyError::Semantic(format!(
                "definition '{start_type}' permission '{perm_name}' path references unknown \
                 object type '{current_type}'"
            ))
        })?;
        let rel = current_def.relations.get(rel_name).ok_or_else(|| {
            PolicyError::Semantic(format!(
                "definition '{start_type}' permission '{perm_name}' path references unknown \
                 relation '{rel_name}' on object type '{current_type}'"
            ))
        })?;
        if rel.service.is_some() {
            if i != rel_names.len() - 1 {
                return Err(PolicyError::Semantic(format!(
                    "definition '{start_type}' permission '{perm_name}' path's remote relation \
                     '{rel_name}' must be the last hop before the terminal (B3: there is no local \
                     table to keep joining through past a cross-service relation)"
                )));
            }
            // `caller` is rejected outright here, not silently reinterpreted
            // as `anchor` -- a remote fetch always asks the data-owning node
            // about the original principal (`compile::emit_remote_terminal`
            // unconditionally binds `session.anchor_did.unwrap_or(subject_did)`,
            // ignoring whatever terminal word the path names), so a policy
            // author who writes `caller` on a remote path would otherwise
            // get `anchor` semantics -- a strictly *broader* principal in any
            // proxied chain -- with no error and no warning. The
            // confused-deputy defense this exists for is exactly the reason
            // this must be a loud parse-time error instead of an invisible
            // substitution.
            if terminal != "anchor" {
                return Err(PolicyError::Semantic(format!(
                    "definition '{start_type}' permission '{perm_name}' path's remote relation \
                     '{rel_name}' must terminate in 'anchor', not '{terminal}' -- a remote fetch \
                     always resolves against the original principal, never the proxying caller"
                )));
            }
            // A remote relation's target isn't locally resolvable (it lives
            // in another service's policy), so the rest of this path can't
            // be validated here (there is none left, per the check above).
            // Compiling it resolves the fetch instead (`compile::plan_read`).
            return Ok(());
        }
        current_type = &rel.target;
    }

    let terminal_def = policy.definitions.get(current_type).ok_or_else(|| {
        PolicyError::Semantic(format!(
            "definition '{start_type}' permission '{perm_name}' path terminal reaches unknown \
             object type '{current_type}'"
        ))
    })?;
    if terminal_def.principal_column.is_none() {
        return Err(PolicyError::Semantic(format!(
            "object type '{current_type}' is used as a path terminal (definition '{start_type}' \
             permission '{perm_name}') but declares no principal_column"
        )));
    }
    Ok(())
}
