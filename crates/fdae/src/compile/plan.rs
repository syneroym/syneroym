//! Planning and orchestration for ReBAC read compilation and finalization.

use std::{collections::BTreeSet, ptr};

use rusqlite::types::Value;
use serde_json::Value as Json;
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};

use super::{
    emit::{compile_permission, find_definition},
    types::{CompiledSieve, FetchCtx, FetchResult, MAX_FETCH_IDS, Mode, PendingSieve, ReadPlan},
};
use crate::{
    policy::{Definition, Policy, PolicyError},
    trace::DecisionTrace,
};

/// Binds each [`PendingSieve`] marker's fetched id-set into its predicate
/// text position and the corresponding `?` values into `params`, producing
/// a finished [`CompiledSieve`] ready to run exactly like a fully-local one.
/// Fails closed (never silently drops a fetch) if `results` is missing a
/// slot `pending` needs, or if a fetch's id-set exceeds [`MAX_FETCH_IDS`].
/// An empty id-set compiles to `{correlate_expr} IN (SELECT 1 WHERE 0)`
/// (an empty-subquery membership test -- unambiguously `false`), **not**
/// `{correlate_expr} IN (NULL)`: under SQLite's three-valued logic
/// `x IN (NULL)` evaluates to `NULL`, which reads as `false` in a bare
/// `WHERE` but does **not** invert under `NOT` the way `false` does -- an
/// `exclusion`-operator permission with a remote hop that legitimately
/// resolves to nobody would otherwise deny every row (`NOT NULL` is
/// `NULL`, not `true`) instead of excluding none. `IN (<empty subquery>)`
/// has no such ambiguity -- there is no candidate row to compare against,
/// NULL or otherwise. Verified against SQLite directly:
/// `SELECT typeof('x' IN (NULL))` -- `'null'`; `SELECT 'x' IN (SELECT 1
/// WHERE 0), NOT ('x' IN (SELECT 1 WHERE 0))` -- `0, 1`.
pub fn finalize(
    pending: PendingSieve,
    results: &[FetchResult],
) -> Result<CompiledSieve, PolicyError> {
    let PendingSieve {
        mut where_clause,
        mut params,
        masked_fields,
        where_caveats,
        mut trace,
        markers,
        abac_permissions,
    } = pending;

    // Markers are inserted in ascending `params_index` order so `shift`
    // (the running count of ids already spliced in) correctly accounts for
    // every earlier insertion's effect on later positions -- `Vec::insert`
    // shifts everything at/after its index rightward, exactly matching how
    // later `?` occurrences in the text sit after earlier ones.
    let mut ordered = markers;
    ordered.sort_by_key(|m| m.params_index);

    let mut shift = 0usize;
    // One `RemoteFetchTrace` entry per distinct *slot* consumed, not per
    // marker occurrence -- the same remote relation reached by several OR'd
    // permission paths shares one fetch (`FetchCtx::register`'s dedup), so
    // it should leave one provenance record, not one per occurrence.
    let mut traced_slots: BTreeSet<usize> = BTreeSet::new();
    for marker in &ordered {
        let result = results.iter().find(|r| r.slot == marker.slot).ok_or_else(|| {
            PolicyError::Semantic(format!(
                "finalize: missing fetch result for slot {}",
                marker.slot.0
            ))
        })?;
        let ids = result.ids.as_slice();
        if traced_slots.insert(marker.slot.0) {
            trace.remote_fetches.push(result.trace.clone());
        }
        if ids.len() > MAX_FETCH_IDS {
            return Err(PolicyError::Semantic(format!(
                "remote fetch returned {} ids, exceeding the {MAX_FETCH_IDS} cap",
                ids.len()
            )));
        }
        let ids_sql = if ids.is_empty() {
            "SELECT 1 WHERE 0".to_string()
        } else {
            vec!["?"; ids.len()].join(", ")
        };
        let replacement = format!("{} IN ({})", marker.correlate_expr, ids_sql);
        where_clause = where_clause.replacen(&marker.token, &replacement, 1);
        if !ids.is_empty() {
            let insert_at = marker.params_index + shift;
            for (i, id) in ids.iter().enumerate() {
                params.insert(insert_at + i, Value::Text(id.clone()));
            }
            shift += ids.len();
        }
    }

    Ok(CompiledSieve {
        where_clause,
        params,
        masked_fields,
        where_caveats,
        trace,
        abac_permissions,
    })
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
///   selected paths require a remote relationship fetch**: this function is the
///   synchronous/local-only entry point and cannot itself resolve one -- a
///   caller that needs to may call [`plan_read`] directly. Either way the
///   caller must treat this as deny, never as unfiltered access.
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
        // `plan_read` already emitted a trace for this compilation,
        // but it necessarily reads as an *allow* (`operation_admitted:
        // true`, no `path_failed`, `compiled_predicate` full of unresolved
        // `@@FDAE_FETCH_...@@` markers) -- it doesn't yet know the caller
        // is `compile_read`, which is about to turn it into a hard deny.
        // Emit a second trace recording the actual outcome, so an operator
        // reading the log sees the deny and why, not just the earlier
        // allow-shaped record.
        let trace = DecisionTrace {
            tier: 3,
            collection: collection.to_string(),
            service_id: service_id.to_string(),
            subject_did: session.subject_did.clone(),
            anchor_did: session.anchor_did.clone(),
            operation: operation.0.clone(),
            operation_admitted: false,
            path_failed: Some(format!(
                "policy requires {} remote relationship fetch(es), which compile_read \
                 (local-only) cannot resolve -- use plan_read/finalize (B3 pipeline stage 2) \
                 instead",
                plan.fetches.len()
            )),
            ..DecisionTrace::default()
        };
        trace.emit();
        return Err(PolicyError::Semantic(format!(
            "policy requires {} remote relationship fetch(es) to answer this read -- compile_read \
             is local-only; use plan_read/finalize (B3 pipeline stage 2) instead",
            plan.fetches.len()
        )));
    }
    Ok(plan.local)
}

/// The two-phase counterpart of [`compile_read`]: same
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
            operation: operation.0.clone(),
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
                    operation: operation.0.clone(),
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

    // Stage-4 ABAC opt-in (ADR-0017 §7): every applicable permission that set
    // `authorize_rows: true`, including the `default` fallback if it was
    // just folded into `applicable` above. Empty -- the overwhelmingly
    // common case -- means no after-step.
    let abac_permissions: Vec<String> = applicable
        .iter()
        .filter(|name| def.permissions.get(*name).is_some_and(|p| p.authorize_rows))
        .cloned()
        .collect();

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
        operation: operation.0.clone(),
        operation_admitted: true,
        applicable_permissions: applicable.iter().cloned().collect(),
        compiled_predicate: Some(where_clause.clone()),
        rows_reached: None,
        row_id: None,
        write_phase: None,
        path_failed,
        caveats_applied,
        remote_fetches: Vec::new(),
        abac_permissions: abac_permissions.clone(),
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
                abac_permissions,
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
                abac_permissions,
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

fn deny_all() -> CompiledSieve {
    CompiledSieve {
        where_clause: "0=1".to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
        trace: DecisionTrace::default(),
        abac_permissions: Vec::new(),
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
