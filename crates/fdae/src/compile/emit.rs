//! SQL AST emission for ReBAC paths, permissions, and structural queries.

use rusqlite::types::Value;
use serde_json::Value as Json;
use syneroym_ucan::SessionContext;

use super::types::{FetchCtx, MAX_RECURSION_DEPTH, RESERVED_COLUMNS, StructuralQuery};
use crate::policy::{CondOp, Definition, Operator, Permission, Policy, PolicyError, Relation};

/// Matches case-insensitively (ASCII fold), mirroring SQLite's own
/// identifier resolution: an unquoted table name is case-insensitive, so a
/// case-sensitive lookup here would let a caller name the same physical
/// table under a spelling this policy doesn't recognize and fall through to
/// the unfiltered "no definition" path while the query still hits the real,
/// policy-governed table.
pub(crate) fn find_definition<'a>(
    policy: &'a Policy,
    collection: &str,
) -> Option<(&'a str, &'a Definition)> {
    policy
        .definitions
        .iter()
        .find(|(key, def)| {
            key.as_str().eq_ignore_ascii_case(collection)
                || def.table.eq_ignore_ascii_case(collection)
        })
        .map(|(key, def)| (key.as_str(), def))
}

/// The physical table backing the definition matching `collection`
/// (case-insensitively, by key or table -- same rule as `find_definition`),
/// or `None` if no definition matches. The native `resolve-relation` needs
/// this for two reasons: (1) as a **hard pre-check** -- unlike an ordinary
/// `compile_read` call, where "no definition" correctly means "the grant
/// layer already admitted this read, run unfiltered" (`compile_read`'s
/// `Ok(None)`), `resolve-relation` answers a cross-service relationship
/// question with *no* backing grant-layer admission at all, so a `relation`
/// name matching no definition must deny, never fall through to an
/// unfiltered dump; and (2) because `ServiceStore::query` (unlike
/// `compile_read`'s own permissive key-or-table matching) addresses a
/// collection by its **literal physical table name** -- a caller passing a
/// policy's *definition key* (e.g. `RemoteFetch.relation`, which is a
/// local relation name, not necessarily a table name) would otherwise hit
/// `collection-not-found` even though the definition itself resolves fine.
#[must_use]
pub fn definition_table<'a>(policy: &'a Policy, collection: &str) -> Option<&'a str> {
    find_definition(policy, collection).map(|(_, def)| def.table.as_str())
}

/// Whether *any* permission on the definition backing `collection` opts into
/// the stage-4 after-step (ADR-0017 §7, `Permission.authorize_rows`).
/// Coarser than a compiled sieve's `abac_permissions` (which knows which
/// permissions this caller actually selected) and deliberately so: the one
/// caller is the native `resolve-relation` (both its sieve-backed branch
/// and its `resolve_structural` fallback), which has no compiled sieve for
/// the requesting anchor and must fail closed rather than let a remote
/// caller route around this node's after-step. An unknown `collection`
/// returns `false` -- `find_definition` returning `None` already means "no
/// definition to gate," identical to `definition_table`'s own behavior.
#[must_use]
pub fn definition_has_abac(policy: &Policy, collection: &str) -> bool {
    find_definition(policy, collection)
        .is_some_and(|(_, def)| def.permissions.values().any(|p| p.authorize_rows))
}

/// The raw `<principal_column> = ?` predicate for a
/// definition that has explicitly opted into
/// [`Definition::resolvable_without_capability`], bypassing the capability/
/// grant-intersection gate [`compile_read`] requires. The caller (native
/// `resolve-relation`) uses this only when the requesting
/// anchor holds zero capabilities scoped to the target service -- see that
/// field's doc comment for the authorization-model tradeoff; this function
/// itself performs no capability check, since it has no `SessionContext` to
/// check one against.
///
/// Reuses the same reserved-column-vs-JSON-payload addressing as every
/// other predicate this compiler emits (mirrors `col`), so a
/// `principal_column` of `"creator_id"` resolves to the physical column,
/// not `json_extract(payload, '$.creator_id')`.
///
/// Returns `Ok(None)` when `relation` names no definition, or a definition
/// that hasn't opted in -- the caller treats that identically to "not
/// resolvable," i.e. deny (an empty id-set), never an error a caller could
/// misread as "resolved to nothing found" vs. "not permitted to ask".
pub fn resolve_structural(
    policy: &Policy,
    relation: &str,
    principal: &str,
) -> Result<Option<StructuralQuery>, PolicyError> {
    let Some((_, def)) = find_definition(policy, relation) else {
        return Ok(None);
    };
    if !def.resolvable_without_capability {
        return Ok(None);
    }
    let principal_col = def.principal_column.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!(
            "definition '{relation}' declares resolvable_without_capability but no \
             principal_column, so it cannot be structurally resolved"
        ))
    })?;
    let mut params: Vec<String> = Vec::new();
    let where_clause = if RESERVED_COLUMNS.contains(&principal_col.as_str()) {
        format!("{}.{principal_col} = ?", def.table)
    } else {
        params.push(format!("$.{principal_col}"));
        format!("json_extract({}.payload, ?) = ?", def.table)
    };
    params.push(principal.to_string());
    Ok(Some(StructuralQuery { table: def.table.clone(), where_clause, params }))
}

pub(crate) fn compile_permission(
    policy: &Policy,
    object_type: &str,
    perm: &Permission,
    session: &SessionContext,
    params: &mut Vec<Value>,
    fetches: &mut FetchCtx,
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
            clauses.push(compile_path(policy, object_type, path, session, params, fetches)?);
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

/// `anchor` resolves to the original principal the chain acts for
/// (`SessionContext.anchor_did`), not the immediate presenting caller --
/// the confused-deputy defense (ADR-0015 A5, amended). A direct call has no
/// distinct anchor (`anchor_did == None`), and that falls back to
/// `subject_did` -- a direct caller *is* the anchor -- rather than denying
/// the policy outright.
fn terminal_value(terminal: &str, session: &SessionContext) -> Result<String, PolicyError> {
    match terminal {
        "caller" => Ok(session.subject_did.clone()),
        "anchor" => Ok(session.anchor_did.clone().unwrap_or_else(|| session.subject_did.clone())),
        other => Err(PolicyError::Semantic(format!("unknown path terminal '{other}'"))),
    }
}

/// One resolved, validated hop of a path's relation walk. `target_def` is
/// `None` for a remote hop (`relation.service.is_some()`): the target object
/// type lives on another service's policy, not locally resolvable (mirrors
/// `policy::validate_relations`' own skip). A remote hop is always the last
/// element of `hops` -- `resolve_hops` rejects one anywhere else -- so
/// `target_def` is `Some` everywhere `emit_chain`'s generic (non-terminal)
/// branch reads it.
struct Hop<'a> {
    name: &'a str,
    relation: &'a Relation,
    target_def: Option<&'a Definition>,
}

/// Resolves and validates every non-terminal segment of a path in order,
/// failing closed on a recursive relation anywhere but the last hop, or a
/// remote (cross-service) relation anywhere but the last hop (a remote
/// hop's fetched id-set is checked directly against the *preceding* local
/// row, mirroring the last-local-hop terminal check -- there is no local
/// table to keep joining through past it).
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
        let is_last = i == rel_names.len() - 1;
        if relation.service.is_some() && !is_last {
            return Err(PolicyError::Semantic(format!(
                "relation '{rel_name}' is remote (service: '{}'); a remote relation must be the \
                 last hop before the path terminal",
                relation.service.as_deref().unwrap_or_default()
            )));
        }
        if relation.recursive && !is_last {
            return Err(PolicyError::Semantic(format!(
                "recursive relation '{rel_name}' must be the last hop before the path terminal"
            )));
        }
        let target_def = if relation.service.is_some() {
            None
        } else {
            Some(get_def(policy, &relation.target)?)
        };
        hops.push(Hop { name: rel_name.as_str(), relation, target_def });
        if relation.service.is_none() {
            current_type = &relation.target;
        }
    }
    Ok(hops)
}

/// Walks a path (`[relation..., terminal]`) into a correlated `EXISTS`
/// subquery, a single `EXISTS (WITH RECURSIVE ...)` block when the last
/// relation is recursive, or a direct `IN (...)` membership check against a
/// not-yet-fetched id-set when the last relation is remote.
fn compile_path(
    policy: &Policy,
    start_type: &str,
    path: &[String],
    session: &SessionContext,
    params: &mut Vec<Value>,
    fetches: &mut FetchCtx,
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
    emit_chain(&hops, &start_def.table, terminal, session, params, &mut alias_idx, fetches)
}

/// Emits nested `EXISTS` for a chain of local-join hops, fusing the last two
/// hops into a single `EXISTS (WITH RECURSIVE ...)` block when the final hop
/// is recursive (ADR-0017), or emitting a direct `IN (...)`
/// membership check when the final hop is remote.
fn emit_chain(
    hops: &[Hop],
    correlate_qualifier: &str,
    terminal: &str,
    session: &SessionContext,
    params: &mut Vec<Value>,
    alias_idx: &mut usize,
    fetches: &mut FetchCtx,
) -> Result<String, PolicyError> {
    match hops {
        [] => Err(PolicyError::Semantic("internal: emit_chain called with no hops".to_string())),
        [leading, recursive] if recursive.relation.recursive => {
            emit_fused_recursive(leading, recursive, correlate_qualifier, terminal, session, params)
        }
        [remote] if remote.relation.service.is_some() => {
            emit_remote_terminal(remote, correlate_qualifier, session, params, fetches)
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
            // Structurally guaranteed by `resolve_hops`: only the *last*
            // hop of a chain can be remote (`target_def: None`), and a
            // remote last hop is caught by the `[remote] if ...` arm above
            // before reaching this generic one -- so `hop` here is always
            // local. Fail closed rather than panic on the defensive case.
            let target_def = hop.target_def.ok_or_else(|| {
                PolicyError::Semantic(format!(
                    "internal: non-terminal hop '{}' has no local target definition",
                    hop.name
                ))
            })?;
            let inner = if rest.is_empty() {
                let principal_col = target_def.principal_column.as_ref().ok_or_else(|| {
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
                emit_chain(rest, &alias, terminal, session, params, alias_idx, fetches)?
            };
            Ok(format!(
                "EXISTS (SELECT 1 FROM {} AS {alias} WHERE {alias}.id = {correlate_expr} AND \
                 {inner})",
                target_def.table
            ))
        }
    }
}

/// The terminal hop of a path whose last relation is remote:
/// unlike a local hop, there is no local `target_table` to `EXISTS`-join
/// against (the object lives on another service). Instead, the current
/// row's own `join_column` value will be checked for membership in the
/// id-set [`finalize`] later binds from the fetched relationship proof --
/// this function only registers the fetch and returns a bare marker token
/// standing in for the *entire* predicate (never resolved here; see
/// [`PendingMarker::correlate_expr`] for why the marker can't be scoped to
/// just the eventual `IN (...)` list).
///
/// The fetch's principal is always the **anchor**
/// (`session.anchor_did`, falling back to `subject_did` for a direct
/// caller), never the presenting caller -- the confused-deputy defense
/// (ADR-0015 A5) -- regardless of whether this path's own declared terminal
/// word is `caller` or `anchor`: a remote node must always be asked "what
/// can the original principal reach," never "what can the proxying service
/// reach."
fn emit_remote_terminal(
    hop: &Hop,
    correlate_qualifier: &str,
    session: &SessionContext,
    params: &mut Vec<Value>,
    fetches: &mut FetchCtx,
) -> Result<String, PolicyError> {
    let join_column = hop.relation.join_column.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!(
            "relation '{}' is remote but declares no join_column -- a remote relation needs one \
             to identify which local column is checked against the fetched id-set",
            hop.name
        ))
    })?;
    let correlate_expr = col(correlate_qualifier, join_column, params);
    let service = hop.relation.service.clone().ok_or_else(|| {
        PolicyError::Semantic(format!("internal: relation '{}' is not remote", hop.name))
    })?;
    let expected_asserter_did = hop.relation.expected_asserter_did.clone().ok_or_else(|| {
        PolicyError::Semantic(format!(
            "internal: relation '{}' is remote but declares no expected_asserter_did",
            hop.name
        ))
    })?;
    let principal_did = session.anchor_did.clone().unwrap_or_else(|| session.subject_did.clone());
    // `hop.relation.target`, not `hop.name`: the wire `relation` names the
    // remote **object type**, which the remote's own `definitions:` map
    // resolves it against (`definition_table`/`resolve_structural`) -- the
    // local edge name (`hop.name`, e.g. "owner") lives in a different
    // namespace the remote has no reason to recognize.
    let token = fetches.register(
        service,
        hop.relation.target.clone(),
        principal_did,
        expected_asserter_did,
        correlate_expr,
        params.len(),
    )?;
    // The marker stands for the whole predicate (see `PendingMarker::
    // correlate_expr`), not just an `IN (...)` list -- `finalize` needs to
    // be able to substitute a literal `0` for an empty id-set.
    Ok(token)
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
    // A recursive relation is never remote (schema-enforced mutual
    // exclusivity, `policy::validate_relation_shape`), so `target_def` is
    // always `Some` here; fail closed rather than panic on the defensive
    // case, matching `emit_chain`'s own guard.
    let target_def = recursive.target_def.ok_or_else(|| {
        PolicyError::Semantic(format!(
            "internal: recursive relation '{}' has no local target definition",
            recursive.name
        ))
    })?;
    let principal_col = target_def.principal_column.as_ref().ok_or_else(|| {
        PolicyError::Semantic(format!(
            "object type '{}' is used as a path terminal but declares no principal_column",
            recursive.relation.target
        ))
    })?;

    let seed_table = &target_def.table;

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
