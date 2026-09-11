use rusqlite::{Connection, types::Value as SqlValue};
use syneroym_fdae::{CompiledSieve, DecisionTrace, Mode, compile_read};
use syneroym_ucan::Ability;

use super::{query_raw::QUERY_RAW_MAX_VM_OPS, schema::validate_identifier};
use crate::{auth::QueryAuth, errors::map_rusqlite_error, filter, host_store};

/// The real, post-execution outcome of a Mode A (point-in-time) predicate
/// run -- the distinction `compile_read`'s compile-time trace cannot make,
/// since it never executes SQL (ADR-0017 §9).
pub(crate) enum ModeAOutcome {
    /// The predicate matched a row.
    Matched,
    /// The predicate ran to completion and matched no row -- a genuine,
    /// admitted-but-unreachable deny.
    NotMatched,
    /// The predicate never ran to a real answer: a watchdog interrupt, a
    /// malformed caveat, a missing target table, or similar. Must not be
    /// recorded as `NotMatched` -- that would claim a fact about the data
    /// ("no row satisfied the predicate") that was never established, and
    /// would make a compute-budget abort indistinguishable in the trace
    /// from an ordinary deny.
    Aborted(String),
}

/// Emits `sieve.trace` a second time with the real, post-execution Mode A
/// outcome filled in. `compile_read` already emitted a compile-time trace
/// on `sieve.trace` (`rows_reached: None` -- it never executes SQL); this is
/// the only place "rows not reached" (an admitted operation whose compiled
/// predicate matched no row) -- or a policy-evaluation abort -- becomes
/// knowable.
pub(crate) fn emit_mode_a_execution_trace(trace: Option<&DecisionTrace>, outcome: ModeAOutcome) {
    let Some(trace) = trace else { return };
    let mut trace = trace.clone();
    match outcome {
        ModeAOutcome::Matched => trace.rows_reached = Some(true),
        ModeAOutcome::NotMatched => {
            trace.rows_reached = Some(false);
            if trace.path_failed.is_none() {
                trace.path_failed = Some("no row satisfied the compiled predicate".to_string());
            }
        }
        ModeAOutcome::Aborted(reason) => {
            trace.path_failed = Some(format!("policy evaluation aborted: {reason}"));
        }
    }
    trace.emit();
}

/// Mode B counterpart of [`emit_mode_a_execution_trace`]: `query`/
/// `aggregate`/`delete_many` never get a per-row outcome (`rows_reached` is
/// always `None` for these, unlike Mode A), so there is nothing to augment
/// post-execution. But `plan_read`'s own `trace.emit()` runs *before* a
/// cross-service relationship fetch resolves, so it necessarily logs
/// `remote_fetches: []` -- `finalize` only folds the real
/// `RemoteFetchTrace`(es) into `sieve.trace` afterward. Re-emitting that
/// already-finalized trace here is the only place a successful Mode B read's
/// fetch provenance (asserter DID, TTL) becomes observable; a fully local
/// sieve just logs the same allow/deny a second time, mirroring Mode A's
/// always-re-emit precedent.
pub(crate) fn emit_mode_b_trace(sieve: Option<&CompiledSieve>) {
    if let Some(s) = sieve {
        s.trace.clone().emit();
    }
}

/// The FDAE watchdog budget (ADR-0017 §8): a hard-coded interim default,
/// aliased to `QUERY_RAW_MAX_VM_OPS` since neither the policy schema nor
/// substrate config carries a budget field yet (deferred -- an `fdae`
/// schema change plus substrate-config plumbing, not this crate's call to
/// make). Recorded so the fixed constant isn't mistaken for "configurable,
/// done".
pub(crate) const FDAE_MAX_VM_OPS: i32 = QUERY_RAW_MAX_VM_OPS;

/// Clears only the progress handler on drop -- unlike `QueryRawGuard`, the
/// FDAE sieve paths install no authorizer (they emit only host-generated,
/// parameterized SQL, never arbitrary caller SQL), so clearing one here
/// would be dead noise. Needed on both reader-pool connections (reused
/// across calls) and the persistent writer connection (`delete_many`) so a
/// sieve'd call never leaves its budget installed for the next borrower.
pub(crate) struct ProgressGuard<'c> {
    pub(crate) conn: &'c Connection,
}

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        let _ = self.conn.progress_handler(0, None::<fn() -> bool>);
    }
}

pub(crate) fn install_watchdog(
    conn: &Connection,
) -> Result<ProgressGuard<'_>, host_store::DataLayerError> {
    conn.progress_handler(FDAE_MAX_VM_OPS, Some(|| true)).map_err(map_rusqlite_error)?;
    Ok(ProgressGuard { conn })
}

/// Returns `(clause, params)` = RLS ∧ each compiled caveat's `where`. RLS
/// and caveat `where` filters are both intersective and must AND together
/// -- dropping `where_caveats` would let a `caveats.where={"region":"EU"}`
/// caller see every region (a dropped-caveat bug).
pub(crate) fn merge_sieve(
    sieve: &CompiledSieve,
) -> Result<(String, Vec<SqlValue>), host_store::DataLayerError> {
    let mut clauses = vec![format!("({})", sieve.where_clause)];
    let mut params: Vec<SqlValue> = sieve.params.clone();
    for caveat in &sieve.where_caveats {
        let raw = serde_json::to_string(caveat)
            .map_err(|e| host_store::DataLayerError::Internal(format!("caveat serialize: {e}")))?;
        if let Some(cf) = filter::compile_filter(Some(&raw))? {
            clauses.push(format!("({})", cf.where_clause));
            params.extend(cf.params);
        }
    }
    Ok((clauses.join(" AND "), params))
}

/// Compiles the FDAE sieve for a `data-layer/read` operation, or `Ok(None)`
/// when `auth` is absent (today's unfiltered behavior). A compile error is
/// loud (`Err`), never silently treated as unfiltered -- Mode B/A's own
/// caller decides whether that maps to a hard error or fail-closed `false`.
pub(crate) fn compile_sieve_for(
    auth: Option<&QueryAuth<'_>>,
    collection: &str,
    mode: Mode,
) -> Result<Option<CompiledSieve>, host_store::DataLayerError> {
    compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_READ, mode)
}

pub(crate) fn compile_sieve_for_op(
    auth: Option<&QueryAuth<'_>>,
    collection: &str,
    operation: &str,
    mode: Mode,
) -> Result<Option<CompiledSieve>, host_store::DataLayerError> {
    let Some(auth) = auth else { return Ok(None) };
    // A caller that already ran `plan_read` + the `resolve_fetches`
    // orchestration + `finalize` (because the policy's selected paths
    // needed a remote relationship fetch) hands the already-compiled sieve
    // straight through -- `compile_read` below would otherwise fail closed
    // on exactly that case (it's the local-only entry point).
    //
    // Asserted, not assumed: `QueryAuth` carries no type-level guarantee
    // that a pre-resolved sieve was compiled for *this* `operation` --
    // `resolved_sieve` is `pub` on a type callers outside this module can
    // construct, and both current ingresses populate it unconditionally
    // (not only for the cross-service-fetch case), so a mismatch would
    // otherwise pass through silently and authorize `operation` against a
    // predicate compiled for a different (possibly wider) ability. Every
    // sieve records the ability it was compiled for in its own trace, so
    // the check is one comparison, not a new field.
    if let Some(sieve) = &auth.resolved_sieve {
        if sieve.trace.operation != operation {
            return Err(host_store::DataLayerError::PermissionDenied);
        }
        return Ok(Some(sieve.clone()));
    }
    // Validated here, before `collection` reaches `compile_read` and a
    // strict-mode deny logs it verbatim into `path_failed` (an `info!`
    // line) -- otherwise a WASM guest could inject newlines/escapes into
    // the operator log through an unvalidated collection name before
    // `do_get`/`do_query` get a chance to validate it downstream on the
    // pool thread.
    validate_identifier(collection)?;
    compile_read(
        auth.policy,
        collection,
        auth.session,
        auth.service_id,
        &Ability(operation.to_string()),
        mode,
    )
    .map_err(|e| host_store::DataLayerError::Internal(e.to_string()))
}

pub(crate) fn sieve_masked_fields(sieve: &Option<CompiledSieve>) -> Vec<String> {
    sieve.as_ref().map(|s| s.masked_fields.clone()).unwrap_or_default()
}
