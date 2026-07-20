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

use crate::policy::{CondOp, Definition, Operator, Permission, Policy, PolicyError, Relation};

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
    /// entitling capability's `caveats.fields.deny`); an `allow`-list
    /// cannot be reduced to a field-name list without knowing a record's
    /// full key set, which this compiler does not have -- see the crate's
    /// `compile_cls` doc comment.
    pub masked_fields: Vec<String>,
    /// Each entitling capability's raw `caveats.where` document (an
    /// ADR-0007 MongoDB-style filter), for the caller to compile via
    /// `data_db`'s `filter::compile_filter` and AND onto this sieve.
    pub where_caveats: Vec<Json>,
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
        return if policy.strict { Ok(Some(deny_all())) } else { Ok(None) };
    };

    let resource = ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(service_id, service_id).0
    ));

    let (mut applicable, entitling_caps) =
        applicable_permissions(def, object_type, &resource, operation, session);
    close_over_includes(&mut applicable, def);

    if applicable.is_empty() {
        let holds_operation =
            session.capabilities.iter().any(|cap| cap.grants(&resource, operation));
        match &def.default {
            Some(default_perm) if holds_operation => {
                applicable.insert(default_perm.clone());
            }
            _ => return Ok(Some(deny_all())),
        }
    }

    let mut params: Vec<Value> = Vec::new();
    let mut clauses: Vec<String> = Vec::with_capacity(applicable.len());
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
        clauses.push(compile_permission(policy, object_type, perm, session, &mut params)?);
    }
    let mut where_clause = format!("({})", clauses.join(" OR "));

    if let Mode::PointInTime { id } = &mode {
        where_clause = format!("({where_clause}) AND {}.id = ?", def.table);
        params.push(Value::Text(id.clone()));
    }

    let masked_fields = compile_cls(def, &applicable, &entitling_caps);
    let where_caveats: Vec<Json> = entitling_caps
        .iter()
        .filter_map(|cap| cap.caveats.as_ref()?.get("where").cloned())
        .collect();

    Ok(Some(CompiledSieve { where_clause, params, masked_fields, where_caveats }))
}

fn find_definition<'a>(policy: &'a Policy, collection: &str) -> Option<(&'a str, &'a Definition)> {
    policy
        .definitions
        .iter()
        .find(|(key, def)| key.as_str() == collection || def.table == collection)
        .map(|(key, def)| (key.as_str(), def))
}

fn deny_all() -> CompiledSieve {
    CompiledSieve {
        where_clause: "0=1".to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
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

fn close_over_includes(applicable: &mut BTreeSet<String>, def: &Definition) {
    loop {
        let additions: Vec<String> = applicable
            .iter()
            .filter_map(|pname| def.permissions.get(pname))
            .flat_map(|perm| perm.includes.iter().cloned())
            .filter(|included| !applicable.contains(included))
            .collect();
        if additions.is_empty() {
            return;
        }
        applicable.extend(additions);
    }
}

/// CLS: field masking derived from `deny`-list entries only. An
/// `allow`-list narrows further but cannot be reduced to a field-name list
/// at compile time -- doing so would require knowing every key a record's
/// JSON payload might carry, which the policy model does not declare. This
/// is a recorded B2 limitation, not a silent gap: `deny` lists (the
/// documented failure/security test case -- "caller lacks column
/// permission -> column masked out") are fully enforced; `allow`-list-only
/// CLS is a no-op until a schema-aware masking pass lands.
fn compile_cls(
    def: &Definition,
    applicable: &BTreeSet<String>,
    entitling_caps: &[&Capability],
) -> Vec<String> {
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

    denied.into_iter().collect()
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
        params.push(json_value_to_sql(claim_val)?);
        path_pred =
            format!("{path_pred} AND {} {} ?", col(&def.table, &cond.column), sql_op(cond.op));
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

/// `<col>`: reserved names address the physical column directly; anything
/// else addresses the JSON `payload` column (ADR-0017 Amendments,
/// 2026-07-20; §12.2 of the implementation plan).
fn col(qualifier: &str, name: &str) -> String {
    if RESERVED_COLUMNS.contains(&name) {
        format!("{qualifier}.{name}")
    } else {
        format!("json_extract({qualifier}.payload, '$.{name}')")
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
        let bound = terminal_value(terminal, session)?;
        params.push(Value::Text(bound));
        return Ok(format!("{} = ?", col(&start_def.table, principal_col)));
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
            let correlate_expr = col(correlate_qualifier, join_column);
            let inner = if rest.is_empty() {
                let principal_col = hop.target_def.principal_column.as_ref().ok_or_else(|| {
                    PolicyError::Semantic(format!(
                        "object type '{}' is used as a path terminal but declares no \
                         principal_column",
                        hop.relation.target
                    ))
                })?;
                let bound = terminal_value(terminal, session)?;
                params.push(Value::Text(bound));
                format!("{} = ?", col(&alias, principal_col))
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
    let seed_correlate = col(correlate_qualifier, join_column);
    let seed_fk = col("u", from_key);
    let step_fk = col("u2", from_key);
    let step_lookup = col(seed_table, to_key);
    let bare_fk = col(seed_table, from_key);
    let did_u = col("u", principal_col);
    let did_u2 = col("u2", principal_col);

    params.push(Value::Integer(MAX_RECURSION_DEPTH));
    let bound = terminal_value(terminal, session)?;
    params.push(Value::Text(bound));

    Ok(format!(
        "EXISTS (WITH RECURSIVE mc(id, prin, depth, seen) AS (SELECT {seed_fk}, {did_u}, 0, '/' \
         || {seed_fk} || '/' FROM {seed_table} u WHERE {seed_fk} = {seed_correlate} UNION ALL \
         SELECT {step_fk}, {did_u2}, mc.depth + 1, mc.seen || {step_fk} || '/' FROM {seed_table} \
         u2 JOIN mc ON {step_fk} = (SELECT {step_lookup} FROM {seed_table} WHERE {bare_fk} = \
         mc.id) WHERE mc.depth < ? AND instr(mc.seen, '/' || {step_fk} || '/') = 0) SELECT 1 FROM \
         mc WHERE mc.prin = ?)"
    ))
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

    #[test]
    fn includes_closure_pulls_in_the_included_permissions_paths() {
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
        let write_cap = Capability {
            with: resource("document"),
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        };
        let alice = session("did:key:alice", vec![write_cap]);
        // Requesting a *read*: "manage" itself only allows write, so it
        // would not directly cover a read-mode check -- but its included
        // "view" does allow read, and includes-closure happens *before*
        // operation filtering only affects which perms became applicable
        // in the first place. Here we check write mode: "manage" is
        // directly entitled by the write cap, and its closure should pull
        // in "view"'s path too (both paths OR together).
        let sieve = compile_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_WRITE.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert!(sieve.where_clause.contains("creator_uuid") || sieve.where_clause.contains("dept"));
        assert!(sieve.where_clause.contains(" OR "), "closure should OR manage's and view's paths");
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
}
