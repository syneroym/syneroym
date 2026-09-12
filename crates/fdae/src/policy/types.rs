//! The FDAE policy document types and data structures.

use std::collections::BTreeMap;

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
pub(crate) const MAX_PATH_HOPS: usize = 32;

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
    /// Opts this definition into structural cross-service
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
    /// time by `compile::plan_read`. Requires
    /// `join_column` too, exactly like a local join -- it names which local
    /// column is checked (`IN (...)`) against the remote's returned id-set,
    /// since there is no local `target` table to `EXISTS`-join through.
    #[serde(default)]
    pub service: Option<String>,
    /// Required alongside `service`: the DID a fetched `RelationshipProof`
    /// for this relation must be signed by. Not derived --
    /// `Identity::derive_service_identity` is a node-private
    /// derivation (keyed on that node's own secret key), so a different
    /// node can never independently reproduce it. The policy author instead
    /// declares an explicit, auditable trust anchor, same category as
    /// `resolvable_without_capability`; `resolve_fetches` rejects a proof
    /// whose `asserter_did` doesn't match this value.
    #[serde(default)]
    pub expected_asserter_did: Option<String>,
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
    /// Stage-4 ABAC opt-in (ADR-0017 §7). When true, a read admitted through
    /// this permission is additionally passed to the service's guest-exported
    /// `authorize-rows` after-step before any row reaches the caller.
    /// Restrict-only: the after-step may drop rows or redact fields, never
    /// admit a row this permission's `paths:` did not reach.
    ///
    /// **The opt-in is per permission, but its effect is call-scoped, not
    /// row-scoped.** A definition's applicable
    /// permissions OR into one compiled `where_clause`; if *any* applicable
    /// permission on this definition opts in, every row the combined clause
    /// admits is judged by the after-step, including rows a *different*
    /// sibling permission (one that left this `false`) alone would have
    /// admitted. Still restrict-only -- the after-step can never widen past
    /// what the sieve already picked -- but a row's admission cannot be
    /// attributed back to a single permission, so the guest cannot apply
    /// different logic per originating rule; `auth-context.permissions`
    /// (the WIT `authorizer` interface) is every applicable permission on
    /// this read, batch-scoped, not the one that happened to admit a given
    /// row.
    #[serde(default)]
    pub authorize_rows: bool,
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
