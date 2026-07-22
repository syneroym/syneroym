//! ReBAC -> SQL compiler (ADR-0017 §3-§4): compiles a parsed [`Policy`] and a
//! caller's `SessionContext` into a parameterized `WHERE EXISTS`/`WITH
//! RECURSIVE` row-security block (RLS) plus CLS field-masking metadata.
//!
//! The watchdog/timeout backstop (ADR-0017 §8) is not compiled into SQL --
//! it is installed on the connection around query execution by the caller
//! (`data_db`), exactly like the existing aggregate-query progress handler.

use std::{collections::BTreeSet, ptr};

use rusqlite::types::Value;
use serde_json::Value as Json;
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};

use crate::{
    policy::{CondOp, Definition, Operator, Permission, Policy, PolicyError, Relation},
    trace::DecisionTrace,
};

/// Depth backstop for a recursive relation's self-join, bound as a `?`
/// param (never interpolated). The `visited_track` path-concatenation guard
/// is the primary cycle defense; this is the secondary bound.
const MAX_RECURSION_DEPTH: i64 = 64;

/// Physical columns every collection table carries directly; any other
/// policy-referenced column name addresses the JSON `payload` column
/// instead (`json_extract`).
const RESERVED_COLUMNS: [&str; 4] = ["id", "creator_id", "created_at", "updated_at"];

/// The compiled row-security block, shaped so it ANDs directly onto
/// `data_db`'s `CompiledFilter` with no conversion.
#[derive(Debug, Clone)]
pub struct CompiledSieve {
    /// A boolean SQL expression over the base table's columns.
    pub where_clause: String,
    /// Bound values, in binding order.
    pub params: Vec<Value>,
    /// CLS: payload JSON field paths to strip post-fetch. Derived from
    /// `deny`-list entries only (policy `Permission.fields.deny` union each
    /// entitling capability's `caveats.fields.deny`). `parse_and_validate`
    /// rejects a policy `Permission.fields.allow` outright (an allow-list
    /// can't be reduced to a field-name-to-strip list without knowing a
    /// record's full key set, which this compiler does not have), so it
    /// never reaches here from the policy side; a capability's *caveat*
    /// `fields.allow` is a runtime UCAN value outside the policy document
    /// and is not similarly rejectable -- it remains an unenforced no-op,
    /// same category as `syneroym-ucan`'s
    /// `caveats_passthrough_is_not_yet_enforced`.
    pub masked_fields: Vec<String>,
    /// Each entitling capability's raw `caveats.where` document (an
    /// ADR-0007 MongoDB-style filter), for the caller to compile via
    /// `data_db`'s `filter::compile_filter` and AND onto this sieve.
    pub where_caveats: Vec<Json>,
    /// ADR-0017 §9 decision trace for this compilation, already emitted via
    /// `tracing` by `compile_read`. Carried on the sieve so a Mode A caller
    /// (`check_access`) can clone it, fill in `rows_reached` once the
    /// predicate has actually been run, and emit a second, execution-aware
    /// trace.
    pub trace: DecisionTrace,
}

/// Which of ADR-0017 §4's two compilation modes to produce.
#[derive(Debug, Clone)]
pub enum Mode {
    /// Mode B -- wrap the caller's own query: no rows outside the sieve are
    /// ever returned.
    Filter,
    /// Mode A -- point-in-time: reduce the sieve to a boolean over one row.
    PointInTime { id: String },
}

/// Compiles the row-security block for `operation` on `collection`, as seen
/// by `session` under `policy`. `service_id` names the app-instance/service
/// pair the collection belongs to (used to build the collection-qualified
/// resource capabilities are checked against, ADR-0017 §3.2's grant∩policy
/// intersection fix). `operation` is the platform ability actually being
/// requested (`data-layer/read` for `query`/`get`/`aggregate`, or whatever
/// `check-access` was asked about for Mode A) -- only permissions whose
/// `allows` covers it (by `Ability::entails`) can become applicable, so a
/// caller holding only a read-level capability cannot pass a write-mode
/// point-in-time check.
///
/// Returns:
/// - `Ok(None)` -- no definition for this collection and the policy is not
///   `strict`: the grant layer already admitted this read, no filtering.
/// - `Ok(Some(sieve))` -- apply this block (may be a deny-all `0=1`).
/// - `Err(PolicyError)` -- malformed/unsupported input; the caller must treat
///   this as deny, never as unfiltered access.
pub fn compile_read(
    policy: &Policy,
    collection: &str,
    session: &SessionContext,
    service_id: &str,
    operation: &Ability,
    mode: Mode,
) -> Result<Option<CompiledSieve>, PolicyError> {
    let Some((object_type, def)) = find_definition(policy, collection) else {
        if !policy.strict {
            return Ok(None);
        }
        let trace = DecisionTrace {
            tier: 3,
            operation_admitted: false,
            path_failed: Some(format!(
                "no policy definition matches collection '{collection}' and the policy is strict"
            )),
            compiled_predicate: Some("0=1".to_string()),
            ..DecisionTrace::default()
        };
        trace.emit();
        return Ok(Some(CompiledSieve { trace, ..deny_all() }));
    };

    let resource = ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(service_id, service_id).0
    ));

    let (mut applicable, mut entitling_caps) =
        applicable_permissions(def, object_type, &resource, operation, session);
    close_over_includes(&mut applicable, def, operation);

    if applicable.is_empty() {
        let holding_caps: Vec<&Capability> =
            session.capabilities.iter().filter(|cap| cap.grants(&resource, operation)).collect();
        let operation_admitted = !holding_caps.is_empty();
        // The default permission is only a fallback *within the same
        // grant-intersection contract* every other route obeys: its own
        // `allows` must cover `operation`, or a caller holding an unrelated
        // (e.g. write) capability could ride a read-only (or ability-less)
        // default permission's paths straight through a write-mode check.
        let default_covers_operation =
            def.default.as_ref().and_then(|name| def.permissions.get(name)).is_some_and(|perm| {
                perm.allows.iter().any(|a| Ability(a.clone()).entails(operation))
            });
        match &def.default {
            Some(default_perm) if operation_admitted && default_covers_operation => {
                applicable.insert(default_perm.clone());
                for cap in holding_caps {
                    push_unique(&mut entitling_caps, cap);
                }
            }
            _ => {
                let path_failed = if !operation_admitted {
                    format!(
                        "no held capability grants operation '{}' on this resource",
                        operation.0
                    )
                } else {
                    "operation is granted by a held capability, but no permission's allows covers \
                     it and no applicable default permission is configured"
                        .to_string()
                };
                let trace = DecisionTrace {
                    tier: 3,
                    held: describe_caps(&holding_caps),
                    operation_admitted,
                    path_failed: Some(path_failed),
                    compiled_predicate: Some("0=1".to_string()),
                    ..DecisionTrace::default()
                };
                trace.emit();
                return Ok(Some(CompiledSieve { trace, ..deny_all() }));
            }
        }
    }

    let mut params: Vec<Value> = Vec::new();
    let mut clauses: Vec<String> = Vec::with_capacity(applicable.len());
    let mut claim_absent_for: Vec<String> = Vec::new();
    for pname in &applicable {
        let Some(perm) = def.permissions.get(pname) else {
            // `default` is validated at parse time to name a real
            // permission; every other member of `applicable` came from
            // `def.permissions` directly. Unreachable in practice, but
            // fail closed rather than panic.
            return Err(PolicyError::Semantic(format!(
                "permission '{pname}' not found on definition '{object_type}'"
            )));
        };
        let clause = compile_permission(policy, object_type, perm, session, &mut params)?;
        // `compile_permission` returns exactly the literal string "0=1" in
        // one place: a condition whose claim is absent from
        // `session.claims`. Every other branch builds "1=1" or an `EXISTS`
        // predicate, so this text match unambiguously identifies the
        // claim-absent fail-closed case for the decision trace below.
        if clause == "0=1" {
            claim_absent_for.push(pname.clone());
        }
        clauses.push(clause);
    }
    let base_where_clause = format!("({})", clauses.join(" OR "));
    let mut where_clause = base_where_clause.clone();

    if let Mode::PointInTime { id } = &mode {
        where_clause = format!("({where_clause}) AND {}.id = ?", def.table);
        params.push(Value::Text(id.clone()));
    }

    let masked_fields = compile_cls(def, &applicable, &entitling_caps)?;
    let where_caveats: Vec<Json> = entitling_caps
        .iter()
        .filter_map(|cap| cap.caveats.as_ref()?.get("where").cloned())
        .collect();

    let path_failed = (base_where_clause == "(0=1)").then(|| {
        if claim_absent_for.is_empty() {
            "no applicable permission's path predicate is satisfiable".to_string()
        } else {
            format!("condition claim absent for permission(s): {}", claim_absent_for.join(", "))
        }
    });
    let caveats_applied: Vec<String> = masked_fields
        .iter()
        .map(|f| format!("fields.deny:{f}"))
        .chain(where_caveats.iter().map(|c| format!("where:{c}")))
        .collect();
    let trace = DecisionTrace {
        tier: 3,
        held: describe_caps(&entitling_caps),
        operation_admitted: true,
        applicable_permissions: applicable.iter().cloned().collect(),
        compiled_predicate: Some(where_clause.clone()),
        rows_reached: None,
        path_failed,
        caveats_applied,
    };
    trace.emit();

    Ok(Some(CompiledSieve { where_clause, params, masked_fields, where_caveats, trace }))
}

/// `held` descriptors for a decision trace: `<resource>::<ability>` per
/// evaluated capability, cheap and stable enough to log without exposing
/// caveat contents.
fn describe_caps(caps: &[&Capability]) -> Vec<String> {
    caps.iter().map(|cap| format!("{}::{}", cap.with.0, cap.can.0)).collect()
}

/// Matches case-insensitively (ASCII fold), mirroring SQLite's own
/// identifier resolution: an unquoted table name is case-insensitive, so a
/// case-sensitive lookup here would let a caller name the same physical
/// table under a spelling this policy doesn't recognize and fall through to
/// the unfiltered "no definition" path while the query still hits the real,
/// policy-governed table.
fn find_definition<'a>(policy: &'a Policy, collection: &str) -> Option<(&'a str, &'a Definition)> {
    policy
        .definitions
        .iter()
        .find(|(key, def)| {
            key.as_str().eq_ignore_ascii_case(collection)
                || def.table.eq_ignore_ascii_case(collection)
        })
        .map(|(key, def)| (key.as_str(), def))
}

fn deny_all() -> CompiledSieve {
    CompiledSieve {
        where_clause: "0=1".to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
        trace: DecisionTrace::default(),
    }
}

/// The grant∩policy intersection (ADR-0017 §2/§3.2): a permission is
/// applicable when its `allows` covers `operation` *and* the caller holds a
/// capability that grants it -- either a platform-ability capability whose
/// `can` entails one of `allows`' covering abilities, or a capability
/// naming this exact `app/<object_type>.<permission>` reference.
fn applicable_permissions<'a>(
    def: &Definition,
    object_type: &str,
    resource: &ResourceUri,
    operation: &Ability,
    session: &'a SessionContext,
) -> (BTreeSet<String>, Vec<&'a Capability>) {
    let mut applicable = BTreeSet::new();
    let mut entitling_caps: Vec<&Capability> = Vec::new();

    for (pname, perm) in &def.permissions {
        let covering_abilities: Vec<Ability> = perm
            .allows
            .iter()
            .map(|a| Ability(a.clone()))
            .filter(|ability| ability.entails(operation))
            .collect();
        if covering_abilities.is_empty() {
            continue;
        }

        let mut entitled = false;
        let app_ability = Ability(format!("app/{object_type}.{pname}"));
        for cap in &session.capabilities {
            if covering_abilities.iter().any(|ability| cap.grants(resource, ability)) {
                entitled = true;
                push_unique(&mut entitling_caps, cap);
            }
            if cap.grants(resource, &app_ability) {
                entitled = true;
                push_unique(&mut entitling_caps, cap);
            }
        }
        if entitled {
            applicable.insert(pname.clone());
        }
    }

    (applicable, entitling_caps)
}

fn push_unique<'a>(caps: &mut Vec<&'a Capability>, cap: &'a Capability) {
    if !caps.iter().any(|held| ptr::eq(*held, cap)) {
        caps.push(cap);
    }
}

/// Widens `applicable` by each already-applicable permission's `includes`,
/// but only when the included permission's *own* `allows` covers
/// `operation` -- otherwise a write-mode check could pull in an
/// unconditionally-public (`paths: []`) read-only sibling permission and
/// silently grant write access through its `1=1` path predicate. Closure
/// widens *which* already-operation-eligible permissions apply; it must
/// never re-open the operation gate `applicable_permissions` already
/// closed.
fn close_over_includes(applicable: &mut BTreeSet<String>, def: &Definition, operation: &Ability) {
    loop {
        let additions: Vec<String> = applicable
            .iter()
            .filter_map(|pname| def.permissions.get(pname))
            .flat_map(|perm| perm.includes.iter().cloned())
            .filter(|included| !applicable.contains(included))
            .filter(|included| {
                def.permissions.get(included).is_some_and(|perm| {
                    perm.allows.iter().any(|a| Ability(a.clone()).entails(operation))
                })
            })
            .collect();
        if additions.is_empty() {
            return;
        }
        applicable.extend(additions);
    }
}

/// CLS: field masking derived from `deny`-list entries only. A policy
/// `Permission.fields.allow` is rejected at parse time (`policy::
/// validate_permissions`), since it can't be reduced to a field-name list
/// at compile time -- doing so would require knowing every key a record's
/// JSON payload might carry, which the policy model does not declare. A
/// capability's *caveat* `fields.allow` reaches here unrestricted (caveats
/// are a runtime UCAN value, not part of the parsed policy document) and
/// remains an unenforced no-op -- see `CompiledSieve::masked_fields`.
fn compile_cls(
    def: &Definition,
    applicable: &BTreeSet<String>,
    entitling_caps: &[&Capability],
) -> Result<Vec<String>, PolicyError> {
    let mut denied: BTreeSet<String> = BTreeSet::new();

    for pname in applicable {
        let Some(perm) = def.permissions.get(pname) else { continue };
        let Some(fields) = &perm.fields else { continue };
        let Some(deny) = &fields.deny else { continue };
        denied.extend(deny.iter().cloned());
    }

    for cap in entitling_caps {
        let Some(caveats) = &cap.caveats else { continue };
        let Some(deny) = caveats.get("fields").and_then(|f| f.get("deny")).and_then(Json::as_array)
        else {
            continue;
        };
        denied.extend(deny.iter().filter_map(|v| v.as_str().map(str::to_string)));
    }

    // A dotted entry (from a runtime capability caveat, so it can't be
    // rejected at policy parse time the way a policy `fields.deny` entry
    // is) would silently mask nothing: `strip_masked_fields` only removes
    // flat top-level keys. Fail closed rather than let it round-trip as an
    // unenforced no-op.
    if let Some(dotted) = denied.iter().find(|f| f.contains('.')) {
        return Err(PolicyError::Semantic(format!(
            "capability caveat fields.deny entry '{dotted}' looks like a nested field path -- \
             this slice only masks flat top-level keys, so a dotted entry would silently mask \
             nothing"
        )));
    }

    Ok(denied.into_iter().collect())
}

fn compile_permission(
    policy: &Policy,
    object_type: &str,
    perm: &Permission,
    session: &SessionContext,
    params: &mut Vec<Value>,
) -> Result<String, PolicyError> {
    // Resolve every condition's claim *before* compiling any path, so a
    // fail-closed short-circuit below never leaves params pushed by a path
    // predicate this permission is about to discard in favor of "0=1".
    let mut claim_values: Vec<&Json> = Vec::with_capacity(perm.conditions.len());
    for cond in &perm.conditions {
        let Some(val) = session.claims.get(&cond.claim) else {
            return Ok("0=1".to_string());
        };
        claim_values.push(val);
    }

    let mut path_pred = if perm.paths.is_empty() {
        "1=1".to_string()
    } else {
        let mut clauses = Vec::with_capacity(perm.paths.len());
        for path in &perm.paths {
            clauses.push(compile_path(policy, object_type, path, session, params)?);
        }
        match perm.operator {
            Operator::Union => format!("({})", clauses.join(" OR ")),
            Operator::Intersection => format!("({})", clauses.join(" AND ")),
            Operator::Exclusion => {
                let Some((first, rest)) = clauses.split_first() else {
                    return Err(PolicyError::Semantic(
                        "exclusion operator requires at least one path".to_string(),
                    ));
                };
                if rest.is_empty() {
                    format!("({first})")
                } else {
                    format!("({first} AND NOT ({}))", rest.join(" OR "))
                }
            }
        }
    };

    let def = get_def(policy, object_type)?;
    for (cond, claim_val) in perm.conditions.iter().zip(claim_values) {
        // `col_expr` (and the `?` it may push for a JSON-path param) must be
        // computed *before* the claim value is pushed: it appears first in
        // the text below, and positional `?` binding must match text order.
        let col_expr = col(&def.table, &cond.column, params);
        params.push(json_value_to_sql(claim_val)?);
        path_pred = format!("{path_pred} AND {col_expr} {} ?", sql_op(cond.op));
    }

    Ok(path_pred)
}

fn sql_op(op: CondOp) -> &'static str {
    match op {
        CondOp::Eq => "=",
        CondOp::Ne => "!=",
        CondOp::Gt => ">",
        CondOp::Gte => ">=",
        CondOp::Lt => "<",
        CondOp::Lte => "<=",
    }
}

fn json_value_to_sql(value: &Json) -> Result<Value, PolicyError> {
    match value {
        Json::Null => Ok(Value::Null),
        Json::Bool(b) => Ok(Value::Integer(i64::from(*b))),
        Json::Number(n) => {
            n.as_i64().map(Value::Integer).or_else(|| n.as_f64().map(Value::Real)).ok_or_else(
                || PolicyError::Semantic(format!("claim value '{n}' is not representable as SQL")),
            )
        }
        Json::String(s) => Ok(Value::Text(s.clone())),
        Json::Array(_) | Json::Object(_) => Err(PolicyError::Semantic(
            "a condition's claim value must be a scalar, not an array or object".to_string(),
        )),
    }
}

fn get_def<'a>(policy: &'a Policy, type_name: &str) -> Result<&'a Definition, PolicyError> {
    policy
        .definitions
        .get(type_name)
        .ok_or_else(|| PolicyError::Semantic(format!("unknown object type '{type_name}'")))
}

/// `<col>`: reserved names address the physical column directly (`name` is
/// checked against a fixed list, never interpolated as arbitrary text);
/// anything else addresses the JSON `payload` column via `json_extract`,
/// with the JSON path bound as a `?` parameter -- never spliced into the
/// string literal -- mirroring `data_db::filter::json_path_param`. Only
/// `qualifier` (a compiler-chosen alias, or a policy `table` name already
/// restricted by the schema's identifier pattern) is interpolated as text;
/// no SQL identifier can be bound as a parameter, so `table` still relies
/// on schema validation rather than this function.
fn col(qualifier: &str, name: &str, params: &mut Vec<Value>) -> String {
    if RESERVED_COLUMNS.contains(&name) {
        format!("{qualifier}.{name}")
    } else {
        params.push(Value::Text(format!("$.{name}")));
        format!("json_extract({qualifier}.payload, ?)")
    }
}

fn terminal_value(terminal: &str, session: &SessionContext) -> Result<String, PolicyError> {
    match terminal {
        "caller" => Ok(session.subject_did.clone()),
        "anchor" => Err(PolicyError::Semantic(
            "the 'anchor' path terminal is not implemented in this slice (B3); use 'caller'"
                .to_string(),
        )),
        other => Err(PolicyError::Semantic(format!("unknown path terminal '{other}'"))),
    }
}

/// One resolved, validated hop of a path's relation walk.
struct Hop<'a> {
    name: &'a str,
    relation: &'a Relation,
    target_def: &'a Definition,
}

/// Resolves and validates every non-terminal segment of a path in order,
/// failing closed on a remote relation (cross-service, B3) or a recursive
/// relation anywhere but the last hop (B2's supported placement, §3.4).
fn resolve_hops<'a>(
    policy: &'a Policy,
    start_type: &str,
    rel_names: &'a [String],
) -> Result<Vec<Hop<'a>>, PolicyError> {
    let mut hops = Vec::with_capacity(rel_names.len());
    let mut current_type: &str = start_type;
    for (i, rel_name) in rel_names.iter().enumerate() {
        let current_def = get_def(policy, current_type)?;
        let relation = current_def.relations.get(rel_name.as_str()).ok_or_else(|| {
            PolicyError::Semantic(format!(
                "relation '{rel_name}' not found on object type '{current_type}'"
            ))
        })?;
        if let Some(service) = &relation.service {
            return Err(PolicyError::Semantic(format!(
                "relation '{rel_name}' is remote (service: '{service}'); cross-service relations \
                 require B3"
            )));
        }
        if relation.recursive && i != rel_names.len() - 1 {
            return Err(PolicyError::Semantic(format!(
                "recursive relation '{rel_name}' must be the last hop before the path terminal"
            )));
        }
        let target_def = get_def(policy, &relation.target)?;
        hops.push(Hop { name: rel_name.as_str(), relation, target_def });
        current_type = &relation.target;
    }
    Ok(hops)
}

/// Walks a path (`[relation..., terminal]`) into a correlated `EXISTS`
/// subquery, or a single `EXISTS (WITH RECURSIVE ...)` block when the last
/// relation is recursive (§3.4).
fn compile_path(
    policy: &Policy,
    start_type: &str,
    path: &[String],
    session: &SessionContext,
    params: &mut Vec<Value>,
) -> Result<String, PolicyError> {
    let Some((terminal, rel_names)) = path.split_last() else {
        return Err(PolicyError::Semantic("path must have at least a terminal".to_string()));
    };

    let start_def = get_def(policy, start_type)?;

    // Zero-relation path: the terminal is checked directly on the starting
    // type's own row (e.g. a `user` definition's `paths: [["caller"]]`).
    if rel_names.is_empty() {
        let principal_col = start_def.principal_column.as_ref().ok_or_else(|| {
            PolicyError::Semantic(format!(
                "object type '{start_type}' is used as a path terminal but declares no \
                 principal_column"
            ))
        })?;
        let col_expr = col(&start_def.table, principal_col, params);
        let bound = terminal_value(terminal, session)?;
        params.push(Value::Text(bound));
        return Ok(format!("{col_expr} = ?"));
    }

    let hops = resolve_hops(policy, start_type, rel_names)?;
    if let Some(last) = hops.last()
        && last.relation.recursive
        && hops.len() < 2
    {
        return Err(PolicyError::Semantic(format!(
            "recursive relation '{}' needs a preceding local-join hop to correlate its seed from",
            last.name
        )));
    }

    let mut alias_idx = 0usize;
    emit_chain(&hops, &start_def.table, terminal, session, params, &mut alias_idx)
}

/// Emits nested `EXISTS` for a chain of local-join hops, fusing the last
/// two hops into a single `EXISTS (WITH RECURSIVE ...)` block when the
/// final hop is recursive -- the seed of the recursive walk is exactly the
/// row the immediately preceding local join reaches (worked example,
/// ADR-0017 §3.4), so it is not wrapped in its own extra `EXISTS` layer.
fn emit_chain(
    hops: &[Hop],
    correlate_qualifier: &str,
    terminal: &str,
    session: &SessionContext,
    params: &mut Vec<Value>,
    alias_idx: &mut usize,
) -> Result<String, PolicyError> {
    match hops {
        [] => Err(PolicyError::Semantic("internal: emit_chain called with no hops".to_string())),
        [leading, recursive] if recursive.relation.recursive => {
            emit_fused_recursive(leading, recursive, correlate_qualifier, terminal, session, params)
        }
        [hop, rest @ ..] => {
            *alias_idx += 1;
            let alias = format!("a{alias_idx}");
            let join_column = hop.relation.join_column.as_ref().ok_or_else(|| {
                PolicyError::Semantic(format!(
                    "relation '{}' is not a local join (missing join_column)",
                    hop.name
                ))
            })?;
            let correlate_expr = col(correlate_qualifier, join_column, params);
            let inner = if rest.is_empty() {
                let principal_col = hop.target_def.principal_column.as_ref().ok_or_else(|| {
                    PolicyError::Semantic(format!(
                        "object type '{}' is used as a path terminal but declares no \
                         principal_column",
                        hop.relation.target
                    ))
                })?;
                let col_expr = col(&alias, principal_col, params);
                let bound = terminal_value(terminal, session)?;
                params.push(Value::Text(bound));
                format!("{col_expr} = ?")
            } else {
                emit_chain(rest, &alias, terminal, session, params, alias_idx)?
            };
            Ok(format!(
                "EXISTS (SELECT 1 FROM {} AS {alias} WHERE {alias}.id = {correlate_expr} AND \
                 {inner})",
                hop.target_def.table
            ))
        }
    }
}

fn emit_fused_recursive(
    leading: &Hop,
    recursive: &Hop,
    correlate_qualifier: &str,
    terminal: &str,
    session: &SessionContext,
    params: &mut Vec<Value>,
) -> Result<String, PolicyError> {
    if recursive.relation.target != leading.relation.target {
        return Err(PolicyError::Semantic(format!(
            "recursive relation '{}' targets '{}', which must match the preceding relation '{}' \
             target '{}'",
            recursive.name, recursive.relation.target, leading.name, leading.relation.target
        )));
    }
    let join_column = leading.relation.join_column.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!(
            "relation '{}' is not a local join (missing join_column)",
            leading.name
        ))
    })?;
    let from_key = recursive.relation.from_key.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!("recursive relation '{}' missing from_key", recursive.name))
    })?;
    let to_key = recursive.relation.to_key.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!("recursive relation '{}' missing to_key", recursive.name))
    })?;
    let principal_col = recursive.target_def.principal_column.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!(
            "object type '{}' is used as a path terminal but declares no principal_column",
            recursive.relation.target
        ))
    })?;

    let seed_table = &recursive.target_def.table;

    // `from_key` under "u" and "u2" (and `principal_col`/`to_key`) each
    // appear several times in the text below. `col()` binds a non-reserved
    // name's JSON path as a fresh `?` param per call, so each textual
    // occurrence must call `col()` again (never reuse a previously
    // rendered fragment) -- reusing a cached "json_extract(..., ?)" string
    // would repeat its `?` in the text without a matching extra param.
    // Building left-to-right as a sequence of statements (rather than one
    // `format!`) makes each `col()` call's param land in the same position
    // as its `?` in the assembled text, by construction.
    let mut sql = String::new();
    sql.push_str("EXISTS (WITH RECURSIVE mc(id, prin, depth, seen) AS (SELECT ");
    sql.push_str(&col("u", from_key, params));
    sql.push_str(", ");
    sql.push_str(&col("u", principal_col, params));
    sql.push_str(", 0, '/' || ");
    sql.push_str(&col("u", from_key, params));
    sql.push_str(" || '/' FROM ");
    sql.push_str(seed_table);
    sql.push_str(" u WHERE ");
    sql.push_str(&col("u", from_key, params));
    sql.push_str(" = ");
    sql.push_str(&col(correlate_qualifier, join_column, params));
    sql.push_str(" UNION ALL SELECT ");
    sql.push_str(&col("u2", from_key, params));
    sql.push_str(", ");
    sql.push_str(&col("u2", principal_col, params));
    sql.push_str(", mc.depth + 1, mc.seen || ");
    sql.push_str(&col("u2", from_key, params));
    sql.push_str(" || '/' FROM ");
    sql.push_str(seed_table);
    sql.push_str(" u2 JOIN mc ON ");
    sql.push_str(&col("u2", from_key, params));
    sql.push_str(" = (SELECT ");
    sql.push_str(&col(seed_table, to_key, params));
    sql.push_str(" FROM ");
    sql.push_str(seed_table);
    sql.push_str(" WHERE ");
    sql.push_str(&col(seed_table, from_key, params));
    sql.push_str(" = mc.id) WHERE mc.depth < ?");
    params.push(Value::Integer(MAX_RECURSION_DEPTH));
    sql.push_str(" AND instr(mc.seen, '/' || ");
    sql.push_str(&col("u2", from_key, params));
    sql.push_str(" || '/') = 0) SELECT 1 FROM mc WHERE mc.prin = ?)");
    let bound = terminal_value(terminal, session)?;
    params.push(Value::Text(bound));

    Ok(sql)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::{Map, json};
    use syneroym_ucan::ResourceUri as Uri;

    use super::*;
    use crate::policy::parse_and_validate;

    const SERVICE_ID: &str = "svc-a";

    fn resource(collection: &str) -> Uri {
        Uri(format!("{}/collection/{collection}", Uri::service(SERVICE_ID, SERVICE_ID).0))
    }

    fn session(subject_did: &str, capabilities: Vec<Capability>) -> SessionContext {
        SessionContext {
            subject_did: subject_did.to_string(),
            capabilities,
            claims: Map::new(),
            verified_at_secs: 0,
        }
    }

    fn read_cap(collection: Option<&str>) -> Capability {
        let with = match collection {
            Some(c) => resource(c),
            None => ResourceUri::service(SERVICE_ID, SERVICE_ID),
        };
        Capability { with, can: Ability(Ability::DATA_LAYER_READ.to_string()), caveats: None }
    }

    fn run_sieve(conn: &Connection, base_table: &str, sieve: &CompiledSieve) -> Vec<String> {
        let sql = format!(
            "SELECT {base_table}.id FROM {base_table} WHERE {} ORDER BY {base_table}.id",
            sieve.where_clause
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        stmt.query_map(rusqlite::params_from_iter(sieve.params.iter()), |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    fn seed_schema(conn: &Connection) {
        conn.execute_batch(
            "
            CREATE TABLE users (
                id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
                payload TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE documents (
                id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
                payload TEXT NOT NULL DEFAULT '{}'
            );
            ",
        )
        .unwrap();
    }

    fn insert_user(conn: &Connection, id: &str, did: &str, manager_id: Option<&str>) {
        let payload = json!({"did": did, "manager_id": manager_id});
        conn.execute("INSERT INTO users (id, payload) VALUES (?1, ?2)", (id, payload.to_string()))
            .unwrap();
    }

    fn insert_document(conn: &Connection, id: &str, creator_uuid: &str) {
        let payload = json!({"creator_uuid": creator_uuid});
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
            (id, payload.to_string()),
        )
        .unwrap();
    }

    fn single_hop_policy() -> Policy {
        parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn single_hop_exists_prunes_to_the_creator() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_user(&conn, "u-bob", "did:key:bob", None);
        insert_document(&conn, "doc-1", "u-alice");
        insert_document(&conn, "doc-2", "u-bob");

        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();

        assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
    }

    #[test]
    fn single_hop_denies_a_stranger() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_policy();
        let mallory = session("did:key:mallory", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &mallory,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();

        assert!(run_sieve(&conn, "documents", &sieve).is_empty());
    }

    #[test]
    fn recursive_relation_terminates_on_a_cyclic_manager_graph() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        // eve -> frank -> eve: a deliberately cyclic manager graph.
        insert_user(&conn, "u-eve", "did:key:eve", Some("u-frank"));
        insert_user(&conn, "u-frank", "did:key:frank", Some("u-eve"));
        insert_user(&conn, "u-mallory", "did:key:mallory", None);
        insert_document(&conn, "doc-1", "u-eve");

        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "management_chain", "caller"]]
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
                    }
                }
            }"#,
        )
        .unwrap();

        for (did, expect_visible) in
            [("did:key:eve", true), ("did:key:frank", true), ("did:key:mallory", false)]
        {
            let s = session(did, vec![read_cap(Some("document"))]);
            let sieve = compile_read(
                &policy,
                "document",
                &s,
                SERVICE_ID,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .unwrap()
            .unwrap();
            let visible = run_sieve(&conn, "documents", &sieve);
            assert_eq!(!visible.is_empty(), expect_visible, "did={did}");
        }
    }

    #[test]
    fn exclusion_operator_and_condition_claim_bind_correctly() {
        // `view` = reachable as creator, EXCLUDING documents that
        // specifically embargo the caller, further ANDed with a
        // claims-bound region match. Both `creator` and `embargoed_from`
        // are ordinary single-hop (many-to-one) relations, so this
        // exercises the exclusion operator and the conditions/claims bind
        // through the real relation-walk machinery, not a hand-rolled
        // shape assertion.
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_user(&conn, "u-bob", "did:key:bob", None);
        let insert_embargo_doc = |id: &str, creator: &str, embargoed: &str, region: &str| {
            let payload =
                json!({"creator_uuid": creator, "embargoed_uuid": embargoed, "region": region});
            conn.execute(
                "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
                (id, payload.to_string()),
            )
            .unwrap();
        };
        // alice created all three; doc-1 embargoes alice herself (excluded
        // despite being creator); doc-2 embargoes bob instead (visible to
        // alice); doc-3 is like doc-2 but in a different region.
        insert_embargo_doc("doc-1", "u-alice", "u-alice", "EU");
        insert_embargo_doc("doc-2", "u-alice", "u-bob", "EU");
        insert_embargo_doc("doc-3", "u-alice", "u-bob", "US");

        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {
                            "creator": {"target": "user", "join_column": "creator_uuid"},
                            "embargoed_from": {"target": "user", "join_column": "embargoed_uuid"}
                        },
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "operator": "exclusion",
                                "paths": [["creator", "caller"], ["embargoed_from", "caller"]],
                                "conditions": [{"column": "region", "claim": "region"}]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();

        let alice_with_region = |region: &str| {
            let mut claims = Map::new();
            claims.insert("region".to_string(), json!(region));
            SessionContext {
                subject_did: "did:key:alice".to_string(),
                capabilities: vec![read_cap(Some("document"))],
                claims,
                verified_at_secs: 0,
            }
        };

        let alice_eu = alice_with_region("EU");
        let sieve = compile_read(
            &policy,
            "document",
            &alice_eu,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run_sieve(&conn, "documents", &sieve),
            vec!["doc-2"],
            "doc-1 excluded (alice is embargoed from it); doc-3 excluded (region mismatch)"
        );

        let alice_us = alice_with_region("US");
        let sieve = compile_read(
            &policy,
            "document",
            &alice_us,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run_sieve(&conn, "documents", &sieve),
            vec!["doc-3"],
            "region claim correctly switches the visible row"
        );
    }

    #[test]
    fn condition_with_absent_claim_fails_closed() {
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "user": {
                        "table": "users",
                        "principal_column": "did",
                        "permissions": {
                            "view_self": {
                                "allows": ["data-layer/read"],
                                "paths": [["caller"]],
                                "conditions": [{"column": "region", "claim": "region"}]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let alice = session("did:key:alice", vec![read_cap(Some("user"))]);
        let sieve = compile_read(
            &policy,
            "user",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.where_clause, "(0=1)");
    }

    #[test]
    fn no_definition_and_not_strict_is_unfiltered() {
        let policy = parse_and_validate(r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
        let alice = session("did:key:alice", vec![]);
        let sieve = compile_read(
            &policy,
            "unrelated_collection",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        assert!(sieve.is_none());
    }

    #[test]
    fn strict_mode_denies_an_undefined_collection() {
        let policy =
            parse_and_validate(r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#)
                .unwrap();
        let alice = session("did:key:alice", vec![]);
        let sieve = compile_read(
            &policy,
            "unrelated_collection",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.where_clause, "0=1");
    }

    #[test]
    fn no_applicable_permission_and_no_default_denies() {
        let policy = single_hop_policy();
        // A capability for a *different* collection: covers the operation
        // but not this resource, so no permission becomes applicable and
        // there is no `default` -- default-deny (D-04-02-b).
        let bob = session("did:key:bob", vec![read_cap(Some("other_collection"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &bob,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.where_clause, "0=1");
    }

    #[test]
    fn write_mode_check_ignores_a_read_only_permission() {
        // "view" only allows data-layer/read; a caller holding *only* a
        // read capability must not pass a write-mode point-in-time check
        // through it.
        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_WRITE.to_string()),
            Mode::PointInTime { id: "doc-1".to_string() },
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.where_clause, "0=1");
    }

    #[test]
    fn write_capable_permission_also_covers_a_read_check() {
        // "manage" allows both read and write; entailment means a
        // write-capable grant also satisfies a read-mode check.
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "manage": {
                                "allows": ["data-layer/read", "data-layer/write"],
                                "paths": [["creator", "caller"]]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();
        let write_cap = Capability {
            with: resource("document"),
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        };
        let alice = session("did:key:alice", vec![write_cap]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_ne!(sieve.where_clause, "0=1");
    }

    /// Isolates the `includes` closure from direct entailment: the caller
    /// holds *only* an app-permission grant for "manage" (`app/document.
    /// manage`, a flat, self-entailing-only ability string), never a
    /// platform-ability capability -- so "view" can only ever become
    /// applicable through `manage`'s `includes`, never through the direct
    /// route. This is what makes the write-mode assertion below a real
    /// regression test for the escalation Reviewer 1 found: closure used to
    /// widen unconditionally, so a write-mode check would previously have
    /// pulled in "view" (read-only) anyway.
    #[test]
    fn includes_closure_is_gated_by_the_included_permissions_own_allows() {
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {
                            "creator": {"target": "user", "join_column": "creator_uuid"},
                            "parent_dept": {"target": "department", "join_column": "owner_dept_id"}
                        },
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "caller"]]
                            },
                            "manage": {
                                "allows": ["data-layer/write"],
                                "includes": ["view"],
                                "paths": [["parent_dept", "caller"]]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"},
                    "department": {"table": "departments", "principal_column": "owner_did"}
                }
            }"#,
        )
        .unwrap();
        let app_cap = Capability {
            with: resource("document"),
            can: Ability("app/document.manage".to_string()),
            caveats: None,
        };
        let alice = session("did:key:alice", vec![app_cap]);

        // Write mode: "view" (allows: read) does not cover write, so
        // closure must NOT pull its path in -- the predicate is exactly
        // manage's own path, no OR.
        let write_sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_WRITE.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert!(
            !write_sieve.where_clause.contains(" OR "),
            "a write-mode check must not pull in a read-only included permission's path"
        );

        // Read mode: "manage" is still applicable (its own `allows: write`
        // entails read), and "view" (allows: read) *does* cover this
        // operation -- closure should widen to OR its path in too.
        let read_sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert!(
            read_sieve.where_clause.contains(" OR "),
            "a read-mode check should widen through includes when the included permission covers \
             it"
        );
    }

    #[test]
    fn collection_selector_grant_is_honored_and_scoped() {
        // Guards the §3.2 finding: a capability scoped to
        // `.../collection/document` must be admitted for `document` and
        // denied for an unrelated collection under the *same* service.
        let policy = single_hop_policy();
        let scoped = Capability {
            with: resource("document"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        };
        let alice = session("did:key:alice", vec![scoped]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_ne!(
            sieve.where_clause, "0=1",
            "the scoped grant must be admitted for its own collection"
        );
    }

    #[test]
    fn app_permission_route_admits_a_named_grant() {
        let policy = single_hop_policy();
        let named = Capability {
            with: resource("document"),
            can: Ability("app/document.view".to_string()),
            caveats: None,
        };
        let alice = session("did:key:alice", vec![named]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_ne!(sieve.where_clause, "0=1");
    }

    #[test]
    fn remote_relation_fails_closed_at_compile_time() {
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"owner": {"target": "employee", "service": "hr-svc"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["owner", "caller"]]}
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let err = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    #[test]
    fn point_in_time_mode_ands_the_id_predicate_last() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");
        insert_document(&conn, "doc-2", "u-alice");

        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::PointInTime { id: "doc-1".to_string() },
        )
        .unwrap()
        .unwrap();

        assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
    }

    #[test]
    fn adversarial_subject_did_is_bound_not_interpolated() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_policy();
        let attacker = session("attacker' OR '1'='1", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &attacker,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();

        // If this were string-interpolated, `OR '1'='1'` would make every
        // row visible. Bound as `?`, it is treated as an inert literal.
        assert!(run_sieve(&conn, "documents", &sieve).is_empty());
        assert!(
            sieve.where_clause.contains('?'),
            "the DID must be a bound placeholder, not inlined text"
        );
    }

    #[test]
    fn cls_masked_fields_union_policy_and_capability_deny_lists() {
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "user": {
                        "table": "users",
                        "principal_column": "did",
                        "permissions": {
                            "view_self": {
                                "allows": ["data-layer/read"],
                                "paths": [["caller"]],
                                "fields": {"deny": ["ssn"]}
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let cap = Capability {
            with: resource("user"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: Some(json!({"fields": {"deny": ["salary"]}})),
        };
        let alice = session("did:key:alice", vec![cap]);
        let sieve = compile_read(
            &policy,
            "user",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.masked_fields, vec!["salary".to_string(), "ssn".to_string()]);
    }

    #[test]
    fn where_caveats_are_collected_from_entitling_capabilities() {
        let policy = single_hop_policy();
        let cap = Capability {
            with: resource("document"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: Some(json!({"where": {"region": "EU"}})),
        };
        let alice = session("did:key:alice", vec![cap]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.where_caveats, vec![json!({"region": "EU"})]);
    }

    #[test]
    fn intersection_operator_requires_every_path_to_hold() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_user(&conn, "u-bob", "did:key:bob", None);
        let insert_reviewed_doc = |id: &str, creator: &str, reviewer: &str| {
            let payload = json!({"creator_uuid": creator, "reviewer_uuid": reviewer});
            conn.execute(
                "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
                (id, payload.to_string()),
            )
            .unwrap();
        };
        insert_reviewed_doc("doc-both", "u-alice", "u-alice");
        insert_reviewed_doc("doc-creator-only", "u-alice", "u-bob");
        insert_reviewed_doc("doc-reviewer-only", "u-bob", "u-alice");

        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {
                            "creator": {"target": "user", "join_column": "creator_uuid"},
                            "reviewer": {"target": "user", "join_column": "reviewer_uuid"}
                        },
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "operator": "intersection",
                                "paths": [["creator", "caller"], ["reviewer", "caller"]]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run_sieve(&conn, "documents", &sieve),
            vec!["doc-both"],
            "intersection requires alice to be both creator and reviewer"
        );
    }

    #[test]
    fn plain_two_hop_chain_prunes_through_both_joins() {
        // document -creator-> user -home_department-> department, with the
        // *department's owner* (not the creator) as the terminal -- a
        // non-recursive, non-fused 2-hop chain, distinct from the
        // recursive-fused case the other multi-hop test covers.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE departments (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE users (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE documents (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO departments (id, payload) VALUES ('dept-eng', ?1)",
            [json!({"owner_did": "did:key:carol"}).to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (id, payload) VALUES ('u-alice', ?1)",
            [json!({"dept_id": "dept-eng"}).to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
            [json!({"creator_uuid": "u-alice"}).to_string()],
        )
        .unwrap();

        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "home_department", "caller"]]
                            }
                        }
                    },
                    "user": {
                        "table": "users",
                        "relations": {"home_department": {"target": "department", "join_column": "dept_id"}}
                    },
                    "department": {"table": "departments", "principal_column": "owner_did"}
                }
            }"#,
        )
        .unwrap();

        let carol = session("did:key:carol", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &carol,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run_sieve(&conn, "documents", &sieve),
            vec!["doc-1"],
            "carol owns alice's home department, two joins away from the document"
        );

        let dave = session("did:key:dave", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &dave,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert!(run_sieve(&conn, "documents", &sieve).is_empty());
    }

    #[test]
    fn default_fallback_still_carries_the_entitling_capabilitys_caveats() {
        // Regression test (both reviews flagged this independently): when
        // no permission is directly/app-permission-applicable and access
        // comes only through `default`, the capability that satisfied
        // `holds_operation` must still contribute its caveats -- dropping
        // them would silently widen access beyond what the caveat allows.
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "default": "fallback",
                        "permissions": {
                            "fallback": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "caller"]]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();
        let caveat_cap = Capability {
            with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: Some(json!({"where": {"region": "EU"}, "fields": {"deny": ["ssn"]}})),
        };
        let alice = session("did:key:alice", vec![caveat_cap]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.where_caveats, vec![json!({"region": "EU"})]);
        assert_eq!(sieve.masked_fields, vec!["ssn".to_string()]);
    }

    #[test]
    fn default_permission_not_covering_operation_is_denied() {
        // Regression for the escalation this review found: `default` used
        // to apply regardless of whether its own permission's `allows`
        // covered the requested operation, so a caller holding *only* a
        // write capability could ride a read-only (or ability-less)
        // default permission's paths straight through a write-mode check.
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "default": "fallback",
                        "permissions": {
                            "fallback": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "caller"]]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();
        let write_cap = Capability {
            with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        };
        let alice = session("did:key:alice", vec![write_cap]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_WRITE.to_string()),
            Mode::PointInTime { id: "doc-1".to_string() },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            sieve.where_clause, "0=1",
            "a write-mode check must not fall through a read-only default permission"
        );
    }

    #[test]
    fn collection_lookup_is_case_insensitive_like_sqlite() {
        // Regression for the bypass this review found: SQLite resolves
        // table names case-insensitively, so a case-sensitive
        // `find_definition` let a caller spell the collection differently
        // than the policy and fall through to the unfiltered "no
        // definition" path against the *same* physical table.
        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "DOCUMENT",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        assert!(
            sieve.is_some(),
            "a differently-cased collection name must still resolve to the same definition, not \
             fall through to unfiltered"
        );
    }

    #[test]
    fn caveat_fields_deny_with_a_dotted_path_fails_closed() {
        // Regression: a runtime capability caveat can't be rejected at
        // policy parse time the way a policy `fields.deny` entry is, but a
        // dotted entry would silently mask nothing the same way -- fail
        // the compile instead of returning an unenforced mask.
        let policy = single_hop_policy();
        let cap = Capability {
            with: resource("document"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: Some(json!({"fields": {"deny": ["profile.ssn"]}})),
        };
        let alice = session("did:key:alice", vec![cap]);
        let err = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    // -- ADR-0017 §9 decision trace (M04B Slice B2 Phase 5) -----------------
    //
    // `compile_read` returns the same `DecisionTrace` it emits via `tracing`
    // on `CompiledSieve::trace`, so these assert on the struct directly
    // rather than capturing log output. `do_check_access`'s "rows not
    // reached" trace (the fourth deny reason -- known only after Mode A
    // actually executes the compiled predicate against a row) is covered in
    // `data_db`, the layer that runs that query.

    #[test]
    fn decision_trace_records_operation_not_admitted() {
        // No capabilities at all: nothing grants the operation on this
        // resource, distinct from holding a grant but reaching no row.
        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.trace.tier, 3);
        assert!(!sieve.trace.operation_admitted);
        assert!(sieve.trace.held.is_empty());
        assert!(
            sieve.trace.path_failed.as_deref().is_some_and(|r| r.contains("no held capability")),
            "path_failed was: {:?}",
            sieve.trace.path_failed
        );
    }

    #[test]
    fn decision_trace_records_strict_unknown_collection() {
        let policy =
            parse_and_validate(r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#)
                .unwrap();
        let alice = session("did:key:alice", vec![]);
        let sieve = compile_read(
            &policy,
            "unrelated_collection",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.trace.tier, 3);
        assert!(!sieve.trace.operation_admitted);
        assert!(
            sieve.trace.path_failed.as_deref().is_some_and(|r| r.contains("strict")),
            "path_failed was: {:?}",
            sieve.trace.path_failed
        );
    }

    #[test]
    fn decision_trace_records_claim_absent() {
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "user": {
                        "table": "users",
                        "principal_column": "did",
                        "permissions": {
                            "view_self": {
                                "allows": ["data-layer/read"],
                                "paths": [["caller"]],
                                "conditions": [{"column": "region", "claim": "region"}]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let alice = session("did:key:alice", vec![read_cap(Some("user"))]);
        let sieve = compile_read(
            &policy,
            "user",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert!(sieve.trace.operation_admitted);
        assert_eq!(sieve.trace.applicable_permissions, vec!["view_self".to_string()]);
        assert!(
            sieve.trace.path_failed.as_deref().is_some_and(|r| r.contains("claim absent")),
            "path_failed was: {:?}",
            sieve.trace.path_failed
        );
    }

    #[test]
    fn decision_trace_records_allow_with_no_path_failed() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert!(sieve.trace.operation_admitted);
        assert_eq!(sieve.trace.applicable_permissions, vec!["view".to_string()]);
        assert!(sieve.trace.path_failed.is_none());
        assert_eq!(sieve.trace.compiled_predicate.as_deref(), Some(sieve.where_clause.as_str()));
        assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
    }
}
