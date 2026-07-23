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

/// Upper bound on a single remote fetch's returned id-set, bound as the
/// `IN (...)` list's cardinality (Slice B3 plan §5, fan-out containment). An
/// unbounded id-set would let a misbehaving or compromised remote blow up
/// the local query's `IN` list arbitrarily. Matches `data_db`'s existing
/// per-page query cap.
pub const MAX_FETCH_IDS: usize = 1000;

/// Correlation key into a [`ReadPlan`]'s `fetches`: one per distinct
/// `(service, relation)` pair a policy's selected paths need (deduped --
/// multiple hops naming the same remote relation share one fetch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchSlot(usize);

/// One remote relationship-proof fetch [`plan_read`] needs before its sieve
/// can be finalized (ADR-0017 §6 / pipeline stage 2, Slice B3). The
/// orchestration that actually performs the fetch (resolving `service` to a
/// DID, issuing the proxy call, enforcing the timeout) lives outside this
/// crate (`crates/fdae` stays proxy-free, plan §1.1); this struct is only
/// the *request* shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFetch {
    /// Logical service name from `Relation.service`, resolved to a DID by
    /// the caller (this crate stays free of the app-context registry).
    pub service: String,
    /// The relation name (the key in `Definition.relations`), passed through
    /// verbatim as the remote's own relationship-resolution parameter.
    pub relation: String,
    /// The principal the remote must evaluate for -- always the **anchor**
    /// (`session.anchor_did`, falling back to `subject_did` for a direct
    /// caller), never the presenting/proxying caller. The confused-deputy
    /// defense (ADR-0015 A5): this holds regardless of whether the path's
    /// own declared terminal word is `caller` or `anchor`.
    pub principal_did: String,
    pub slot: FetchSlot,
}

/// One fetched result, matched back to its [`RemoteFetch`] by `slot`.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub slot: FetchSlot,
    pub ids: Vec<String>,
}

/// One not-yet-resolved position in a [`PendingSieve`]'s SQL text: a unique
/// text token standing in for a remote fetch's eventual `IN (?, ?, ...)`
/// list, plus where in the flat `params` sequence that list's bound values
/// belong once known.
#[derive(Debug, Clone)]
struct PendingMarker {
    slot: FetchSlot,
    /// Unique per *occurrence* (not per slot -- the same slot can appear at
    /// multiple text positions when several OR'd permission paths reach the
    /// same remote relation), so `finalize` can replace each occurrence
    /// independently via a single, unambiguous `replacen`.
    token: String,
    /// Index into `PendingSieve.params` where this marker's id-set values
    /// are inserted -- i.e. `params.len()` at the moment this marker was
    /// emitted during path compilation, so binding order matches the `?`
    /// occurrences' left-to-right text order.
    params_index: usize,
}

/// A [`CompiledSieve`] that still needs one or more remote relationship
/// fetches before it can run -- the "plan" half of the two-phase compile
/// (Slice B3 plan §1.1). Opaque outside this module; the only thing a
/// caller does with one is pass it to [`finalize`] alongside the fetched
/// [`FetchResult`]s.
#[derive(Debug, Clone)]
pub struct PendingSieve {
    where_clause: String,
    params: Vec<Value>,
    masked_fields: Vec<String>,
    where_caveats: Vec<Json>,
    trace: DecisionTrace,
    markers: Vec<PendingMarker>,
}

/// The result of [`plan_read`]: either a fully-compiled local sieve (the B2
/// case -- `fetches` empty, `pending` `None`, `local` mirrors
/// `compile_read`'s `Option<CompiledSieve>` exactly), or a
/// [`PendingSieve`] plus the [`RemoteFetch`]es it's waiting on.
#[derive(Debug, Clone)]
pub struct ReadPlan {
    /// `Some` iff `fetches` is empty and a definition was found (or the
    /// policy is unfiltered for this collection, in which case this is
    /// `None` too -- identical three-way meaning to `compile_read`'s
    /// `Ok(None)` / `Ok(Some(_))`, disambiguated from the "needs fetches"
    /// case by `fetches` itself).
    pub local: Option<CompiledSieve>,
    pub fetches: Vec<RemoteFetch>,
    /// `Some` iff `fetches` is non-empty.
    pub pending: Option<PendingSieve>,
}

/// Mutable state threaded through path compilation alongside `params`: the
/// distinct remote fetches this compilation has discovered so far (deduped
/// per `(service, relation)`) and the text markers standing in for their
/// eventual `IN (...)` lists.
#[derive(Debug, Default)]
struct FetchCtx {
    fetches: Vec<RemoteFetch>,
    markers: Vec<PendingMarker>,
    next_marker: usize,
}

impl FetchCtx {
    /// Registers one occurrence of a remote relation reached at the current
    /// `params_index`, deduping the underlying fetch by `(service,
    /// relation)` but always emitting a fresh, unique text token for this
    /// occurrence. Returns the token to splice into the SQL text as
    /// `IN (<token>)`.
    fn register(
        &mut self,
        service: String,
        relation: String,
        principal_did: String,
        params_index: usize,
    ) -> String {
        let slot =
            match self.fetches.iter().find(|f| f.service == service && f.relation == relation) {
                Some(existing) => existing.slot,
                None => {
                    let slot = FetchSlot(self.fetches.len());
                    self.fetches.push(RemoteFetch { service, relation, principal_did, slot });
                    slot
                }
            };
        let token = format!("@@FDAE_FETCH_{}_{}@@", slot.0, self.next_marker);
        self.next_marker += 1;
        self.markers.push(PendingMarker { slot, token: token.clone(), params_index });
        token
    }
}

/// Binds each [`PendingSieve`] marker's fetched id-set into its `IN (...)`
/// text position and the corresponding `?` values into `params`, producing
/// a finished [`CompiledSieve`] ready to run exactly like a fully-local one.
/// Fails closed (never silently drops a fetch) if `results` is missing a
/// slot `pending` needs, or if a fetch's id-set exceeds [`MAX_FETCH_IDS`].
/// An empty id-set compiles to `IN (NULL)` (always false, valid SQL) rather
/// than the invalid `IN ()`.
pub fn finalize(
    pending: PendingSieve,
    results: &[FetchResult],
) -> Result<CompiledSieve, PolicyError> {
    let PendingSieve { mut where_clause, mut params, masked_fields, where_caveats, trace, markers } =
        pending;

    // Markers are inserted in ascending `params_index` order so `shift`
    // (the running count of ids already spliced in) correctly accounts for
    // every earlier insertion's effect on later positions -- `Vec::insert`
    // shifts everything at/after its index rightward, exactly matching how
    // later `?` occurrences in the text sit after earlier ones.
    let mut ordered = markers;
    ordered.sort_by_key(|m| m.params_index);

    let mut shift = 0usize;
    for marker in &ordered {
        let ids =
            results.iter().find(|r| r.slot == marker.slot).map(|r| r.ids.as_slice()).ok_or_else(
                || {
                    PolicyError::Semantic(format!(
                        "finalize: missing fetch result for slot {}",
                        marker.slot.0
                    ))
                },
            )?;
        if ids.len() > MAX_FETCH_IDS {
            return Err(PolicyError::Semantic(format!(
                "remote fetch returned {} ids, exceeding the {MAX_FETCH_IDS} cap",
                ids.len()
            )));
        }
        let replacement =
            if ids.is_empty() { "NULL".to_string() } else { vec!["?"; ids.len()].join(", ") };
        where_clause = where_clause.replacen(&marker.token, &replacement, 1);
        if !ids.is_empty() {
            let insert_at = marker.params_index + shift;
            for (i, id) in ids.iter().enumerate() {
                params.insert(insert_at + i, Value::Text(id.clone()));
            }
            shift += ids.len();
        }
    }

    Ok(CompiledSieve { where_clause, params, masked_fields, where_caveats, trace })
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
/// - `Err(PolicyError)` -- malformed/unsupported input, **or the policy's
///   selected paths require a remote relationship fetch** (B3 pipeline stage
///   2): this function is the synchronous/local-only entry point (unchanged
///   from B2) and cannot itself resolve one -- a caller that needs to may call
///   [`plan_read`] directly. Either way the caller must treat this as deny,
///   never as unfiltered access.
pub fn compile_read(
    policy: &Policy,
    collection: &str,
    session: &SessionContext,
    service_id: &str,
    operation: &Ability,
    mode: Mode,
) -> Result<Option<CompiledSieve>, PolicyError> {
    let plan = plan_read(policy, collection, session, service_id, operation, mode)?;
    if !plan.fetches.is_empty() {
        return Err(PolicyError::Semantic(format!(
            "policy requires {} remote relationship fetch(es) to answer this read -- compile_read \
             is local-only; use plan_read/finalize (B3 pipeline stage 2) instead",
            plan.fetches.len()
        )));
    }
    Ok(plan.local)
}

/// The two-phase counterpart of [`compile_read`] (Slice B3 plan §1.1): same
/// inputs, but when a selected permission path needs a remote relation
/// (`Relation.service.is_some()`), it is compiled with a placeholder `IN`
/// predicate and recorded as a [`RemoteFetch`] instead of failing closed.
/// `compile_read` is exactly `plan_read` plus "no remote fetches were
/// needed, or error" -- see its doc comment for the three-way `local`
/// meaning this shares.
pub fn plan_read(
    policy: &Policy,
    collection: &str,
    session: &SessionContext,
    service_id: &str,
    operation: &Ability,
    mode: Mode,
) -> Result<ReadPlan, PolicyError> {
    let Some((object_type, def)) = find_definition(policy, collection) else {
        if !policy.strict {
            return Ok(ReadPlan { local: None, fetches: Vec::new(), pending: None });
        }
        let trace = DecisionTrace {
            tier: 3,
            collection: collection.to_string(),
            service_id: service_id.to_string(),
            subject_did: session.subject_did.clone(),
            operation_admitted: false,
            path_failed: Some(format!(
                "no policy definition matches collection '{collection}' and the policy is strict"
            )),
            compiled_predicate: Some("0=1".to_string()),
            ..DecisionTrace::default()
        };
        trace.emit();
        return Ok(ReadPlan {
            local: Some(CompiledSieve { trace, ..deny_all() }),
            fetches: Vec::new(),
            pending: None,
        });
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
                    collection: collection.to_string(),
                    service_id: service_id.to_string(),
                    subject_did: session.subject_did.clone(),
                    held: describe_caps(&holding_caps),
                    operation_admitted,
                    path_failed: Some(path_failed),
                    compiled_predicate: Some("0=1".to_string()),
                    ..DecisionTrace::default()
                };
                trace.emit();
                return Ok(ReadPlan {
                    local: Some(CompiledSieve { trace, ..deny_all() }),
                    fetches: Vec::new(),
                    pending: None,
                });
            }
        }
    }

    let mut params: Vec<Value> = Vec::new();
    let mut fetch_ctx = FetchCtx::default();
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
        let clause =
            compile_permission(policy, object_type, perm, session, &mut params, &mut fetch_ctx)?;
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
    let mut where_clause = format!("({})", clauses.join(" OR "));

    if let Mode::PointInTime { id } = &mode {
        where_clause = format!("({where_clause}) AND {}.id = ?", def.table);
        params.push(Value::Text(id.clone()));
    }

    let masked_fields = compile_cls(def, &applicable, &entitling_caps)?;
    let where_caveats: Vec<Json> = entitling_caps
        .iter()
        .filter_map(|cap| cap.caveats.as_ref()?.get("where").cloned())
        .collect();

    // A deny is knowable at compile time only when *every* applicable
    // permission's own clause denied via the claim-absent fail-closed path
    // -- checking the *joined* string instead (e.g. `base_where_clause ==
    // "(0=1)"`) would miss a multi-permission deny: two "0=1" clauses OR
    // together as "(0=1 OR 0=1)", never as the literal "(0=1)" a naive
    // string match expects. `claim_absent_for` only ever grows to
    // `applicable.len()` (one push per clause, at most), so equality here
    // is exactly "every clause was 0=1".
    let path_failed = (claim_absent_for.len() == applicable.len()).then(|| {
        format!("condition claim absent for permission(s): {}", claim_absent_for.join(", "))
    });
    // Field names and caveat-filter *keys* are policy/grant shape, safe to
    // log; the caveat filter's *values* (DIDs, tenant ids, row predicates)
    // are not, so only their keys are recorded here.
    let caveats_applied: Vec<String> = masked_fields
        .iter()
        .map(|f| format!("fields.deny:{f}"))
        .chain(where_caveats.iter().map(|c| format!("where.keys:[{}]", json_object_keys(c))))
        .collect();
    let trace = DecisionTrace {
        tier: 3,
        collection: collection.to_string(),
        service_id: service_id.to_string(),
        subject_did: session.subject_did.clone(),
        anchor_did: session.anchor_did.clone(),
        held: describe_caps(&entitling_caps),
        operation_admitted: true,
        applicable_permissions: applicable.iter().cloned().collect(),
        compiled_predicate: Some(where_clause.clone()),
        rows_reached: None,
        path_failed,
        caveats_applied,
    };
    trace.emit();

    if fetch_ctx.fetches.is_empty() {
        Ok(ReadPlan {
            local: Some(CompiledSieve {
                where_clause,
                params,
                masked_fields,
                where_caveats,
                trace,
            }),
            fetches: Vec::new(),
            pending: None,
        })
    } else {
        Ok(ReadPlan {
            local: None,
            fetches: fetch_ctx.fetches,
            pending: Some(PendingSieve {
                where_clause,
                params,
                masked_fields,
                where_caveats,
                trace,
                markers: fetch_ctx.markers,
            }),
        })
    }
}

/// Comma-joined top-level keys of a caveat `where` document, for the
/// decision trace -- a summary of *which* fields a caveat filters on
/// without echoing the filter's bound values.
fn json_object_keys(doc: &Json) -> String {
    match doc {
        Json::Object(map) => map.keys().cloned().collect::<Vec<_>>().join(","),
        _ => String::new(),
    }
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

/// The physical table backing the definition matching `collection`
/// (case-insensitively, by key or table -- same rule as `find_definition`),
/// or `None` if no definition matches. B3's native `resolve-relation` needs
/// this for two reasons: (1) as a **hard pre-check** -- unlike an ordinary
/// `compile_read` call, where "no definition" correctly means "the grant
/// layer already admitted this read, run unfiltered" (`compile_read`'s
/// `Ok(None)`), `resolve-relation` answers a cross-service relationship
/// question with *no* backing grant-layer admission at all, so a `relation`
/// name matching no definition must deny, never fall through to an
/// unfiltered dump; and (2) because `ServiceStore::query` (unlike
/// `compile_read`'s own permissive key-or-table matching) addresses a
/// collection by its **literal physical table name** -- a caller passing a
/// policy's *definition key* (e.g. B3's `RemoteFetch.relation`, which is a
/// local relation name, not necessarily a table name) would otherwise hit
/// `collection-not-found` even though the definition itself resolves fine.
#[must_use]
pub fn definition_table<'a>(policy: &'a Policy, collection: &str) -> Option<&'a str> {
    find_definition(policy, collection).map(|(_, def)| def.table.as_str())
}

/// A raw `<principal_column> = ?` predicate for [`resolve_structural`] (B3
/// D-B3-3, the A2 fallback): no `WHERE EXISTS`, no capability check, no
/// `CompiledSieve` -- just enough to run `SELECT id FROM <table> WHERE
/// <where_clause>` against the caller's own connection.
#[derive(Debug, Clone)]
pub struct StructuralQuery {
    pub table: String,
    pub where_clause: String,
    /// Bound values, in binding order -- always text (a JSON path or the
    /// principal DID itself), unlike `CompiledSieve::params` which can carry
    /// any scalar claim value.
    pub params: Vec<String>,
}

/// B3 D-B3-3 (A2): the raw `<principal_column> = ?` predicate for a
/// definition that has explicitly opted into
/// [`Definition::resolvable_without_capability`], bypassing the capability/
/// grant-intersection gate [`compile_read`] requires. The caller (native
/// `resolve-relation`, Slice B3 Phase 3) uses this only when the requesting
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
/// remote (cross-service) relation anywhere but the last hop (B3: a remote
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
/// relation is recursive (§3.4), or -- when the last relation is remote (B3)
/// -- a direct `IN (...)` membership check against a not-yet-fetched id-set.
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
/// is recursive (§3.4, ADR-0017), or emitting a direct `IN (...)`
/// membership check when the final hop is remote (B3, §3).
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

/// The terminal hop of a path whose last relation is remote (B3, plan §3):
/// unlike a local hop, there is no local `target_table` to `EXISTS`-join
/// against (the object lives on another service). Instead, the current
/// row's own `join_column` value is checked for membership in the id-set
/// [`finalize`] later binds from the fetched relationship proof -- a plain
/// `{col_expr} IN (<fetch marker>)`, registered against `fetches` rather
/// than resolved here.
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
    let principal_did = session.anchor_did.clone().unwrap_or_else(|| session.subject_did.clone());
    let token = fetches.register(service, hop.name.to_string(), principal_did, params.len());
    Ok(format!("{correlate_expr} IN ({token})"))
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
            anchor_did: None,
            capabilities,
            claims: Map::new(),
            verified_at_secs: 0,
        }
    }

    fn session_with_anchor(
        subject_did: &str,
        anchor_did: &str,
        capabilities: Vec<Capability>,
    ) -> SessionContext {
        SessionContext {
            subject_did: subject_did.to_string(),
            anchor_did: Some(anchor_did.to_string()),
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

    fn single_hop_anchor_policy() -> Policy {
        parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "anchor"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap()
    }

    /// ADR-0015 A5 (amended): `anchor` filters by the original principal a
    /// proxying caller acts for, not the presenting caller itself -- the
    /// confused-deputy defense. A caller presenting `subject_did = svc_1`
    /// but anchored to `did:key:alice` reaches alice's row.
    #[test]
    fn anchor_terminal_filters_by_the_original_principal_not_the_caller() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_anchor_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();

        assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
    }

    /// A direct call carries no distinct anchor (`anchor_did == None`) --
    /// the compiler falls back to `subject_did` (a direct caller *is* the
    /// anchor) rather than denying the policy.
    #[test]
    fn anchor_terminal_falls_back_to_subject_did_when_anchor_is_absent() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_anchor_policy();
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

    /// The mirror of the filtering test: a caller who *does* own the row is
    /// denied when acting for an anchor who doesn't. Using
    /// `subject_did = alice` (the row's actual owner) is the discriminating
    /// case -- if the sieve wrongly bound `caller` instead of `anchor`, this
    /// row would leak; a stranger `subject_did` (as in an earlier version of
    /// this test) can't tell the two apart, since neither identity would
    /// match either way.
    #[test]
    fn anchor_terminal_denies_when_the_anchor_is_a_stranger() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_anchor_policy();
        let proxying_service = session_with_anchor(
            "did:key:alice",
            "did:key:mallory",
            vec![read_cap(Some("document"))],
        );
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();

        assert!(run_sieve(&conn, "documents", &sieve).is_empty());
    }

    /// The decision trace must surface `session.anchor_did` (ADR-0015 A5,
    /// amended) -- without it, an operator reading the log line for a
    /// proxying caller cannot tell whether the decision was made for
    /// `subject_did` or for a different principal it was acting on behalf
    /// of, which is exactly what the anchor mechanism exists to make
    /// auditable.
    #[test]
    fn decision_trace_records_the_anchor_did() {
        let policy = single_hop_anchor_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sieve.trace.subject_did, "did:key:svc-1");
        assert_eq!(sieve.trace.anchor_did.as_deref(), Some("did:key:alice"));
    }

    /// `anchor` resolution must hold in `Mode::PointInTime` too -- a wrong
    /// terminal there is a boolean allow/deny, not merely a missing row, so
    /// this exercises a code path Mode B's tests don't reach.
    #[test]
    fn anchor_terminal_holds_in_point_in_time_mode() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        insert_user(&conn, "u-alice", "did:key:alice", None);
        insert_document(&conn, "doc-1", "u-alice");

        let policy = single_hop_anchor_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::PointInTime { id: "doc-1".to_string() },
        )
        .unwrap()
        .unwrap();
        assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);

        let proxying_service_wrong_anchor = session_with_anchor(
            "did:key:svc-1",
            "did:key:mallory",
            vec![read_cap(Some("document"))],
        );
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service_wrong_anchor,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::PointInTime { id: "doc-1".to_string() },
        )
        .unwrap()
        .unwrap();
        assert!(run_sieve(&conn, "documents", &sieve).is_empty());
    }

    /// `anchor` resolution on a multi-hop, non-recursive chain --
    /// `emit_chain` resolves the terminal on a separate code path from the
    /// single-hop/zero-hop case.
    #[test]
    fn anchor_terminal_holds_across_a_multi_hop_chain() {
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
                                "paths": [["creator", "home_department", "anchor"]]
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

        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:carol", vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run_sieve(&conn, "documents", &sieve),
            vec!["doc-1"],
            "carol is the anchor, two joins away from the document -- not the caller (svc-1)"
        );
    }

    /// `anchor` resolution on a recursive relation -- `emit_fused_recursive`
    /// resolves the terminal on a separate code path from the non-recursive
    /// cases above.
    #[test]
    fn anchor_terminal_holds_on_a_recursive_relation() {
        let conn = Connection::open_in_memory().unwrap();
        seed_schema(&conn);
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
                                "paths": [["creator", "management_chain", "anchor"]]
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

        for (anchor_did, expect_visible) in
            [("did:key:eve", true), ("did:key:frank", true), ("did:key:mallory", false)]
        {
            let proxying_service =
                session_with_anchor("did:key:svc-1", anchor_did, vec![read_cap(Some("document"))]);
            let sieve = compile_read(
                &policy,
                "document",
                &proxying_service,
                SERVICE_ID,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .unwrap()
            .unwrap();
            let visible = run_sieve(&conn, "documents", &sieve).contains(&"doc-1".to_string());
            assert_eq!(visible, expect_visible, "anchor {anchor_did}");
        }
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
                anchor_did: None,
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

    fn remote_relation_policy() -> Policy {
        parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"owner": {
                            "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
                        }},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}
                        }
                    }
                }
            }"#,
        )
        .unwrap()
    }

    /// `compile_read` (B2's synchronous, local-only entry point) still fails
    /// closed on a policy whose selected path needs a remote fetch -- it has
    /// no way to perform one itself. `plan_read` is the B3 entry point that
    /// actually resolves it (see the tests below).
    #[test]
    fn compile_read_fails_closed_when_a_remote_fetch_is_needed() {
        let policy = remote_relation_policy();
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

    /// `plan_read` splits a policy needing a remote relation into a
    /// `RemoteFetch` (carrying the anchor as principal, per the
    /// confused-deputy defense) plus a `PendingSieve`, instead of failing
    /// closed -- the plan half of the two-phase compile (B3 plan §1.1).
    #[test]
    fn plan_read_collects_a_remote_fetch_instead_of_failing_closed() {
        let policy = remote_relation_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let plan = plan_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();

        assert!(plan.local.is_none());
        assert!(plan.pending.is_some());
        assert_eq!(plan.fetches.len(), 1);
        let fetch = &plan.fetches[0];
        assert_eq!(fetch.service, "hr-svc");
        assert_eq!(fetch.relation, "owner");
        assert_eq!(
            fetch.principal_did, "did:key:alice",
            "the fetch's principal is the anchor, not the proxying caller"
        );
    }

    /// A fully-local policy plans identically to `compile_read` -- `local`
    /// carries the finished sieve and `fetches`/`pending` are empty/`None`,
    /// exactly B2's shape. Zero behavior change for every existing
    /// local-only policy.
    #[test]
    fn plan_read_of_a_fully_local_policy_has_no_fetches() {
        let policy = single_hop_policy();
        let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
        let plan = plan_read(
            &policy,
            "document",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        assert!(plan.fetches.is_empty());
        assert!(plan.pending.is_none());
        assert!(plan.local.is_some());
    }

    /// `finalize` binds a fetched id-set into the pending sieve's `IN (...)`
    /// predicate and runs correctly against real seeded rows: the local row
    /// whose `owner_uuid` is in the fetched set is visible; one that isn't,
    /// isn't.
    #[test]
    fn finalize_binds_the_fetched_id_set_and_runs_correctly() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE documents (
                id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
                payload TEXT NOT NULL DEFAULT '{}'
            );",
        )
        .unwrap();
        let insert = |id: &str, owner_uuid: &str| {
            conn.execute(
                "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
                (id, json!({"owner_uuid": owner_uuid}).to_string()),
            )
            .unwrap();
        };
        insert("doc-1", "emp-alice");
        insert("doc-2", "emp-bob");

        let policy = remote_relation_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let mut plan = plan_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        let slot = plan.fetches[0].slot;
        let pending = plan.pending.take().unwrap();

        let results = vec![FetchResult { slot, ids: vec!["emp-alice".to_string()] }];
        let sieve = finalize(pending, &results).unwrap();
        assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
    }

    /// Mirrors the above with an empty fetched id-set: `IN (NULL)` is valid
    /// SQL and always false, never the invalid `IN ()`.
    #[test]
    fn finalize_binds_an_empty_id_set_as_in_null_not_invalid_sql() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE documents (
                id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
                payload TEXT NOT NULL DEFAULT '{}'
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
            [json!({"owner_uuid": "emp-alice"}).to_string()],
        )
        .unwrap();

        let policy = remote_relation_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let mut plan = plan_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        let slot = plan.fetches[0].slot;
        let pending = plan.pending.take().unwrap();
        let sieve = finalize(pending, &[FetchResult { slot, ids: Vec::new() }]).unwrap();
        assert!(run_sieve(&conn, "documents", &sieve).is_empty());
    }

    /// An id-set larger than `MAX_FETCH_IDS` is rejected rather than
    /// silently truncated or spliced into an unbounded `IN (...)` list (B3
    /// plan §5, fan-out containment).
    #[test]
    fn finalize_rejects_an_oversized_id_set() {
        let policy = remote_relation_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let mut plan = plan_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        let slot = plan.fetches[0].slot;
        let pending = plan.pending.take().unwrap();
        let oversized: Vec<String> = (0..MAX_FETCH_IDS + 1).map(|i| format!("id-{i}")).collect();
        let err = finalize(pending, &[FetchResult { slot, ids: oversized }]).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    /// `finalize` fails closed, rather than panicking or silently leaving
    /// the marker text in place, when `results` is missing a slot the
    /// pending sieve actually needs.
    #[test]
    fn finalize_fails_closed_on_a_missing_fetch_result() {
        let policy = remote_relation_policy();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let plan = plan_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .pending
        .unwrap();
        let err = finalize(plan, &[]).unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    /// D-B3-5: a `recursive: true` relation that is also remote
    /// (`service` set) is rejected at parse time -- B3 does not support an
    /// iterative cross-node transitive closure. Confirms the guard the
    /// policy schema already enforces (`policy::` tests pin the schema
    /// layer directly); this is the fdae-crate-level confirmation that a
    /// `Policy` value with that combination can never reach `plan_read` at
    /// all, since `Policy` is only ever constructed via `parse_and_validate`.
    #[test]
    fn remote_and_recursive_on_the_same_relation_cannot_reach_plan_read() {
        let err = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "user": {
                        "table": "users",
                        "principal_column": "did",
                        "relations": {
                            "management_chain": {
                                "target": "user", "from_key": "id", "to_key": "manager_id",
                                "recursive": true, "service": "hr-svc"
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap_err();
        assert!(matches!(err, PolicyError::Semantic(_)));
    }

    /// Two OR'd permission paths that both reach the *same* remote relation
    /// collapse into a single `RemoteFetch` (deduped by `(service,
    /// relation)`, plan §5) even though the marker text appears twice.
    #[test]
    fn plan_read_dedupes_repeated_fetches_to_the_same_remote_relation() {
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"owner": {
                            "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
                        }},
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "paths": [["owner", "anchor"], ["owner", "caller"]]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        let proxying_service =
            session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
        let plan = plan_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        assert_eq!(plan.fetches.len(), 1, "both paths name the same (service, relation)");
    }

    fn resolvable_employee_policy(principal_col: &str) -> Policy {
        parse_and_validate(&format!(
            r#"{{
                "version": "fdae/v1",
                "definitions": {{
                    "employee": {{
                        "table": "employees",
                        "principal_column": "{principal_col}",
                        "resolvable_without_capability": true
                    }}
                }}
            }}"#
        ))
        .unwrap()
    }

    /// D-B3-3 (A2): a definition opted into `resolvable_without_capability`
    /// resolves via a bare `principal_column = ?` predicate, runnable
    /// directly against the seeded table.
    #[test]
    fn resolve_structural_runs_correctly_against_a_json_payload_principal_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE employees (
                id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
                payload TEXT NOT NULL DEFAULT '{}'
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO employees (id, payload) VALUES ('emp-1', ?1)",
            [json!({"did": "did:key:alice"}).to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO employees (id, payload) VALUES ('emp-2', ?1)",
            [json!({"did": "did:key:bob"}).to_string()],
        )
        .unwrap();

        let policy = resolvable_employee_policy("did");
        let resolved = resolve_structural(&policy, "employee", "did:key:alice").unwrap().unwrap();
        assert_eq!(resolved.table, "employees");

        let sql = format!("SELECT id FROM {} WHERE {}", resolved.table, resolved.where_clause);
        let mut stmt = conn.prepare(&sql).unwrap();
        let ids: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(resolved.params.iter()), |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(ids, vec!["emp-1"]);
    }

    /// A reserved `principal_column` (e.g. `creator_id`) resolves to the
    /// physical column, not `json_extract(payload, '$.creator_id')` -- the
    /// same reserved-column addressing every other predicate uses.
    #[test]
    fn resolve_structural_addresses_a_reserved_column_directly() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE employees (
                id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
                payload TEXT NOT NULL DEFAULT '{}'
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO employees (id, creator_id) VALUES ('emp-1', 'did:key:alice')",
            [],
        )
        .unwrap();

        let policy = resolvable_employee_policy("creator_id");
        let resolved = resolve_structural(&policy, "employee", "did:key:alice").unwrap().unwrap();
        assert_eq!(resolved.where_clause, "employees.creator_id = ?");

        let sql = format!("SELECT id FROM {} WHERE {}", resolved.table, resolved.where_clause);
        let mut stmt = conn.prepare(&sql).unwrap();
        let ids: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(resolved.params.iter()), |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(ids, vec!["emp-1"]);
    }

    /// A definition that has *not* opted in resolves to `None`, not an
    /// error -- the caller treats this identically to "not found," never
    /// silently permitting structural resolution.
    #[test]
    fn resolve_structural_is_none_when_not_opted_in() {
        let policy = single_hop_policy();
        assert!(resolve_structural(&policy, "document", "did:key:alice").unwrap().is_none());
    }

    #[test]
    fn resolve_structural_is_none_for_an_unknown_relation() {
        let policy = resolvable_employee_policy("did");
        assert!(resolve_structural(&policy, "nonexistent", "did:key:alice").unwrap().is_none());
    }

    /// `definition_table` resolves either the definition key or the
    /// physical table name (case-insensitively) to the physical table --
    /// B3's `resolve-relation` needs the *table*, since `ServiceStore::query`
    /// addresses a collection literally, unlike `compile_read`'s own
    /// permissive key-or-table matching.
    #[test]
    fn definition_table_resolves_by_key_or_table_case_insensitively() {
        let policy = resolvable_employee_policy("did");
        assert_eq!(definition_table(&policy, "employee"), Some("employees"));
        assert_eq!(definition_table(&policy, "EMPLOYEE"), Some("employees"));
        assert_eq!(definition_table(&policy, "employees"), Some("employees"));
        assert_eq!(definition_table(&policy, "nonexistent"), None);
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

    #[test]
    fn decision_trace_claim_absent_across_multiple_permissions_is_detected() {
        // Regression: the deny used to be detected by string-matching the
        // *joined* predicate against the literal "(0=1)", which only holds
        // for a single applicable permission. Two permissions that both
        // fail claim resolution OR together as "(0=1 OR 0=1)" -- a
        // different string -- so the old check silently missed this and
        // logged the decision as an allow.
        let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view_a": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "caller"]],
                                "conditions": [{"column": "region", "claim": "region"}]
                            },
                            "view_b": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "caller"]],
                                "conditions": [{"column": "tier", "claim": "tier"}]
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();
        // A plain platform-ability read capability makes both `view_a` and
        // `view_b` applicable (each's `allows` covers `data-layer/read`);
        // neither `region` nor `tier` is in the caller's claims, so both
        // clauses fail closed independently.
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
            sieve.where_clause, "(0=1 OR 0=1)",
            "sanity: both permissions' paths must actually be OR'd, not deduplicated"
        );
        assert!(sieve.trace.operation_admitted);
        assert_eq!(
            sieve.trace.applicable_permissions,
            vec!["view_a".to_string(), "view_b".to_string()]
        );
        let reason = sieve.trace.path_failed.expect("a fully claim-absent deny must be traced");
        assert!(reason.contains("view_a") && reason.contains("view_b"), "reason was: {reason}");
    }

    #[test]
    fn decision_trace_records_default_not_covering_operation() {
        // The fifth deny reason (compile.rs's H1-hardened branch): the
        // caller holds a grant for the operation, but no permission's
        // `allows` covers it and the configured `default` doesn't either --
        // distinct from "operation not admitted" (no grant at all). Same
        // policy/capability shape as
        // `default_permission_not_covering_operation_is_denied`, which pins
        // the SQL; this test pins the trace.
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
        assert!(
            sieve.trace.operation_admitted,
            "the caller does hold a grant for the operation -- distinct from no grant at all"
        );
        assert!(sieve.trace.applicable_permissions.is_empty());
        assert!(
            sieve
                .trace
                .path_failed
                .as_deref()
                .is_some_and(|r| r.contains("no applicable default permission")),
            "path_failed was: {:?}",
            sieve.trace.path_failed
        );
    }

    #[test]
    fn compile_read_emits_a_deny_via_tracing() {
        // Nothing else in this suite proves `compile_read` actually calls
        // `trace.emit()` -- every other decision-trace test asserts on the
        // `CompiledSieve::trace` field the function *returns*, which would
        // stay green even if the `emit()` calls inside `compile_read` were
        // deleted entirely. This test captures real `tracing` output around
        // a call, the same way `data_db::sqlite::tests::decision_trace_
        // records_rows_not_reached_after_check_access_executes` proves
        // `do_check_access`'s own `emit()`.
        use std::{
            io,
            sync::{Arc, Mutex},
        };

        use tracing_subscriber::prelude::*;

        struct MockWriter {
            logs: Arc<Mutex<Vec<u8>>>,
        }
        impl io::Write for MockWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.logs.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();
        let make_writer = move || MockWriter { logs: logs_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
        let subscriber = tracing_subscriber::registry().with(layer);

        let policy =
            parse_and_validate(r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#)
                .unwrap();
        let alice = session("did:key:alice", vec![]);
        tracing::subscriber::with_default(subscriber, || {
            let _ = compile_read(
                &policy,
                "unrelated_collection",
                &alice,
                SERVICE_ID,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .unwrap();
        });

        let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        assert!(logs_content.contains("fdae decision: deny"), "logs were: {logs_content}");
        assert!(logs_content.contains("unrelated_collection"), "logs were: {logs_content}");
        assert!(logs_content.contains("did:key:alice"), "logs were: {logs_content}");
    }
}
