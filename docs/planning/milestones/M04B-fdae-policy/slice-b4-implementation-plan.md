# Slice B4-fdae Implementation Plan — Stage-4 WASM ABAC

**Source of record:** [`task.md`](task.md) "Slice B4-fdae: Stage-4 WASM ABAC"
(§ *Ordered Implementation Slices*), the Failure/Security matrix rows 7–9, and
Performance Budgets row 3. **ADR:**
[ADR-0017](../../../decisions/0017-fdae-policy-schema-and-compilation.md) §7
(with §8's safety rails and §9's decision trace),
[ADR-0005](../../../decisions/0005-wasm-fuel-quota-schema.md) (fuel),
[ADR-0015](../../../decisions/0015-ucan-capability-model.md) A5 (`anchor_did`).

**Verified against `main` @ `4302827`** (Slice B3.5-fdae complete). Every line
number and symbol below was read on that commit. **Revised 2026-07-25** after
two review passes; corrections are marked *(rev 2)* where they change an earlier
instruction.

---

## 0. What this slice is, in one paragraph

The SQL sieve (B2/B3) decides *which rows a caller may reach*. Stage 4 is the
after-step for the cases SQL cannot express: the host hands the sieve's already
authorized candidate rows to a **guest-exported** `authorize-rows` function,
which returns one decision per row — `allow` / `deny` / `redact(fields)`. It is
**batched**, **opt-in per policy permission**, **restrict-only** (it can only
subtract rows and fields; it can never admit a row the sieve excluded, by
construction — it is only ever shown rows that already passed), and
**fuel-/time-metered** with deny-closed on overrun.

### 0.1 How the after-step function is specified, and what deploy requires

Answering this directly, because it is the first thing a reader asks and §3 only
implies it.

- **The policy does not name a function.** Opt-in is a boolean on a
  `permissions:` entry — `authorize_rows: true` (§3.2) — and nothing more. The
  function the host calls is a single **well-known export** at a fixed name:
  `syneroym:data-layer/authorizer@0.1.0#authorize-rows` (§3.4). One export per
  component, however many permissions opt in.
- **So one guest function serves every opted-in permission on every
  collection.** It therefore *must* be told which collection and which
  permissions triggered it, or it cannot dispatch. The illustrative
  `auth-context` in ADR-0017 §7 carries neither; §3.4 adds `collection` and
  `permissions` for exactly this reason (§5 item 1).
- **Yes, the substrate expects the export to be present** — but only when a
  policy opts in. A component with no opted-in permission is never asked, and a
  component that doesn't export it is never looked up, so this stays additive
  for every service shipping today. When a policy *does* opt in and the export
  is missing, the deploy is **rejected** (§3.7, `validate_stage4_export`) rather
  than accepted into a state where every read through that permission would
  deny closed at runtime.
- **Rejected alternative — a policy-named export** (`authorize_rows: "my-fn"`,
  several distinct functions per component). It buys nothing the `collection` +
  `permissions` fields don't already give the guest, and it multiplies the
  deploy-time export check and the missing-export failure mode by the number of
  names a policy can mention. Flagged so the choice is visible, not assumed.

---

## 1. Blocking design decisions — resolve before coding

These are places where `task.md`/ADR-0017 do not determine the implementation
and where guessing would produce materially different code. Recommendations are
given; **D-B4-1 and D-B4-2 must be answered before Phase 2 starts** — §3 is
written assuming both take the recommended path and is not executable otherwise
(D-B4-2 spells out exactly what changes if it doesn't). The rest must be
answered before their own phase.

### D-B4-1 — How does a stage-4 guest run at all? (blocking)

Not addressed anywhere in ADR-0017 or `task.md`, and it is the load-bearing
question of the slice. Both read ingresses reach the sieve from inside a context
where calling a guest export is not possible in place:

- **Ingress (i)**, `sandbox_wasm/src/host_capabilities.rs:553` (`store::Host::query`)
  runs *inside a host function of a live component instance*. Wasmtime forbids
  re-entering the same component instance, and a host function receives
  `&mut HostState`, not `&mut Store<HostState>` — there is no handle to call an
  export with.
- **Ingress (ii)**, `control_plane/src/synsvc_native.rs:672` (`"query"` arm) has
  no WASM engine reference at all; `SynSvcNativeService` holds only
  `storage_provider`, `key_store`, `fdae_policy`, `service_proxy`, `service_id`.

**Recommendation (assumed by the rest of this plan):** the after-step runs in a
**freshly instantiated, throw-away `Store`/`Instance` of the same component**,
exactly like `AppSandboxEngine::deliver_message`
([engine.rs:980](../../../../crates/sandbox_wasm/src/engine.rs#L980)) and
`invoke_lifecycle_hook` ([engine.rs:885](../../../../crates/sandbox_wasm/src/engine.rs#L885))
already do. Both ingresses reach it through a new object-safe trait
(`RowAuthorizer`, §3.2) held as a `Weak<dyn RowAuthorizer>`, mirroring how
`Weak<dyn ServiceProxy>` is already threaded into both
(`HostState.service_proxy`, `SynSvcNativeService.service_proxy`).

Cost: one component instantiation per stage-4-active read. `substrate.wasm.instantiation_ms`
is already measured; Phase 5's bench quantifies it.

### D-B4-2 — Under what identity does the after-step run, and can it read? (blocking)

ADR-0017 §7: *"Both escape hatches run under the service's own identity, never
the caller's"* and *"It may issue read-only lookups"*. But today the only
service-own identities are:

- `CallerContext::service_system` — capability-less, so `plan_read` falls to
  `deny_all()` and **every lookup returns zero rows**. §7's lookups would be
  dead on arrival.
- `CallerContext::local_elevated` — carries `data-layer/admin` and is
  FDAE-exempt (`host_capabilities.rs:212`), i.e. it also unlocks `execute-ddl`
  and `query-raw`. Far more than "read-only".

There is no read-only elevated context. Additionally, host **write** paths carry
*no* capability gate at all today (D-04-02-f), so "read-only" cannot be achieved
by choosing a weaker capability — it must be enforced structurally.

**Recommendation:** add a third substrate-injected level (§3.1) —
`AuthLevel::LocalReadOnly` / `CallerContext::service_abac(service_id)`,
**sieve-exempt** like `LocalElevated` and carrying **no capabilities at all**,
paired with a `HostState.read_only` flag that hard-denies every mutating and
egress host function. Read-only is a property of the host state, not of a
capability nobody checks.

*(rev 2 — two corrections to the first draft, both raised in review.)*

- **The after-step's own lookups read this service's data unfiltered, and that
  is the intended posture, stated once here.** ADR-0017 §7 is explicit: *"the
  service owner authored the policy and could equally have written the same call
  into their service code — and running under the caller's authority breaks most
  real policies (the caller usually cannot read the org chart)."* An earlier
  draft of D-B4-4 described nested lookups as "sieve-filtered but not
  after-step-filtered"; that was wrong and more restrictive than the exemption it
  sat next to. Unfiltered-within-its-own-service is the rule.
- **`service_abac` carries no capability.** With the exemption in place a
  `data-layer/read` capability would never be consulted — reads have no
  capability gate, and every gate that does exist (`execute-ddl`, `query-raw`)
  is hard-denied by `read_only` anyway. Carrying it would imply a check that
  doesn't happen. The reason a *new* `AuthLevel` is still needed rather than
  reusing capability-less `service_system` is that the exemption keys on the
  level, and exempting `System` would re-open D-04-02-h's ingress-(ii) bypass.

**Alternative, if the slice must stay small:** ship stage 4 as a *pure
predicate* — the after-step instance gets `service_system` and the `read_only`
flag, so its lookups return nothing and §7's escape hatch is explicitly
deferred. This **fails Failure-matrix row 8** as written ("Stage-4 ABAC
read-only lookup (§7) exceeds its fuel/time budget"), so it needs an explicit
task.md amendment plus a deferred-backlog row, not a silent drop. Concretely,
choosing it deletes: §3.1 entirely (no new `AuthLevel`, no `service_abac` —
`deliver_message`'s `service_system` is reused), §3.6's `resolve_query_auth`
edit, the `stage4_lookup_sees_its_own_service_data` test, and the row-8 test.
Everything else in §3 — the WIT interface, `apply_stage4`, the engine impl,
`read_only`, the budgets, both ingress wirings, the deploy gate — is unchanged.

### D-B4-3 — Which store operations does stage 4 apply to? (blocking for Phase 4)

`task.md` says "candidate rows come from the sieve" and nothing more. Proposed
mapping, all fail-closed:

| Operation | Stage-4 behavior | Why |
|---|---|---|
| `query` (Mode B) | Run over the returned page's rows | The canonical case |
| `get` (Mode A) | Run over the single row, if present | Same shape, batch of 1 |
| `aggregate` | **`permission-denied`** when stage 4 is active | Rows never surface; identical reasoning to the existing CLS denial, `sqlite.rs:636` |
| `delete_many` | **`permission-denied`** when stage 4 is active | Deletion happens inside SQL; the after-step would need the rows materialized first |
| `check_access` (Mode A bool) | **Open — see below** | |
| `resolve-relation` (B3 receiving side) | **Open — see below** | |

**`check_access`** returns a boolean, not rows. Options: (a) deny-closed
(`Ok(false)`) whenever a stage-4 permission is applicable — simple, fail-closed,
but silently makes `check-access` useless under any stage-4 policy; (b) fetch
the row via the Mode-A sieve, run the after-step over it, and answer
`allow|redact ⇒ true`, `deny ⇒ false` — correct, costs one extra row read plus
an instantiation. **Recommend (b)**; it is the only answer that keeps Mode A and
Mode B agreeing on the same policy.

*(rev 2 — the mechanism, which the first draft priced but never described.)*
`ServiceStore::check_access` returns `bool` and never surfaces the row, so (b)
cannot be implemented inside it. It is implemented **at each ingress** as a
substitution, not an addition:

```
let auth = resolve_query_auth(collection, &Ability(operation), Mode::PointInTime{ id })?;
//  ^ note: the *requested* operation, not data-layer/read -- unchanged from today
match auth.and_then(|a| a.resolved_sieve.as_ref()) {
    Some(s) if !s.abac_permissions.is_empty() => {
        // `get` under a Mode::PointInTime sieve runs exactly the predicate
        // `check_access` would have run, and additionally hands back the row.
        let outcome = store.get(collection, id, auth.as_ref()).await?;   // Err -> Ok(false)
        let Some(record) = outcome.value else { return Ok(false) };      // sieve denied
        let kept = apply_stage4(s, session, service_id, collection, authorizer,
                                vec![to_candidate_row(&record)]).await;
        Ok(matches!(kept, Ok(k) if !k.is_empty()))   // Err or empty -> false
    }
    _ => store.check_access(collection, id, operation, auth.as_ref()).await,  // today's path
}
```

`redact` counts as reachable (`kept` is non-empty) — the question `check-access`
answers is "may this caller reach the row", and a redacted row was reached. One
`get`, one instantiation, only on the stage-4 path.

**`resolve-relation`** (`synsvc_native.rs:409`) answers a structural id-set for a
*remote* node's sieve, via `ServiceStore::query` in its A1 branch. If the
definition also carries a stage-4 permission, that id-set is not stage-4
filtered, so a remote sieve could admit a row this node's after-step would
deny. **Recommend:** hard-deny `resolve-relation` for such a definition (the
remote should not be able to route around the after-step), and record the
"stage-4-aware cross-service resolution" gap in the deferred backlog.

*(rev 2 — how, which the first draft left dangling.)* That branch builds
`QueryAuth { resolved_sieve: None, .. }` (`synsvc_native.rs:544`) and lets
`data_db` compile internally (`sqlite.rs:1880`), so there is **no
`CompiledSieve` in hand** to read `abac_permissions` from, and "selected
permissions" is only knowable from a compile. Rather than add a `plan_read`
call to a path that deliberately avoids one, use a **definition-level** check —
deliberately coarser than the per-read `abac_permissions` and therefore
fail-closed:

```rust
// crates/fdae/src/compile.rs, next to `definition_table` (line 666)
/// Whether *any* permission on the definition backing `collection` opts into
/// the stage-4 after-step. Coarser than a compiled sieve's
/// `abac_permissions` (which knows which permissions this caller actually
/// selected) and deliberately so: the one caller is B3's `resolve-relation`,
/// which has no compiled sieve and must fail closed.
pub fn definition_has_abac(policy: &Policy, collection: &str) -> bool
```

Also **A2** (`resolvable_without_capability`, the bare `principal_column` match)
must take the same deny — it bypasses the sieve entirely, so it would otherwise
be the wider hole of the two.

### D-B4-4 — Recursion: stage 4 reading a stage-4-gated collection

*(rev 2 — rewritten; the first draft's `abac_depth` counter is dropped.)*

A §7 lookup from inside `authorize-rows` hits `HostState::resolve_query_auth`
again. If that collection also opts in, the after-step would re-enter itself.

**Under D-B4-2's recommended path this cannot happen, structurally.** The
after-step instance is `AuthLevel::LocalReadOnly`, `resolve_query_auth` returns
`Ok(None)` for it, so its reads carry no `QueryAuth` at all — no sieve, hence no
`abac_permissions`, hence no second after-step. The exemption **is** the
recursion bound. No depth counter, no new `HostState` field.

That makes it a load-bearing property of a one-line early return, so it gets:
(a) a doc comment on the exemption saying so explicitly, and (b) a regression
test that would fail if the exemption were ever narrowed
(`stage4_nested_read_does_not_re_enter_the_after_step`, §4).

**Under D-B4-2's fallback path** (pure predicate, `service_system`) the question
is moot — a capability-less caller compiles to `deny_all()`, the lookup returns
nothing, and there is nothing to recurse into.

### D-B4-5 — Pagination shortening

Post-hoc row removal makes a `query` page shorter than `limit` while
`next_cursor` is still `Some`. That is new behavior: the sieve pushes down, so
today a full page means a full page. Callers must page until `next_cursor` is
`None`. **Recommendation:** document it on the WIT `query` doc comment and in
`traits.rs`; do not attempt to backfill the page (backfilling would mean an
unbounded number of after-step invocations per call).

---

## 2. Phase plan

| Phase | Crates touched | Deliverable |
|---|---|---|
| 1 | `fdae` | Policy opt-in (`authorize_rows`), schema, `CompiledSieve.abac_permissions`, trace fields |
| 2 | `rpc` | `RowAuthorizer` trait + DTOs + `apply_stage4` orchestration helper; `AuthLevel::LocalReadOnly` |
| 3 | `wit_interfaces`, `sandbox_wasm`, `core` | WIT `authorizer` interface; engine `RowAuthorizer` impl; read-only host state; config budgets |
| 4 | `data_db`, `sandbox_wasm`, `control_plane` | Both ingresses wired; `aggregate`/`delete_many` deny arms; deploy-time validation |
| 5 | `test-components`, tests, benches, docs | Fixture component, failure-matrix rows 7–9, criterion bench, doc/backlog updates |

---

## 3. Exact changes

### 3.1 `crates/rpc` — a read-only substrate identity (D-B4-2)

*(rev 2: `AuthLevel`/`CallerContext` live in `crates/rpc/src/native.rs`;
`crates/ucan` owns `SessionContext`/`Capability` and needs no change.)*

**`crates/rpc/src/native.rs:58`** — add a variant to `AuthLevel`:

```rust
pub enum AuthLevel {
    Delegated,
    Ucan,
    LocalElevated,
    /// Substrate-injected stage-4 ABAC context (ADR-0017 §7): the service
    /// acting as itself for the after-step. Carries **no** capabilities and
    /// is exempt from the FDAE sieve, per §7's "the escape hatches run under
    /// the service's own identity". Read-only-ness comes from
    /// `HostState.read_only`, not from a capability -- host write paths
    /// carry no capability gate of their own (D-04-02-f), so a narrower
    /// capability would enforce nothing. Distinct from capability-less
    /// `System` only because the sieve exemption keys on this level, and
    /// exempting `System` would re-open D-04-02-h's ingress-(ii) bypass.
    LocalReadOnly,
    System,
}
```

**`crates/rpc/src/native.rs`, after `local_elevated` (line 82)** — add:

```rust
/// Substrate-injected stage-4 ABAC identity. Never constructible from
/// guest or wire input: `AppSandboxEngine::authorize_rows` is the sole
/// producer, exactly as `invoke_lifecycle_hook` is for `local_elevated`.
/// Deliberately capability-less -- see `AuthLevel::LocalReadOnly`.
#[must_use]
pub fn service_abac(service_id: &str) -> Self {
    Self {
        caller_did: format!("system:abac:{service_id}"),
        app_instance: None,
        session: SessionContext {
            subject_did: format!("system:abac:{service_id}"),
            ..Default::default()
        },
        auth: AuthLevel::LocalReadOnly,
        proof: None,
    }
}
```

**Call sites to audit for the new variant:** checked on `4302827` — every
`AuthLevel::` use in `crates/` and `apps/` is a construction, an `==`, a
`matches!`, or a test assertion; there is **no exhaustive `match`**, so adding
the variant compiles without further edits. Re-verify with
`rg 'AuthLevel::' crates/ apps/` before relying on it.

### 3.2 `crates/fdae` — policy opt-in and sieve/trace plumbing (Phase 1)

**`crates/fdae/src/policy.rs:109` (`Permission`)** — add:

```rust
/// Stage-4 ABAC opt-in (ADR-0017 §7). When true, a read admitted through
/// this permission is additionally passed to the service's guest-exported
/// `authorize-rows` after-step before any row reaches the caller.
/// Restrict-only: the after-step may drop rows or redact fields, never
/// admit a row this permission's `paths:` did not reach.
#[serde(default)]
pub authorize_rows: bool,
```

**`crates/fdae/schema/fdae-v1.json`**, `$defs.permission.properties` — add
`"authorize_rows": { "type": "boolean" }`. (`additionalProperties: false` is set,
so omitting this makes every opt-in policy fail schema validation.)

**`crates/fdae/src/policy.rs::validate_permissions`** — add one semantic rule:
`authorize_rows: true` on a permission with `paths: []` (unconditionally public)
is **allowed** but is exactly the shape the deploy-time lint should flag; no
parse-time error. No other new rule.

**`crates/fdae/src/compile.rs:33` (`CompiledSieve`)** — add:

```rust
/// Applicable permission names that opted into the stage-4 ABAC after-step
/// (ADR-0017 §7, `Permission.authorize_rows`). Empty -- the overwhelmingly
/// common case -- means no after-step: the sieve's rows are final. Non-empty
/// obliges the *ingress* (never `data_db`, which has no WASM engine) to run
/// `authorize-rows` over the candidate rows before returning them, and
/// obliges `aggregate`/`delete_many` to deny closed.
pub abac_permissions: Vec<String>,
```

Populate in `plan_read` immediately after `close_over_includes`
(`compile.rs:459`) and the `applicable.is_empty()` block:

```rust
let abac_permissions: Vec<String> = applicable
    .iter()
    .filter(|name| def.permissions.get(*name).is_some_and(|p| p.authorize_rows))
    .cloned()
    .collect();
```

Thread it into both `ReadPlan` construction arms (`compile.rs:588-613`) and into
`PendingSieve` (`compile.rs:171`). `deny_all()` (`compile.rs:731`) leaves it
empty — a denied read never runs the after-step.

*(rev 2 — `finalize` is **not** unchanged, contrary to the first draft.)* It
destructures `PendingSieve` exhaustively (`compile.rs:290-297`) and rebuilds
`CompiledSieve` with a complete struct literal (`compile.rs:346`); both need the
new field added. Nothing else in `finalize` changes — the field passes straight
through, untouched by marker substitution.

Two more exhaustive literals that will not compile without edits:

- `plan_read`'s `DecisionTrace` construction at `compile.rs:571-585` — a
  complete literal with no `..Default::default()`.
- `deny_all()`'s callers use `CompiledSieve { trace, ..deny_all() }`
  (`compile.rs:446`, `:505`), which keeps working as long as `deny_all()` itself
  sets the new field.

`DecisionTrace` derives `PartialEq`/`Eq` and is compared whole in existing
tests; adding a field with a `Default` empty `Vec` keeps those comparisons
passing, but grep `rg 'DecisionTrace \{' crates/` for literals that will need
the field spelled out.

**`crates/fdae/src/trace.rs`** — add to `DecisionTrace`:

```rust
/// Applicable permissions that opted into the stage-4 after-step. Recorded
/// at compile time; the *outcome* of that step is a separate, execution-aware
/// trace the ingress emits (`AbacTrace`), the same two-trace shape Mode A
/// already uses for `rows_reached`.
pub abac_permissions: Vec<String>,
```

and a sibling struct emitted by the ingress after the after-step runs:

```rust
/// The stage-4 after-step's actual outcome for one read (ADR-0017 §9).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AbacTrace {
    pub permissions: Vec<String>,
    pub rows_in: usize,
    pub rows_denied: usize,
    pub rows_redacted: usize,
    /// `Some(reason)` when the whole batch was denied closed -- missing
    /// export, fuel/epoch overrun, trap, or a malformed decision list.
    pub failed_closed: Option<String>,
}

impl AbacTrace {
    pub fn emit(&self, collection: &str, service_id: &str, subject_did: &str) { /* info! on failed_closed or rows_denied > 0, debug! otherwise */ }
}
```

Export `AbacTrace` from `crates/fdae/src/lib.rs:18`.

Add the `abac_permissions` field to both `DecisionTrace::emit` field lists
(`trace.rs:91` and `trace.rs:108`). `AbacTrace` is a **separate** struct with its
own `emit`, called by the ingress after the after-step runs — it is not a field
on `DecisionTrace`. *(rev 2: the first draft said to add "`abac_permissions` and
`abac` fields"; there is no `abac` field.)*

### 3.3 `crates/rpc` — the after-step seam (Phase 2)

New file **`crates/rpc/src/fdae_abac.rs`**, sibling to `fdae_fetch.rs` and for
the same reason: `crates/fdae` stays engine-free, `crates/data_db` has no WASM
dependency, and `syneroym-rpc` is the one crate both ingresses already depend
on. Note `syneroym-rpc` does **not** depend on `syneroym-wit-interfaces`, so the
DTOs below are rpc-local and each ingress converts its own `RecordReadValue`.

```rust
/// Wall-clock ceiling one after-step invocation may not exceed, enforced by
/// the caller in addition to the engine's own epoch deadline (same
/// belt-and-braces reasoning as `FDAE_FETCH_TIMEOUT` vs. the proxy's own
/// advisory timeout).
pub const FDAE_ABAC_TIMEOUT: Duration = Duration::from_secs(3);

/// Hard cap on one batch, matching `data_db`'s `MAX_QUERY_PAGE_SIZE` -- one
/// page is one batch, so this can never be the binding constraint on a
/// legitimate read, only on a malformed one.
pub const MAX_ABAC_ROWS: usize = 1000;

#[derive(Debug, Clone)]
pub struct AbacAuthContext {
    /// Mirrors the WIT `auth-context` fields of the same name (§3.4): without
    /// them a guest cannot tell which collection or which rule it is being
    /// asked about, since one export serves all of them.
    pub collection: String,
    pub permissions: Vec<String>,
    pub subject_did: String,
    pub anchor_did: Option<String>,
    /// `with::can` strings, the same shape `DecisionTrace.held` uses --
    /// never the caveats, which can carry row-level data.
    pub capabilities: Vec<String>,
    /// `session.claims`, serialized. A JSON string rather than
    /// `list<tuple<string,string>>` because claims are typed scalars
    /// (`serde_json::Map<String, Value>`) that a string-pair list would
    /// silently flatten.
    pub claims_json: String,
}

impl AbacAuthContext {
    pub fn build(session: &SessionContext, collection: &str, permissions: &[String]) -> Self { /* ... */ }
}

#[derive(Debug, Clone)]
pub struct CandidateRow {
    pub id: String,
    pub payload: Vec<u8>,
    pub creator_id: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowDecision {
    Allow,
    Deny,
    /// Top-level payload keys to strip. Dotted paths are rejected (H3): a
    /// dotted entry would silently mask nothing, so it denies the row.
    Redact(Vec<String>),
}

#[derive(Debug, thiserror::Error)]
pub enum AbacError {
    #[error("no row authorizer is available for service '{0}' (stage-4 policy on a non-WASM or unwired service)")]
    Unavailable(String),
    #[error("service '{0}' does not export syneroym:data-layer/authorizer#authorize-rows")]
    MissingExport(String),
    #[error("stage-4 after-step for '{service}' exceeded its budget: {detail}")]
    BudgetExceeded { service: String, detail: String },
    #[error("stage-4 after-step for '{service}' trapped: {detail}")]
    Trap { service: String, detail: String },
    #[error("stage-4 after-step returned {got} decisions for {expected} rows")]
    ArityMismatch { expected: usize, got: usize },
    #[error("stage-4 after-step returned a malformed decision: {0}")]
    Malformed(String),
    #[error("batch of {0} rows exceeds the {MAX_ABAC_ROWS}-row cap")]
    BatchTooLarge(usize),
}

#[async_trait::async_trait]
pub trait RowAuthorizer: Send + Sync + fmt::Debug {
    /// Invokes `service_id`'s guest-exported `authorize-rows` over one
    /// batch. Returns exactly `rows.len()` decisions, positionally aligned;
    /// any other outcome is an `AbacError` the caller must treat as a
    /// whole-batch deny.
    async fn authorize_rows(
        &self,
        service_id: &str,
        ctx: &AbacAuthContext,
        rows: &[CandidateRow],
    ) -> Result<Vec<RowDecision>, AbacError>;
}

/// An always-empty `Weak<dyn RowAuthorizer>` for contexts with no WASM
/// engine (coordinator mode, tests) -- same unsized-coercion trick as
/// `host_capabilities::empty_service_proxy`.
pub fn empty_row_authorizer() -> Weak<dyn RowAuthorizer> { /* ... */ }
```

The orchestration helper both ingresses call:

```rust
/// Runs the stage-4 after-step over `rows` when `sieve` opts in, and returns
/// the surviving rows each paired with the **extra** field names to strip
/// (the after-step's `redact` set; the caller unions this with the sieve's
/// own `masked_fields` before projecting).
///
/// **Restrict-only by construction**: the guest is only ever shown rows the
/// sieve already admitted and can only answer per-position, so there is no
/// representation for "also allow row X". A decision list that doesn't align
/// 1:1 with `rows` is the one shape that could smuggle an extra row in, and
/// it denies the entire batch.
///
/// **Fail-closed on everything**: missing export, unavailable authorizer,
/// budget overrun, trap, malformed decision, over-large batch -- all return
/// `Err`, which every caller maps to "no rows" (Mode B) / `false` (Mode A),
/// never to "unfiltered".
pub async fn apply_stage4(
    sieve: &CompiledSieve,
    session: &SessionContext,
    service_id: &str,
    collection: &str,
    authorizer: Option<Arc<dyn RowAuthorizer>>,
    rows: Vec<CandidateRow>,
) -> Result<Vec<(CandidateRow, Vec<String>)>, AbacError>
```

Pseudo-code:

```
if sieve.abac_permissions.is_empty() { return Ok(rows.map(|r| (r, vec![]))) }
let mut trace = AbacTrace { permissions: sieve.abac_permissions.clone(), rows_in: rows.len(), ..default };

if rows.len() > MAX_ABAC_ROWS      -> trace.failed_closed = ...; emit; Err(BatchTooLarge)
let Some(auth) = authorizer        else -> trace.failed_closed = ...; emit; Err(Unavailable)
if rows.is_empty()                 -> emit; return Ok(vec![])   // nothing to ask about

let ctx = AbacAuthContext::build(session, collection, &sieve.abac_permissions);
let decisions = time::timeout(FDAE_ABAC_TIMEOUT, auth.authorize_rows(service_id, &ctx, &rows))
    .await
    .map_err(|_| BudgetExceeded{..})??;          // outer: our own deadline; inner: engine's

if decisions.len() != rows.len()   -> trace.failed_closed = ...; emit; Err(ArityMismatch)

let mut kept = Vec::with_capacity(rows.len());
for (row, decision) in rows.into_iter().zip(decisions) {
    match decision {
        Allow          => kept.push((row, vec![])),
        Deny           => trace.rows_denied += 1,
        Redact(fields) => {
            if fields.iter().any(|f| f.contains('.')) {   // H3: dotted paths mask nothing
                trace.failed_closed = Some("redact named a dotted path".into());
                trace.emit(..); return Err(Malformed(..));
            }
            trace.rows_redacted += 1;
            kept.push((row, fields));
        }
    }
}
trace.emit(collection, service_id, &session.subject_did);
Ok(kept)
```

Register the module in `crates/rpc/src/lib.rs` and re-export
`AbacAuthContext, AbacError, CandidateRow, RowAuthorizer, RowDecision,
apply_stage4, empty_row_authorizer, FDAE_ABAC_TIMEOUT, MAX_ABAC_ROWS`
alongside the existing `fdae_fetch` re-exports.

### 3.4 `crates/wit_interfaces` — the guest export (Phase 3)

New file **`crates/wit_interfaces/wit/data-layer/authorizer.wit`**. The
`wit/data-layer/` directory is currently a single-file package
(`data-layer.wit:1` declares `package syneroym:data-layer@0.1.0;`); a second
file in the same directory joins that same package, so the new file repeats the
identical `package` line and adds no new package:

```wit
package syneroym:data-layer@0.1.0;

/// Stage-4 ABAC after-step (ADR-0017 §7). A service *may* export this; a
/// component that doesn't is simply never asked. It is deliberately NOT part
/// of the `host-environment` world -- adding it there would make every
/// deployed component required to implement it.
interface authorizer {
    record auth-context {
        /// The collection whose rows these are. Required: one exported
        /// function serves every opted-in permission on every collection
        /// (§0.1), so without this the guest cannot tell what it is being
        /// asked about. Not in ADR-0017 §7's illustrative shape (§5 item 1).
        collection: string,
        /// The opted-in permissions that selected this row set, so a guest
        /// can apply different logic per rule. Same reason as `collection`.
        permissions: list<string>,
        subject-did: string,
        anchor-did: option<string>,
        capabilities: list<string>,
        claims-json: string,
    }

    record candidate-row {
        id: string,
        payload: list<u8>,
        creator-id: string,
        created-at: u64,
        updated-at: u64,
    }

    variant row-decision {
        allow,
        deny,
        redact(list<string>),
    }

    /// Restrict-only: every row passed in was already admitted by the SQL
    /// sieve, and decisions are positional, so there is no way to admit a
    /// row that isn't in `rows`. A returned list whose length differs from
    /// `rows`, or an `err`, denies the whole batch.
    authorize-rows: func(ctx: auth-context, rows: list<candidate-row>)
        -> result<list<row-decision>, string>;
}

world data-layer-authorizer {
    export authorizer;
}
```

The host does **not** need bindgen types for this: the engine builds `Val`s by
hand and looks the export up dynamically, exactly as `deliver_message` does for
`guest-api::handle-message` (`engine.rs:1022-1043`). Do **not** touch
`wit/host/host.wit`'s `host-environment` world.

Copy the file into `test-components/<fixture>/wit/deps/data-layer/` for the
fixture component (§3.8) and regenerate its checked-in `src/bindings.rs`.

### 3.5 `crates/core` — budgets (Phase 3)

**`crates/core/src/config.rs`, `AppSandboxRole` (line 425)** — add two fields
plus their `default_*` fns and `Default` entries:

```rust
/// Fuel ceiling for one stage-4 ABAC after-step invocation (ADR-0017 §7's
/// "fuel-metered"). Deliberately a small fraction of
/// `default_max_instructions`: the after-step runs once per read on the hot
/// path, and §7's optional read-only lookups are the thing this bounds.
/// Overrun denies the whole batch, never returns partially-checked rows.
pub abac_max_instructions: Option<u64>,      // default Some(50_000_000)

/// Wall-clock budget for one after-step. Tighter than
/// `dispatch_epoch_timeout_secs` for the same reason.
pub abac_epoch_timeout_secs: u64,            // default 2
```

Defaults are a starting point to be re-tuned against Phase 5's criterion bench;
say so in the doc comment.

### 3.6 `crates/sandbox_wasm` — engine + read-only host state (Phase 3)

**`host_capabilities.rs:93` (`HostState`)** — add two fields. *(rev 2: the first
draft had three; `abac_depth` is dropped per D-B4-4 — the sieve exemption
already makes recursion structurally impossible, so a counter would be a second
mechanism guarding nothing.)*

```rust
/// Stage-4 after-step instances (`AuthLevel::LocalReadOnly`) get this set:
/// every mutating and egress host function hard-denies. Not derivable from
/// `caller.auth` alone -- write host paths carry no capability gate today
/// (D-04-02-f), so the check has to live somewhere that isn't the
/// capability layer.
pub read_only: bool,
/// Weak handle to the after-step invoker. `Weak`, not `Arc`: the only
/// implementation is `AppSandboxEngine`, which owns this state's `Store`
/// -- same cycle reasoning as `service_proxy`.
pub row_authorizer: Weak<dyn RowAuthorizer>,
```

Extend `HostState::new` (`host_capabilities.rs:133`) with the two (it is already
`#[allow(clippy::too_many_arguments)]` at 11 args; adding two more positionally
is worse than it looks — prefer threading them as one `InstanceOptions` value,
the same struct the engine already needs below).

**`host_capabilities.rs::resolve_query_auth` (line 206)** — one edit:

Extend the `LocalElevated` exemption to `LocalReadOnly`:
`if matches!(self.caller.auth, AuthLevel::LocalElevated | AuthLevel::LocalReadOnly) { return Ok(None); }`

The existing doc comment (lines 189-205) explains at length why `LocalElevated`
is exempt while `System` deliberately is not; extend it in the same register
with: the after-step's own lookups are ADR-0017 §7's declared escape hatch,
running under the service's own identity and *intentionally* unfiltered within
its own service (D-B4-2); read-only-ness comes from `HostState.read_only`, not
from the sieve; and — the part that must not be lost — **this early return is
also what bounds after-step recursion** (D-B4-4), because a read with no
`QueryAuth` has no sieve and therefore no `abac_permissions` to trigger a second
after-step. Narrowing this exemption without replacing that bound reintroduces
unbounded recursion.

**Read-only denials** — at the top of each of these `impl` methods in
`host_capabilities.rs`, `if self.read_only { return Err(<Denied>); }`:

| Interface | Methods |
|---|---|
| `store::Host` | `create_collection`, `drop_collection`, `put`, `patch`, `delete`, `delete_many`, `batch_mutate`, `execute_ddl`, `query_raw` |
| `blob_store::Host` (line 843) | `put_blob`, `delete_blob`, `open_upload`; `HostBlobWriter::{write, finish}` (line 898) |
| `host_api::Host` (line 309) | `publish`, `subscribe`, `unsubscribe` (a subscription registered from a throw-away instance would outlive it) |
| `proxy::Host` | `call` — §7 is *local* read-only lookups; a cross-service call mid-query is exactly the N+1-over-the-network the ADR was worried about |
| stream host fns (`stream.rs`) | all |

`vault::reveal`, `app_config`, and the read paths (`get`/`query`/`aggregate`/
`check_access`) stay allowed.

**`engine.rs`** — new fields on `AppSandboxEngine` next to
`dispatch_epoch_ticks` (line 175): `abac_epoch_ticks: u64`,
`abac_max_instructions: Option<u64>`, populated in `init`'s `Self { .. }`
literal (line 295) from the new config. *(rev 2)* There are **two**
hand-written test constructors that also spell the struct out in full and will
not compile without the new fields: `engine.rs:1933` and the
`test_app_engine` helper at `engine.rs:1996`.

**`engine.rs::build_store_and_instantiate` (line 741)** — add a parameter
carrying the after-step's differences rather than a fourth positional argument:

```rust
#[derive(Debug, Clone, Copy, Default)]
struct InstanceOptions {
    /// Overrides the service's own quota-derived fuel. `None` keeps it.
    fuel_override: Option<u64>,
    read_only: bool,
}
```

*(rev 2 — corrected call-site list; the first draft's `:1341`/`:1372` were
`GuestStreamCursor::new`/`GuestStreamSink::new`, which take
`dispatch_epoch_ticks` but never call this function.)* The **four** actual call
sites are `engine.rs:868` (`prepare_wasm_execution`), `:887`
(`invoke_lifecycle_hook`), `:993` (`deliver_message`), and `:1198` (the stream
instance) — all pass `InstanceOptions::default()`. Inside,
`store.set_fuel(opts.fuel_override.or(max_instructions))` and `read_only` comes
from `opts`.

**`engine.rs::prepare_wasm_execution` (line 861)** — extend the existing
`debug_assert!` to bar a forwarded `LocalReadOnly` too, for the same reason it
bars `LocalElevated`.

**New `engine.rs` method + trait impl:**

```rust
const AUTHORIZER_INTERFACE: &str = "syneroym:data-layer/authorizer@0.1.0";

#[async_trait::async_trait]
impl RowAuthorizer for AppSandboxEngine {
    async fn authorize_rows(&self, service_id: &str, ctx: &AbacAuthContext, rows: &[CandidateRow])
        -> Result<Vec<RowDecision>, AbacError>
    {
        // 1. instantiate: service_abac caller, abac_epoch_ticks,
        //    InstanceOptions { fuel_override: self.abac_max_instructions,
        //                      read_only: true }
        //    -> instantiation failure maps to AbacError::Unavailable
        // 2. get_wasm_func(Some(AUTHORIZER_INTERFACE), "authorize-rows")
        //    -> Err maps to AbacError::MissingExport (NOT retried; a missing
        //       export can't become present)
        // 3. args = [Val::Record(ctx fields), Val::List(rows as Val::Record)]
        // 4. func.call_async -> map Trap::OutOfFuel / "all fuel consumed" /
        //    "out of fuel" / epoch-deadline traps to AbacError::BudgetExceeded,
        //    everything else to AbacError::Trap.  (Reuse the exact string
        //    matching in execute_wasm_vals:646-659 -- do not invent a second
        //    classification.)
        // 5. results[0] is result<list<row-decision>, string>:
        //    Err(msg) -> AbacError::Trap { detail: msg }
        //    Ok(list) -> map each Val::Variant("allow"|"deny"|"redact", _)
        //                to RowDecision; anything else -> AbacError::Malformed
        // 6. record metrics: substrate.fdae.abac_ms histogram,
        //    substrate.fdae.abac_rows_denied counter
    }
}
```

No retry loop (unlike `deliver_message`): a retried after-step would double the
worst-case latency of a hot-path read, and every failure mode here is already
deny-closed.

Also add a small helper the deploy path needs:

```rust
/// Whether `service_id`'s compiled component exports the stage-4 after-step.
/// Cheap: inspects the cached `InstancePre`'s component type, no
/// instantiation.
pub fn exports_authorize_rows(&self, service_id: &str) -> bool
```

**Wiring `row_authorizer` into `HostState`**: `build_store_and_instantiate`
already holds `self.self_weak` (used for `MessagingContext.engine`). Coerce it:
`let row_authorizer: Weak<dyn RowAuthorizer> = self.self_weak.get().cloned().unwrap_or_default();`

### 3.7 Ingress wiring (Phase 4)

**`crates/data_db`** — three edits:

1. `sqlite.rs::do_aggregate` (line 636): extend the CLS deny to
   `if let Some(s) = sieve && (!s.masked_fields.is_empty() || !s.abac_permissions.is_empty())`,
   with the reason in the doc comment above (line 617).
2. `sqlite.rs::do_delete_many` (line 241): add the same
   `!s.abac_permissions.is_empty() -> PermissionDenied` guard.
3. `traits.rs` doc comments on `query` (line 174) and `get` (line 163): note
   that the returned rows are the *sieve's* output and are still subject to the
   ingress's stage-4 after-step; `data_db` deliberately does not run it (no WASM
   engine here, same separation as `resolve_fetches`).

**`crates/sandbox_wasm/src/host_capabilities.rs`** — ingress (i):

```rust
// store::Host::query, after `store.query(...)`:
let extra = if let Some(sieve) = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref()) {
    let rows: Vec<CandidateRow> = outcome.value.records.iter().map(to_candidate_row).collect();
    match syneroym_rpc::apply_stage4(sieve, session, service_id, &collection,
                                     self.row_authorizer.upgrade(), rows).await {
        Ok(kept) => kept,
        Err(_)  => return Ok(QueryResult { records: vec![], next_cursor: None }),  // deny-closed
    }
} else { /* pass-through */ };
// then: strip_record(record, &union(outcome.masked_fields, per_row_extra))
```

Two mechanical constraints, both already documented at
`host_capabilities.rs:216-223`: the `Send`-future requirement means `session`
and `service_id` must be copied into owned locals **before** the `.await`, and
`self` must not be touched after it — so `self.row_authorizer.upgrade()` happens
before the await too.

Same edit in `store::Host::get` (line 531), batch of 0-or-1.

`store::Host::check_access` (line 637): the substitution spelled out in
D-B4-3 — copy that pseudo-code, don't re-derive it.

**`crates/control_plane/src/synsvc_native.rs`** — ingress (ii), identical logic
in the `"get"` (line 645), `"query"` (line 672) and `"check-access"` arms.
`SynSvcNativeService` needs a new `row_authorizer: Weak<dyn RowAuthorizer>`
field alongside `service_proxy` (line 296), set by whoever constructs it — find
the constructor call sites with `rg 'SynSvcNativeService::new'`.

*(rev 2 — two corrections.)* The optional dependency's feature is
**`app_sandbox`** (`control_plane/Cargo.toml:11`), not `wasm`. And the reason the
trait lives in `syneroym-rpc` is narrower than the first draft claimed: it is
that **`synsvc_native.rs`** never names `AppSandboxEngine` and must not start —
`orchestration.rs` already calls `self.app_sandbox_engine.deploy_wasm(...)`
directly (line 242) and will call `exports_authorize_rows` the same way.

Also per D-B4-3, `resolve_relation` (line 409) gets a deny arm — via
`syneroym_fdae::definition_has_abac`, not via a sieve it doesn't have — covering
**both** its A1 and A2 branches.

**`crates/control_plane/src/dummy_sandbox.rs`** — `AppSandboxEngine` is a
no-op unit struct under `#[cfg(not(feature = "app_sandbox"))]` (line 18). Add a
matching `exports_authorize_rows(&self, _service_id: &str) -> bool { false }`
stub, so a policy that opts in fails the deploy gate in a sandbox-less build
rather than failing to compile it. This is the whole point of that file's
"use the engine unconditionally without `#[cfg]` spam" contract.

**`crates/control_plane/src/service/orchestration.rs`** — deploy-time
validation, next to the two existing lints (lines 81, 120):

```rust
/// A policy that opts a permission into the stage-4 after-step
/// (`authorize_rows: true`) but whose component doesn't export
/// `syneroym:data-layer/authorizer#authorize-rows` would deny **every** read
/// through that permission at runtime (fail-closed, ADR-0017 §8). Failing
/// the deploy is strictly better than shipping a service that returns
/// nothing -- unlike the two warn-only lints above, this has no legitimate
/// shape.
fn validate_stage4_export(...) -> Result<(), String>
```

- WASM service type: reject when the policy opts in and
  `engine.exports_authorize_rows(&service_id)` is false.
- TCP/container service type: reject any `authorize_rows: true` outright (no
  guest to call).
- Placement: in `deploy`, after `parse_and_validate` (line 506) and after the
  component is compiled/cached, but **before** `save_fdae_policy` (line 551) —
  otherwise a rejected deploy still needs `rollback_fdae_policy`.

### 3.8 Test fixture (Phase 5)

New `test-components/abac-test/`. The root `Cargo.toml` has
`members = [..., "test-components/*"]` (line 3) and an explicit `exclude` list
(lines 4–10) for the fixtures that cross-compile to `wasm32-wasip2` — the new
directory **must** be added to that `exclude` list or it breaks the workspace
build. It exports `authorizer` plus a `test-driver` interface, and its behavior is
switchable via a record it reads from its own `profiles`-style collection or
from `app-config`, so one component covers every matrix row:

| Mode | `authorize-rows` behavior | Covers |
|---|---|---|
| `allow_all` | `allow` for every row | Baseline + row 7 (cannot widen) |
| `deny_by_field` | `deny` when `payload.classification == "secret"` | The motivating case |
| `redact` | `redact(["ssn"])` | Row 9 |
| `spin` | infinite loop | Row 8 (fuel/epoch) |
| `bad_arity` | returns one fewer decision than rows | Row 7's structural guard |
| `lookup` | calls `store::query` on a second collection, then decides | §7 read-only lookup |
| `write_attempt` | calls `store::put` | Read-only enforcement |

Follow `test-components/data-layer-test`'s layout exactly (checked-in
`src/bindings.rs`, `wit/deps/`), and add the artifact path to
`crates/core/src/test_constants.rs` next to `data_layer_test_wasm_path` (line
17). Tests skip with the same "artifact not found, run `cargo build --target
wasm32-wasip2 --release`" message the existing ones use
(`data_layer_integration.rs:139`).

---

## 4. Tests

### Unit — `crates/fdae`
- `policy::tests::authorize_rows_defaults_to_false_when_absent` — *(rev 2)*
  parse-only. `Permission` derives `Deserialize` but **not** `Serialize`
  (`policy.rs:107`), so a round-trip test would mean adding `Serialize` to the
  policy model purely for a test; not worth it.
- `policy::tests::rejects_an_unknown_permission_key` — confirms
  `deny_unknown_fields` + the schema's `additionalProperties: false` still bite
  after the schema edit (a typo'd `authorise_rows` must fail loudly, not
  silently disable the after-step)
- `compile::tests::abac_permissions_lists_only_opted_in_applicable_permissions`
- `compile::tests::deny_all_carries_no_abac_permissions`
- `compile::tests::finalize_preserves_abac_permissions_through_a_remote_fetch`

### Unit — `crates/rpc::fdae_abac`
- `apply_stage4_is_a_pass_through_when_no_permission_opts_in`
- `arity_mismatch_denies_the_whole_batch` *(matrix row 7)*
- `deny_drops_only_its_own_row`
- `redact_unions_with_the_sieve_mask_and_never_subtracts` *(row 9)*
- `a_dotted_redact_path_denies_the_row` (H3 precedent)
- `an_unavailable_authorizer_denies_closed`
- `an_over_large_batch_denies_closed`
- `an_elapsed_timeout_denies_closed` *(row 8, host half)*

### Unit — `crates/data_db`
- `tests_fdae::aggregate_denies_closed_under_a_stage4_policy`
- `tests_fdae::delete_many_denies_closed_under_a_stage4_policy`

### Integration — `crates/sandbox_wasm/tests/`
- `stage4_denies_rows_the_guest_rejects_for_a_real_caller` (ingress i)
- `stage4_cannot_admit_a_row_the_sieve_excluded` *(row 7, end to end)*
- `stage4_redact_removes_the_named_field_before_the_guest_sees_it` *(row 9)*
- `stage4_fuel_exhaustion_denies_the_whole_batch` *(row 8)*
- `stage4_missing_export_under_an_opted_in_policy_denies_closed`
- `stage4_instance_cannot_write` (read-only enforcement)
- `stage4_nested_read_does_not_re_enter_the_after_step` (D-B4-4)
- `stage4_lookup_sees_its_own_service_data` (§7, only if D-B4-2 takes the
  recommended path)

### Integration — `crates/router/tests/`
- `guest_self_proxy_data_layer_applies_stage4` (ingress ii), modelled on
  `proxy_dispatch.rs::guest_self_proxy_data_layer_filters_for_a_real_caller_d04_02_h_closed`

### Regression surface for the `resolve-relation` deny (D-B4-3)
Hard-denying `resolve-relation` for a stage-4 definition is a **cross-node
behavior change**, so B3's own proofs are the surface to re-run, not just the
new tests: `crates/substrate/tests/federated_fdae_e2e.rs::federated_fdae_fetch_across_two_real_substrates`
and its asserter-mismatch scenario, plus
`router::native_dispatch_identity::native_dispatch_denies_closed_on_a_cross_service_fetch_failure`.
Add a case where the *remote* definition carries a stage-4 permission and assert
the fetch denies closed rather than returning an unfiltered id-set.

### Deploy — `crates/control_plane`
- `service::orchestration::tests::test_stage4_policy_without_the_export_fails_deploy`
- `..._on_a_tcp_service_fails_deploy`
- `..._rolls_back_the_policy_row_on_rejection`

### Bench — Performance Budgets row 3
`crates/sandbox_wasm/benches/` (new `abac_bench.rs`, or extend
`data_layer_bench.rs`): after-step over 1/10/100/1000-row batches, reporting the
instantiation cost separately from the guest's own execution — the B3 Phase-5
finding was that per-call setup dominates, and the same is likely here. Record
the measured number in `task.md`'s Performance Budgets table and in
`PERF_SUMMARY.md`.

---

## 5. Things in `task.md`/ADR-0017 that are stale or under-specified

Flagged rather than guessed, per the request.

1. **ADR-0017 §7's WIT sketch is labelled "Illustrative"** and returns a bare
   `list<row-decision>` with `auth-context = { subject-did, capabilities, claims }`.
   §3.4 above changes it in three ways: `result<..., string>` (matches every
   other guest export here and gives the trace a message), `anchor-did` added
   (ADR-0015 A5 landed *after* the ADR was written — without it a stage-4
   function cannot tell a proxied read from a direct one, the exact distinction
   `RemoteFetch.principal_did` and `DecisionTrace.anchor_did` exist to make),
   and `claims` as a JSON string. Worth a dated ADR-0017 amendment, in the style
   of the two already there.

2. **ADR-0017 §7 says lookups are "Bounded by ADR-0005's existing fuel quota."**
   That quota is *per invocation of the service's own entry point*
   (`store.set_fuel(max_instructions)` in `build_store_and_instantiate`), and
   the after-step is a **separate** instantiation, so it would silently get a
   *second* full 10-billion-instruction budget rather than being bounded by the
   caller's. §3.5's dedicated `abac_max_instructions` is the fix; the ADR
   sentence is inaccurate as written against `main`.

3. **ADR-0017 §7's "read-only lookups" has no mechanism** and, as §1/D-B4-2
   shows, cannot be satisfied by any identity that exists today: `service_system`
   reads nothing, `local_elevated` reads everything *and* writes. The ADR never
   says which. This is the single biggest unresolved item in the slice.

4. **`task.md`'s Performance Budgets row 3** ("Stage-4 ABAC over a candidate
   batch — Document measured; must not dominate Mode-B query latency") sets no
   number. Given that one after-step costs a full component instantiation, "must
   not dominate" may not be achievable for small pages; propose replacing it
   with a measured figure plus an explicit statement that stage 4 is opt-in and
   its cost is the price of opting in.

5. **`task.md`'s Failure matrix row 7** ("Stage-4 ABAC attempts to widen access
   beyond ReBAC → Rejected") describes a scenario the chosen shape makes
   *unrepresentable* — a positional decision list over rows the sieve already
   picked has no "widen" encoding. The nearest real test is the arity guard plus
   an end-to-end assertion that an `allow`-everything guest still cannot see a
   sieve-excluded row. Worth rewording the row so the evidence matches the
   claim.

6. **ADR-0017 §7 "opt-in per rule"** — "rule" is not a term in the schema. §3.2
   reads it as **per `permissions:` entry**, which is the only granularity the
   compiler's `applicable` set can express. Confirm.

7. **`task.md` line 783: "May fold into B2's design if it stays small."** It
   does not stay small — it adds a WIT interface, a new substrate identity
   level, a read-only host-state mode, a config pair, a deploy-time gate, and a
   test fixture component. Treat B4 as its own slice with the phases above.

8. **`data_db/src/auth.rs:32`'s "per the stage-4 ordering contract"** is the one
   existing code comment that anticipates this slice, and it implies the
   ordering §3.7 uses (sieve → stage 4 on *unmasked* rows → union CLS mask with
   redact set → strip once). Nothing else states that ordering; this plan makes
   it explicit and it should end up in a doc comment, not just a plan.

9. **`resolve-relation` and stage 4** (D-B4-3): B3's receiving side predates
   this slice and answers a raw id-set. Nothing in `task.md` or the B3 plan says
   what should happen when the same definition also carries an after-step.

10. **D-04-02-g (multi-capability caveats are accidentally intersective)** is
    still open and now touches stage 4: `AbacAuthContext.capabilities` flattens
    the same `entitling_caps` list, so a stage-4 function reasoning about "which
    grant admitted this caller" inherits that flattening. Not a B4 blocker — a
    note so it isn't rediscovered.

11. **`task.md`'s exit criterion (line 915) still says "pure-predicate"** —
    *"Stage-4 ABAC wired: pure-predicate, batched, restrict-only default;
    redact/deny tested."* ADR-0017 §7 explicitly relaxed the pure-predicate ban
    (only the N+1 argument survived), and this plan follows the ADR. Left
    unamended, the slice would ship "done" against a criterion it deliberately
    doesn't meet. Reword to "may issue fuel-metered read-only lookups" — or, if
    D-B4-2 takes the fallback path, keep the wording and record §7's hatch as a
    deferral. Either way it is an edit, not a no-op. *(Added §6 checklist item.)*

12. **Slice ordering interaction with B5-fdae, stated in neither doc.** B5
    threads `caller.session` into `put`/`patch`/`delete`/`batch_mutate` and
    gates them on Mode-A `check_access` — the same methods §3.6 hard-denies
    under `read_only`. Two consequences worth writing down now: (a) the
    `read_only` check must run **before** B5's capability gate, so an after-step
    instance is refused on the flag rather than on a capability evaluation it
    would also fail; (b) if D-B4-3 resolves `check_access` to run the after-step
    (option b, recommended), then **every B5 write authorization becomes a
    stage-4 call site** — B4's `check_access` answer silently becomes a
    prerequisite for B5's semantics and its per-write cost. `task.md`'s slice
    order (line 418, B4 → B5) permits this; nothing currently says it.

---

## 6. Completion checklist

- [ ] `cargo +nightly fmt --all`
- [ ] `cargo clippy --workspace --all-targets --all-features` clean
- [ ] `cargo test --workspace`
- [ ] `mise run test:e2e`
- [ ] Import-cleanup pass over every edited file (types via `use`, functions
      qualified by parent module, no inline multi-`::` paths)
- [ ] `docs/planning/deferred-backlog.md`: move the existing **"FDAE stage-4
      WASM ABAC"** row (line 52) to *Recently resolved*; add new rows for
      **`aggregate` and `delete_many` denying closed under a stage-4 policy**
      (§3.7 — new user-visible restriction, defensible by the CLS precedent at
      `sqlite.rs:636` but a real capability gap that needs recording under the
      Mandatory Deferred-Backlog rule), for **`resolve-relation` denying closed**
      and its coarser definition-level granularity (D-B4-3), for the
      **pagination-shortening** consequence (D-B4-5), and — only if D-B4-2 takes
      the fallback — for **§7 read-only lookups**
- [ ] `task.md`: flip Failure-matrix rows 7–9 from ⛔ Deferred to ✅ with test
      names; **reword row 7** so its claim matches what the design can actually
      evidence (§5 item 5); fill Performance Budgets row 3 with the measured
      number; **amend the "pure-predicate" exit criterion at line 915** (§5 item
      11); add the Slice B4-fdae completion entry
- [ ] `docs/planning/traceability-matrix.md`: update the `[FND-IAM]` row
- [ ] ADR-0017: dated amendment for the §7 WIT shape and the fuel-budget
      correction (§5 items 1–3)
- [ ] `status.md`: implementation/verification evidence entry, matching the
      B2/B3 format
