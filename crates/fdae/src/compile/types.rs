//! Types and data structures for ReBAC -> SQL compilation.

use rusqlite::types::Value;
use serde_json::Value as Json;

use crate::{
    policy::PolicyError,
    trace::{DecisionTrace, RemoteFetchTrace},
};

/// Depth backstop for a recursive relation's self-join, bound as a `?`
/// param (never interpolated). The `visited_track` path-concatenation guard
/// is the primary cycle defense; this is the secondary bound.
pub(crate) const MAX_RECURSION_DEPTH: i64 = 64;

/// Physical columns every collection table carries directly; any other
/// policy-referenced column name addresses the JSON `payload` column
/// instead (`json_extract`).
pub(crate) const RESERVED_COLUMNS: [&str; 4] = ["id", "creator_id", "created_at", "updated_at"];

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
    /// Applicable permission names that opted into the stage-4 ABAC after-step
    /// (ADR-0017 §7, `Permission.authorize_rows`). Empty -- the overwhelmingly
    /// common case -- means no after-step: the sieve's rows are final.
    /// Non-empty obliges the *ingress* (never `data_db`, which has no WASM
    /// engine) to run `authorize-rows` over the candidate rows before
    /// returning them, and obliges `aggregate`/`delete_many` to deny
    /// closed.
    pub abac_permissions: Vec<String>,
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
/// `IN (...)` list's cardinality (fan-out containment). An
/// unbounded id-set would let a misbehaving or compromised remote blow up
/// the local query's `IN` list arbitrarily. Matches `data_db`'s existing
/// per-page query cap.
pub const MAX_FETCH_IDS: usize = 1000;

/// Correlation key into a [`ReadPlan`]'s `fetches`: one per distinct
/// `(service, relation)` pair a policy's selected paths need (deduped --
/// multiple hops naming the same remote relation share one fetch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchSlot(pub(crate) usize);

/// One remote relationship-proof fetch [`plan_read`] needs before its sieve
/// can be finalized (ADR-0017 §6). The orchestration that actually performs
/// the fetch (resolving `service` to a DID, issuing the proxy call,
/// enforcing the timeout) lives outside this crate (`crates/fdae` stays
/// proxy-free); this struct is only the *request* shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFetch {
    /// Logical service name from `Relation.service`, resolved to a DID by
    /// the caller (this crate stays free of the app-context registry).
    pub service: String,
    /// The **remote object type** (`Relation.target`), not the local
    /// relation edge name (`Relation`'s own key in `Definition.relations`).
    /// The two live in different namespaces: `hop.name` is only meaningful
    /// inside the *requesting* policy, while the remote resolves this
    /// string against its *own* `definitions:` map (`definition_table` /
    /// `resolve_structural`, which match object-type keys and physical
    /// table names) -- sending the local edge name would ask the remote
    /// about a name it likely has no definition for at all, silently
    /// returning an empty (indistinguishable from a legitimate deny)
    /// instead of resolving.
    pub relation: String,
    /// The principal the remote must evaluate for -- always the **anchor**
    /// (`session.anchor_did`, falling back to `subject_did` for a direct
    /// caller), never the presenting/proxying caller. The confused-deputy
    /// defense (ADR-0015 A5): this holds regardless of whether the path's
    /// own declared terminal word is `caller` or `anchor`.
    pub principal_did: String,
    /// The DID a fetched `RelationshipProof` for this relation must be
    /// signed by, from the policy's own `Relation.expected_asserter_did` --
    /// never derived by the fetching side (a per-node HKDF
    /// derivation cannot be reproduced by a different node), and never
    /// taken from the proof's own self-declared field (self-referential,
    /// verifies for any signer). The caller performing the fetch rejects a
    /// proof whose `asserter_did` doesn't match this value.
    pub expected_asserter_did: String,
    pub slot: FetchSlot,
}

/// One fetched result, matched back to its [`RemoteFetch`] by `slot`. `trace`
/// is the already-verified provenance the fetching side observed (asserter,
/// relation, principal, TTL) -- `finalize` folds it into the sieve's
/// [`DecisionTrace`] so a successful fetch, not just a timeout/deny, leaves a
/// record (ADR-0017 §6 reason 2).
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub slot: FetchSlot,
    pub ids: Vec<String>,
    pub trace: RemoteFetchTrace,
}

/// One not-yet-resolved position in a [`PendingSieve`]'s SQL text: a unique
/// text token standing in for a remote fetch's eventual `IN (?, ?, ...)`
/// list, plus where in the flat `params` sequence that list's bound values
/// belong once known.
#[derive(Debug, Clone)]
pub(crate) struct PendingMarker {
    pub(crate) slot: FetchSlot,
    /// Unique per *occurrence* (not per slot -- the same slot can appear at
    /// multiple text positions when several OR'd permission paths reach the
    /// same remote relation), so `finalize` can replace each occurrence
    /// independently via a single, unambiguous `replacen`.
    pub(crate) token: String,
    /// The bound-column expression (`col()`'s output, e.g.
    /// `documents.owner_uuid` or `json_extract(documents.payload, ?)`) this
    /// marker's fetched id-set is checked against. The token stands in for
    /// the **whole** `{expr} IN (...)` predicate, not just the
    /// parenthesized list, so `finalize` can substitute a
    /// definitively-`false` empty-subquery `IN (SELECT 1 WHERE 0)` for an
    /// empty id-set -- see `finalize`'s doc comment for why `{expr} IN
    /// (NULL)` is the wrong substitution (SQLite three-valued
    /// logic: `NULL`, not `false`, which silently inverts under `NOT`).
    pub(crate) correlate_expr: String,
    /// Index into `PendingSieve.params` where this marker's id-set values
    /// are inserted -- i.e. `params.len()` at the moment this marker was
    /// emitted during path compilation, so binding order matches the `?`
    /// occurrences' left-to-right text order.
    pub(crate) params_index: usize,
}

/// A [`CompiledSieve`] that still needs one or more remote relationship
/// fetches before it can run -- the "plan" half of the two-phase compile.
/// Opaque outside this module; the only thing a caller does with one is
/// pass it to [`finalize`] alongside the fetched [`FetchResult`]s.
#[derive(Debug, Clone)]
pub struct PendingSieve {
    pub(crate) where_clause: String,
    pub(crate) params: Vec<Value>,
    pub(crate) masked_fields: Vec<String>,
    pub(crate) where_caveats: Vec<Json>,
    pub(crate) trace: DecisionTrace,
    pub(crate) markers: Vec<PendingMarker>,
    pub(crate) abac_permissions: Vec<String>,
}

/// The result of [`plan_read`]: either a fully-compiled local sieve (the
/// local-only case -- `fetches` empty, `pending` `None`, `local` mirrors
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
/// per `(service, relation)` -- `relation` is the remote **object type**,
/// so two hops naming the same service and target type share one fetch
/// even if they arrived via different local relation names) and the text
/// markers standing in for their eventual predicates.
#[derive(Debug, Default)]
pub(crate) struct FetchCtx {
    pub(crate) fetches: Vec<RemoteFetch>,
    pub(crate) markers: Vec<PendingMarker>,
    pub(crate) next_marker: usize,
}

impl FetchCtx {
    /// Registers one occurrence of a remote relation reached at the current
    /// `params_index`, deduping the underlying fetch by `(service,
    /// relation)` but always emitting a fresh, unique text token for this
    /// occurrence. Returns the token to splice into the SQL text as the
    /// **entire** boolean predicate (see [`PendingMarker::correlate_expr`]
    /// for why the marker can't be scoped to just the `IN (...)` list).
    pub(crate) fn register(
        &mut self,
        service: String,
        relation: String,
        principal_did: String,
        expected_asserter_did: String,
        correlate_expr: String,
        params_index: usize,
    ) -> Result<String, PolicyError> {
        let slot =
            match self.fetches.iter().find(|f| f.service == service && f.relation == relation) {
                Some(existing) => {
                    // Two hops naming the same remote (service, relation)
                    // must agree on who is trusted to answer for it -- a
                    // policy declaring two different `expected_asserter_did`
                    // values for the same remote type is a misconfiguration,
                    // not a case to silently resolve by keeping whichever
                    // hop registered first.
                    if existing.expected_asserter_did != expected_asserter_did {
                        return Err(PolicyError::Semantic(format!(
                            "relation '{relation}' on service '{service}' is reached with two \
                             different expected_asserter_did values ('{}' vs \
                             '{expected_asserter_did}')",
                            existing.expected_asserter_did
                        )));
                    }
                    existing.slot
                }
                None => {
                    let slot = FetchSlot(self.fetches.len());
                    self.fetches.push(RemoteFetch {
                        service,
                        relation,
                        principal_did,
                        expected_asserter_did,
                        slot,
                    });
                    slot
                }
            };
        let token = format!("@@FDAE_FETCH_{}_{}@@", slot.0, self.next_marker);
        self.next_marker += 1;
        self.markers.push(PendingMarker {
            slot,
            token: token.clone(),
            correlate_expr,
            params_index,
        });
        Ok(token)
    }
}

/// A raw `<principal_column> = ?` predicate for [`resolve_structural`]'s
/// capability-free fallback: no `WHERE EXISTS`, no capability check, no
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
