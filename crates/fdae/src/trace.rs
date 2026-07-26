//! ADR-0017 §9 decision trace: a structured record of one FDAE tier-3 (SQL
//! pushdown sieve) decision, emitted via `tracing` so a deny -- or an
//! admitted-but-runtime-filtered allow -- is observable without a queryable
//! trace API, which is a later slice.

use tracing::{debug, info};

/// Provenance for one successful cross-service relationship-proof fetch
/// (Slice B3 pipeline stage 2, ADR-0017 §6). Recorded regardless of caching
/// -- even an immediate, uncached fetch must leave a record of *which
/// service asserted what, valid how long*, so the trace isn't allow-shaped
/// silence the way a fail-closed deny already is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteFetchTrace {
    /// Logical service name asked (`RemoteFetch.service`), not yet resolved
    /// to a DID -- the DID that actually answered is `asserter_did` below.
    pub service: String,
    /// The remote object type asked about (`RemoteFetch.relation`).
    pub relation: String,
    /// The principal evaluated for -- always the anchor (or the direct
    /// caller when there is no distinct anchor), never the proxying caller.
    pub principal_did: String,
    /// The DID that actually signed the returned `RelationshipProof`,
    /// already verified against the policy's `expected_asserter_did` and
    /// the signature itself before this trace entry is recorded.
    pub asserter_did: String,
    pub valid_until_secs: u64,
}

/// One FDAE tier-3 decision, built by `compile_read` at compile time and,
/// for Mode A, augmented by `check_access`/`get` once the compiled
/// predicate has actually been run against a row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecisionTrace {
    /// Always 3 -- the ADR-0017 pipeline stage this compiler implements.
    pub tier: u8,
    /// The collection this decision was compiled for.
    pub collection: String,
    /// The service the collection belongs to.
    pub service_id: String,
    /// The caller's DID, so a deny line is attributable to a principal.
    pub subject_did: String,
    /// `session.anchor_did` (ADR-0015 A5, amended), surfaced unconditionally
    /// alongside `subject_did` -- mirroring how `subject_did` itself is
    /// recorded regardless of whether any evaluated permission's path
    /// actually terminates in it. `None` for a direct call with no distinct
    /// anchor. Without this, an operator reading the trace has no way to
    /// tell whether a decision was made for `subject_did` or for a
    /// different principal it was proxying for -- exactly the distinction
    /// the anchor mechanism exists to make auditable.
    pub anchor_did: Option<String>,
    /// The grant(s) (`with::can`) evaluated for this decision.
    pub held: Vec<String>,
    /// Whether any held capability grants the requested operation on this
    /// resource, through either the platform-ability or app-permission
    /// route.
    pub operation_admitted: bool,
    /// Permission names whose paths were OR'd into the compiled predicate.
    pub applicable_permissions: Vec<String>,
    /// The compiled SQL boolean expression: for Mode A, a point-in-time
    /// check over one row; for Mode B, the row-filtering predicate every
    /// returned row satisfies.
    pub compiled_predicate: Option<String>,
    /// Mode A only: whether the compiled predicate actually matched a row,
    /// known only once `check_access`/`get` executes it. `None` at compile
    /// time, and always `None` for Mode B (a per-row question, not one
    /// boolean).
    pub rows_reached: Option<bool>,
    /// Set when this decision is a known deny: at compile time (operation
    /// not admitted, a strict-mode unknown collection, or a condition
    /// referencing an absent claim) or, for Mode A, after execution finds
    /// no matching row or aborts before reaching one.
    pub path_failed: Option<String>,
    /// Which caveat *kinds* were folded into this decision -- field names
    /// and caveat-filter keys, deliberately not the caveat *values*
    /// (capability caveats can carry DIDs, tenant ids, and other row-level
    /// data that has no business in an operator log).
    pub caveats_applied: Vec<String>,
    /// Cross-service relationship-proof fetches this decision's compiled
    /// sieve depended on (Slice B3), populated by `finalize` from each
    /// consumed `FetchResult.trace`. Empty for a fully-local policy.
    pub remote_fetches: Vec<RemoteFetchTrace>,
    /// Applicable permissions that opted into the stage-4 after-step
    /// (ADR-0017 §7, `Permission.authorize_rows`). Recorded at compile time;
    /// the *outcome* of that step is a separate, execution-aware trace the
    /// ingress emits ([`AbacTrace`]), the same two-trace shape Mode A
    /// already uses for `rows_reached`.
    pub abac_permissions: Vec<String>,
}

impl DecisionTrace {
    /// Logs this decision via `tracing`: `info` for a deny (the actionable
    /// signal an operator watching `info`-level logs should see without
    /// opting into `debug` noise), `debug` for an allow.
    pub fn emit(&self) {
        if let Some(reason) = &self.path_failed {
            info!(
                tier = self.tier,
                collection = %self.collection,
                service_id = %self.service_id,
                subject_did = %self.subject_did,
                anchor_did = ?self.anchor_did,
                held = ?self.held,
                operation_admitted = self.operation_admitted,
                applicable_permissions = ?self.applicable_permissions,
                compiled_predicate = self.compiled_predicate.as_deref(),
                rows_reached = ?self.rows_reached,
                path_failed = %reason,
                caveats_applied = ?self.caveats_applied,
                remote_fetches = ?self.remote_fetches,
                abac_permissions = ?self.abac_permissions,
                "fdae decision: deny"
            );
        } else {
            debug!(
                tier = self.tier,
                collection = %self.collection,
                service_id = %self.service_id,
                subject_did = %self.subject_did,
                anchor_did = ?self.anchor_did,
                held = ?self.held,
                operation_admitted = self.operation_admitted,
                applicable_permissions = ?self.applicable_permissions,
                compiled_predicate = self.compiled_predicate.as_deref(),
                rows_reached = ?self.rows_reached,
                caveats_applied = ?self.caveats_applied,
                remote_fetches = ?self.remote_fetches,
                abac_permissions = ?self.abac_permissions,
                "fdae decision: allow"
            );
        }
    }
}

/// The stage-4 ABAC after-step's actual outcome for one read (ADR-0017 §9).
/// A separate struct from [`DecisionTrace`], not a field on it: the sieve is
/// compiled synchronously and traced immediately, while the after-step runs
/// later (and only conditionally, per ingress) once candidate rows exist --
/// the same two-trace shape Mode A already uses for `rows_reached`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AbacTrace {
    /// The permissions that triggered this after-step invocation, mirroring
    /// `DecisionTrace.abac_permissions` for the same read.
    pub permissions: Vec<String>,
    pub rows_in: usize,
    pub rows_denied: usize,
    pub rows_redacted: usize,
    /// `Some(reason)` when the whole batch was denied closed -- missing
    /// export, fuel/epoch overrun, trap, or a malformed decision list.
    pub failed_closed: Option<String>,
}

impl AbacTrace {
    /// Logs this outcome via `tracing`: `info` when the batch was denied
    /// closed or any row was denied/redacted (the actionable signal an
    /// operator should see without opting into `debug`), `debug` otherwise.
    pub fn emit(&self, collection: &str, service_id: &str, subject_did: &str) {
        if let Some(reason) = &self.failed_closed {
            info!(
                collection = %collection,
                service_id = %service_id,
                subject_did = %subject_did,
                permissions = ?self.permissions,
                rows_in = self.rows_in,
                rows_denied = self.rows_denied,
                rows_redacted = self.rows_redacted,
                failed_closed = %reason,
                "fdae stage-4 abac: denied closed"
            );
        } else if self.rows_denied > 0 || self.rows_redacted > 0 {
            info!(
                collection = %collection,
                service_id = %service_id,
                subject_did = %subject_did,
                permissions = ?self.permissions,
                rows_in = self.rows_in,
                rows_denied = self.rows_denied,
                rows_redacted = self.rows_redacted,
                "fdae stage-4 abac: rows filtered"
            );
        } else {
            debug!(
                collection = %collection,
                service_id = %service_id,
                subject_did = %subject_did,
                permissions = ?self.permissions,
                rows_in = self.rows_in,
                "fdae stage-4 abac: all rows allowed"
            );
        }
    }
}
