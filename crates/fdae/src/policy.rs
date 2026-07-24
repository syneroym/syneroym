//! The FDAE policy document: typed deserialization target for the `fdae/v1`
//! schema (ADR-0017 §1) plus the semantic validation the JSON Schema can't
//! express.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

/// Upper bound on a path's relation-hop count (excluding the terminal),
/// matching the schema's `paths` item `maxItems: 33` (32 hops + 1
/// terminal). `compile::emit_chain` recurses once per hop with no other
/// depth guard of its own, so an unbounded path would let a policy author
/// (accidentally or otherwise) drive that recursion deep enough to blow the
/// Rust stack -- a process abort (`SIGABRT`), not a catchable error, taking
/// down every service on the substrate, not just the one whose policy this
/// is. Rejected here, at parse time, rather than left to be discovered at
/// first query-compile time against a already-deployed policy.
const MAX_PATH_HOPS: usize = 32;

/// A parsed and validated `fdae/v1` policy document.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: String,
    #[serde(default)]
    pub strict: bool,
    pub definitions: BTreeMap<String, Definition>,
}

/// One `definitions:` entry: a logical object type backed by a physical
/// table.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub table: String,
    /// Column on `table` whose value is the principal DID a `caller`/`anchor`
    /// terminal compares against, when this object type is reached as a path
    /// terminal's target (ADR-0017 Amendments, 2026-07-20). Reserved-name
    /// aware: `id`/`creator_id`/`created_at`/`updated_at` map to the physical
    /// column, any other name maps to `json_extract(payload, '$.<name>')`.
    #[serde(default)]
    pub principal_column: Option<String>,
    #[serde(default)]
    pub relations: BTreeMap<String, Relation>,
    #[serde(default)]
    pub permissions: BTreeMap<String, Permission>,
    /// The permission applied when a caller reaches this object via a grant
    /// but no permission is otherwise selected. Absent means default-deny
    /// within the policy.
    #[serde(default)]
    pub default: Option<String>,
    /// B3 D-B3-3: opts this definition into structural cross-service
    /// relationship resolution -- a remote node's `resolve-relation`
    /// answering "which rows does `principal` reach" via a bare
    /// `principal_column` match, gated only by the requesting anchor's
    /// re-verified identity, **not** by a capability grant on this service.
    /// This is a deliberately looser trust model than the default (reusing
    /// the existing capability-gated sieve, which requires the anchor to
    /// separately hold a real capability from this service): a definition's
    /// own operator must explicitly opt in, per object type, exactly like
    /// `principal_column` itself is an opt-in declaration. `false` by
    /// default -- resolving a relation without this flag requires the
    /// anchor to hold a real capability (the compile_read path).
    #[serde(default)]
    pub resolvable_without_capability: bool,
}

/// A named edge from one object type to another: a local single-hop join, a
/// recursive self-join, or a remote (cross-service) reference.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub target: String,
    /// Remote relation (ADR-0017 §1/§6): a logical service name resolved via
    /// the app-context registry, fetched over the Universal Proxy at query
    /// time (B3 pipeline stage 2, `compile::plan_read`). Requires
    /// `join_column` too, exactly like a local join -- it names which local
    /// column is checked (`IN (...)`) against the remote's returned id-set,
    /// since there is no local `target` table to `EXISTS`-join through.
    #[serde(default)]
    pub service: Option<String>,
    // -- local single-hop join (or, with `service` set, the local column
    // checked against a remote fetch's id-set) --
    #[serde(default)]
    pub join_column: Option<String>,
    // -- recursive self-join --
    #[serde(default)]
    pub from_key: Option<String>,
    #[serde(default)]
    pub to_key: Option<String>,
    #[serde(default)]
    pub recursive: bool,
}

/// A named permission: which platform operations it covers (`allows`) and
/// which rows it reaches (`paths`), plus attribute conditions and CLS.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Permission {
    #[serde(default)]
    pub allows: Vec<String>,
    #[serde(default)]
    pub operator: Operator,
    /// Each entry is `[relation..., terminal]`; an empty outer list (no
    /// entries at all) is `public` -- every row, for anyone holding this
    /// permission.
    #[serde(default)]
    pub paths: Vec<Vec<String>>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Declared entailment: this permission is also applicable whenever any
    /// of these are. Never derived from naming.
    #[serde(default)]
    pub includes: Vec<String>,
    #[serde(default)]
    pub fields: Option<FieldsPolicy>,
}

/// An attribute predicate binding a caller claim against a row column:
/// `<col(def, column)> <op> ?`, with `?` bound to `session.claims[claim]`. A
/// referenced claim absent from `session.claims` makes the condition false
/// (fail-closed), never skipped.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub column: String,
    pub claim: String,
    #[serde(default)]
    pub op: CondOp,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CondOp {
    #[default]
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Operator {
    #[default]
    Union,
    Intersection,
    Exclusion,
}

/// CLS: column-level allow/deny lists (ADR-0015 A3 shape).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FieldsPolicy {
    #[serde(default)]
    pub allow: Option<Vec<String>>,
    #[serde(default)]
    pub deny: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("policy failed schema validation: {0}")]
    Schema(String),
    #[error("policy semantic error: {0}")]
    Semantic(String),
    #[error("unsupported policy version: {0}")]
    UnsupportedVersion(String),
}

const FDAE_V1_SCHEMA: &str = include_str!("../schema/fdae-v1.json");

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
    // only two shapes -- `join_column` and `service` may coexist (B3:
    // `join_column` names the *local* column checked against the remote
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
        // by this slice's compiler (CLS only derives masked_fields from
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_doc(definitions: &str) -> String {
        format!(r#"{{"version": "fdae/v1", "definitions": {definitions}}}"#)
    }

    #[test]
    fn parses_minimal_valid_policy() {
        let doc = minimal_doc(r#"{"user": {"table": "users", "principal_column": "did"}}"#);
        let policy = parse_and_validate(&doc).unwrap();
        assert_eq!(policy.version, "fdae/v1");
        assert!(!policy.strict);
        assert_eq!(policy.definitions.len(), 1);
        assert!(!policy.definitions["user"].resolvable_without_capability);
    }

    #[test]
    fn parses_resolvable_without_capability_when_declared() {
        let doc = minimal_doc(
            r#"{"employee": {
                "table": "employees", "principal_column": "did",
                "resolvable_without_capability": true
            }}"#,
        );
        let policy = parse_and_validate(&doc).unwrap();
        assert!(policy.definitions["employee"].resolvable_without_capability);
    }

    #[test]
    fn rejects_wrong_version_at_schema_stage() {
        let doc = minimal_doc(r#"{"user": {"table": "users"}}"#).replace("fdae/v1", "fdae/v2");
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Schema(_)));
    }

    #[test]
    fn rejects_unknown_top_level_field_via_schema() {
        let doc = r#"{"version": "fdae/v1", "definitions": {}, "bogus": true}"#;
        let err = parse_and_validate(doc).unwrap_err();
        assert!(matches!(err, PolicyError::Schema(_)));
    }

    #[test]
    fn rejects_relation_with_no_shape() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"creator": {"target": "user"}}}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn accepts_remote_relation_with_join_column() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"creator": {
                "target": "user", "join_column": "creator_uuid", "service": "hr-svc"
            }}}}"#,
        );
        parse_and_validate(&doc).unwrap();
    }

    #[test]
    fn rejects_relation_with_join_and_recursive_shapes() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "relations": {
                "management_chain": {
                    "target": "user", "join_column": "manager_id", "from_key": "id",
                    "to_key": "manager_id", "recursive": true
                }
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_recursive_relation_that_is_also_remote() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "relations": {
                "management_chain": {
                    "target": "user", "from_key": "id", "to_key": "manager_id",
                    "recursive": true, "service": "hr-svc"
                }
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_recursive_shape_missing_recursive_flag() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "relations": {
                "management_chain": {"target": "user", "from_key": "id", "to_key": "manager_id"}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_relation_target_not_a_definition() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"creator": {
                "target": "nobody", "join_column": "creator_uuid"
            }}}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn accepts_remote_relation_target_unresolved_locally() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"owner": {
                "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
            }}}}"#,
        );
        parse_and_validate(&doc).unwrap();
    }

    #[test]
    fn rejects_remote_relation_missing_join_column() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"owner": {
                "target": "employee", "service": "hr-svc"
            }}}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    /// B3-04: a `caller` terminal on a path whose last hop is remote is a
    /// parse-time error, not a silent substitution of `anchor` --
    /// `compile::emit_remote_terminal` unconditionally binds the anchor
    /// regardless of the declared terminal word, so accepting `caller` here
    /// would let a policy author write one thing and get another
    /// (`anchor` is the *broader* principal in any proxied chain).
    #[test]
    fn rejects_a_caller_terminal_on_a_remote_relation_path() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"owner": {
                "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
            }}, "permissions": {"view": {
                "allows": ["data-layer/read"], "paths": [["owner", "caller"]]
            }}}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    /// The mirror: `anchor` on the same shape is accepted.
    #[test]
    fn accepts_an_anchor_terminal_on_a_remote_relation_path() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"owner": {
                "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
            }}, "permissions": {"view": {
                "allows": ["data-layer/read"], "paths": [["owner", "anchor"]]
            }}}}"#,
        );
        parse_and_validate(&doc).unwrap();
    }

    #[test]
    fn rejects_path_terminal_target_missing_principal_column() {
        let doc = minimal_doc(
            r#"{
                "document": {"table": "documents", "relations": {"creator": {
                    "target": "user", "join_column": "creator_uuid"
                }}, "permissions": {"view": {"paths": [["creator", "caller"]]}}},
                "user": {"table": "users"}
            }"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn accepts_zero_hop_path_on_a_self_principal_type() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view_self": {"paths": [["caller"]]}
            }}}"#,
        );
        parse_and_validate(&doc).unwrap();
    }

    #[test]
    fn rejects_path_with_unknown_relation() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "permissions": {
                "view": {"paths": [["nonexistent", "caller"]]}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_path_with_unknown_terminal() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view": {"paths": [["nobody"]]}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn accepts_anchor_terminal_at_parse_time() {
        // `anchor` is a syntactically valid terminal at parse time
        // regardless of compile-time support -- compile.rs is where it
        // resolves to a bound value.
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view": {"paths": [["anchor"]]}
            }}}"#,
        );
        parse_and_validate(&doc).unwrap();
    }

    #[test]
    fn rejects_includes_naming_unknown_permission() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view": {"paths": [["caller"]], "includes": ["ghost"]}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_cyclic_includes() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "a": {"paths": [["caller"]], "includes": ["b"]},
                "b": {"paths": [["caller"]], "includes": ["a"]}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_default_naming_unknown_permission() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "default": "ghost",
                "permissions": {"view": {"paths": [["caller"]]}}}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn accepts_the_adr_worked_example_shape() {
        let doc = minimal_doc(
            r#"{
                "document": {
                    "table": "documents",
                    "relations": {
                        "creator": {"target": "user", "join_column": "creator_uuid"},
                        "parent_dept": {"target": "department", "join_column": "owner_dept_id"}
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "operator": "union",
                            "paths": [["creator", "caller"], ["creator", "management_chain", "caller"]]
                        },
                        "manage": {
                            "allows": ["data-layer/read", "data-layer/write", "rpc/move"],
                            "includes": ["view"],
                            "paths": [["creator", "caller"]]
                        }
                    }
                },
                "user": {
                    "table": "users",
                    "principal_column": "did",
                    "relations": {
                        "management_chain": {
                            "target": "user", "from_key": "id", "to_key": "manager_id",
                            "recursive": true
                        }
                    }
                },
                "department": {"table": "departments", "principal_column": "owner_did"}
            }"#,
        );
        let policy = parse_and_validate(&doc).unwrap();
        assert_eq!(policy.definitions.len(), 3);
    }

    #[test]
    fn rejects_fields_allow_since_this_slice_does_not_enforce_it() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view_self": {"paths": [["caller"]], "fields": {"allow": ["name"]}}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_fields_deny_with_a_dotted_nested_path() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view_self": {"paths": [["caller"]], "fields": {"deny": ["profile.ssn"]}}
            }}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn accepts_fields_deny_without_allow() {
        let doc = minimal_doc(
            r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
                "view_self": {"paths": [["caller"]], "fields": {"deny": ["ssn"]}}
            }}}"#,
        );
        parse_and_validate(&doc).unwrap();
    }

    #[test]
    fn rejects_a_table_name_that_is_not_a_safe_sql_identifier() {
        let doc = minimal_doc(r#"{"user": {"table": "users'; DROP TABLE users; --"}}"#);
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Schema(_)));
    }

    #[test]
    fn rejects_a_join_column_that_is_not_a_safe_sql_identifier() {
        let doc = minimal_doc(
            r#"{"document": {"table": "documents", "relations": {"creator": {
                "target": "user", "join_column": "creator_uuid') OR ('1'='1"
            }}}}"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Schema(_)));
    }

    #[test]
    fn rejects_a_definitions_table_colliding_with_another_definitions_key() {
        let doc = minimal_doc(
            r#"{
                "orders": {"table": "orders_tbl"},
                "shipments": {"table": "orders"}
            }"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_a_definitions_table_colliding_case_insensitively() {
        let doc = minimal_doc(
            r#"{
                "orders": {"table": "orders_tbl"},
                "shipments": {"table": "ORDERS_TBL"}
            }"#,
        );
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn rejects_a_path_exceeding_the_max_hop_count_via_schema() {
        let hops: Vec<String> = (0..40).map(|i| format!("\"hop{i}\"")).collect();
        let path = format!("[{}, \"caller\"]", hops.join(", "));
        let doc = minimal_doc(&format!(
            r#"{{"user": {{"table": "users", "principal_column": "did", "permissions": {{
                "view": {{"paths": [{path}]}}
            }}}}}}"#
        ));
        let err = parse_and_validate(&doc).unwrap_err();
        assert!(matches!(err, PolicyError::Schema(_)));
    }

    #[test]
    fn rejects_a_path_exceeding_the_max_hop_count_at_the_semantic_layer_too() {
        // Defense in depth: even a `Policy` constructed directly --
        // bypassing `parse_and_validate`'s schema gate entirely, which
        // nothing stops a caller from doing since every field here is
        // `pub` -- must still be caught by `validate_semantics`'s own
        // hop-count check, not just the schema's `maxItems`.
        let mut path: Vec<String> = (0..=MAX_PATH_HOPS).map(|i| format!("hop{i}")).collect();
        path.push("caller".to_string());
        let mut permissions = BTreeMap::new();
        permissions.insert(
            "view".to_string(),
            Permission {
                allows: vec![],
                operator: Operator::default(),
                paths: vec![path],
                conditions: vec![],
                includes: vec![],
                fields: None,
            },
        );
        let mut definitions = BTreeMap::new();
        definitions.insert(
            "user".to_string(),
            Definition {
                table: "users".to_string(),
                principal_column: Some("did".to_string()),
                relations: BTreeMap::new(),
                permissions,
                default: None,
                resolvable_without_capability: false,
            },
        );
        let policy = Policy { version: "fdae/v1".to_string(), strict: false, definitions };
        let err = validate_semantics(&policy).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }
}
