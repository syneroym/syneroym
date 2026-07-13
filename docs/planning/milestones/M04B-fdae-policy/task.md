# Milestone 4B: FDAE Data-Aware Authorization (M04B-fdae-policy)

> **Provenance.** Split on 2026-07-13 from a combined M4 planning draft along
> the **capability-plumbing vs. data-aware-policy-engine** boundary. The
> authentication + capability foundation this milestone builds on —
> verified caller-identity threading, UCAN context, the Universal Proxy — is the
> sibling milestone
> [M04A-proxy-and-auth-foundation](../M04A-proxy-and-auth-foundation/task.md).
> Code anchors re-verified against `main` @ `6c6e859`.
>
> **What this milestone is.** The Federated Data-Aware Authorization Engine
> (FDAE): declarative ReBAC policy compiled into SQLite so unauthorized rows and
> columns never reach the WASM guest — Row-Level and Column-Level Security via
> the "pushdown sieve" — plus the 4-stage hybrid pipeline (UCAN context →
> cross-service parameter fetch → SQL sieve → optional WASM ABAC). It carries
> **no** M3 carried-forward debt (all five M3→M4 gate items closed in M04A); it
> is purely the new policy engine.

## Goal

Layer FDAE on top of M04A's identity/capability foundation so that Tier 3 —
data-plane row/column filtering — is enforced at query-compilation time. By the
end of M04B, a `data-layer::query` is transparently filtered by the caller's
compiled ReBAC policy: unauthorized rows are pruned inside SQLite (Mode B), and
point-in-time resource checks return a swift Allow/Deny (Mode A), with
cross-service relationship proofs fetched mid-evaluation via the Universal Proxy.

---

## Requirement IDs (Traceability)

| Requirement ID | Sub-scope in M04B | Current matrix status |
|---|---|---|
| `[FND-IAM]` (policy engine) | Declarative Zanzibar-style policy schema; ReBAC→SQL compilation (`WHERE EXISTS`/`WITH RECURSIVE`); RLS + CLS; the 4-stage hybrid pipeline; stage-4 WASM ABAC (`system-requirements-spec.md:971-985`, `system-architecture.md:1826-1848`) | No matrix row exists |

M04A carries the `[FND-IAM]` *foundation* row (identity threading, UCAN context,
Admin capability). M04B carries the `[FND-IAM]` *data-aware-authorization* row.

---

## Dependency on M04A & Co-design Seams

**M04B implementation follows M04A** — hard dependencies:

- **B0's `NativeInvocation` caller-identity field** — Tier 3 filters *against* the
  caller identity; without B0 there is nothing to filter by (today
  `creator_id = self.service_id`, the DB owner, not the caller).
- **B1's SessionContext** — the pushdown compiler binds its normalized
  scopes/claims as SQL `?` parameters (`system-architecture.md:978`).
- **A1's Universal Proxy** — Slice B3's stage-2 cross-service relationship-proof
  fetch rides it.

**M04B *design* (ADR D-04-02) runs in parallel with M04A implementation.** Two
contracts are built in M04A but consumed here; design them jointly (D-04-01 ↔
D-04-02 as a pair):

1. **SessionContext / capability representation** — shape M04A's B1 output so the
   pushdown compiler can bind scopes/claims as `?` parameters.
2. **Universal Proxy request shape** — ensure M04A's A1 can carry a
   relationship-proof fetch (Slice B3), not only an ordinary call.

---

## Decision Register

### B. Blocking — new ADR required before Slice B2 begins

- **D-04-02 — FDAE Policy Schema & Compilation Strategy.** The core design work
  of this milestone. Must settle:
  - The declarative Zanzibar-style policy config format (YAML/JSON) and the
    **typed policy model** it deserializes into (no runtime string lexers —
    `system-architecture.md:1832-1836`), versioned + JSON-Schema-validated at
    deploy (as `app-config` already is).
  - **Two-layer naming** (see FDAE Enforcement Model below): a portable *logical*
    vocabulary (object types, relations, permissions, field names) resolved at
    compile time against a *physical registry* binding (logical name → service
    DID → SQLite DB → table → column) — `system-architecture.md:1834,1881-1883`.
  - How ReBAC relationship chains compile to `WHERE EXISTS` / cycle-protected
    `WITH RECURSIVE` (the "Pushdown Sieve"), the `visited_track` cycle guard, and
    the `sqlite3_progress_handler` watchdog + policy-configurable time budget
    with **default-deny** on timeout (`system-architecture.md:1847-1848`).
  - **RLS vs. CLS** — both, compiling differently: RLS → row-pruning subquery;
    CLS → column projection/masking.
  - The **stage-4 ABAC** WIT signature and its guardrail (below).
  - How the compiled FDAE security subquery **merges** with the caller's
    ADR-0007 MongoDB-style JSON query filter at SQL generation (two compilers,
    one `AND` at the end).

  Resolve as an ADR (per `session-strategy.md` §6) before Slice B2. Suggested
  number 0017. **Design as a pair with M04A's D-04-01**
  ([ADR-0015](../../../decisions/0015-ucan-capability-model.md), written
  2026-07-13) — its `SessionContext`/`Capability` types are the inputs this
  compiler binds as SQL `?` parameters.

---

## FDAE Enforcement Model (design seed for D-04-02)

*Reference material for the ADR — not the ADR itself. Grounded in the request
lifecycle on `main` @ `6c6e859`.*

Access control is a **synthesis of cryptographic capabilities and relational
data state** (`system-requirements-spec.md:976`): **UCAN capabilities** gate
Tiers 0–2 (built in M04A); **FDAE ReBAC** is Tier 3 (this milestone).

**Stage 0 — Context init (produces the truth, not itself a gate).** M04A B1
builds the SessionContext `{caller_did, capabilities, scopes, claims, roles,
env}`. Host-injected and unspoofable (`system-architecture.md:1830`). Everything
below reads it.

**Tier 1 — Service admission (M04A B0).** *"May this caller invoke this interface
on this `service_id` at all?"* — `handle_stream` / `verify_preamble`.

**Tier 2 — Method/argument admission (M04A B0).** *"May this caller invoke THIS
method with THESE args (which collection / topic / blob namespace)?"* — after
deserialization in `dispatch`; the Admin-UCAN gate for `execute-ddl`/`query-raw`
lives here.

**Tier 3 — Data-plane filtering (THIS MILESTONE).** `crates/data_db/src/filter.rs`
+ query building in `crates/data_db/src/sqlite.rs`. The pushdown sieve compiles
ReBAC policy into SQL and merges with the caller's JSON filter:
- **Mode A — Point-In-Time Evaluation** (`system-architecture.md:1843`): "Can
  Alice view document 12?" → append `WHERE documents.id = ?`; return Allow/Deny.
- **Mode B — Relational Data Filtering** (`:1844`): "Show all documents I can
  see" → wrap the base query in the compiled `WHERE EXISTS` security block;
  SQLite prunes at the index level before rows reach the guest.
- **Sub-point 3a — Cross-service parameter fetch** (pipeline stage 2,
  `:1841`): pause evaluation, fetch a remote relationship proof via the Universal
  Proxy (M04A A1), inject, resume. **Slice B3.**
- **Sub-point 3b — After-step WASM ABAC** (pipeline stage 4,
  `system-requirements-spec.md:983`): optional non-relational attribute check on
  candidate rows.

**The 4-stage hybrid pipeline** (`system-requirements-spec.md:979-983`): (1)
context/UCAN verify [M04A] → (2) cross-service fetch [B3] → (3) SQL sieve [B2] →
(4) optional WASM ABAC [stage-4].

**Principles to bake into D-04-02:** enforce at the **data-owning node**
(re-verified, never trusted from the caller's node); **default-deny / fail-closed**
everywhere (watchdog timeout → Denied); **push filtering into SQL** (never
fetch-then-filter in the guest).

### Stage-4 ABAC — signature & guardrail

The after-step is a **pure predicate over provided inputs**, *not* a handle that
can reach back into data sources. It receives candidate-row fields + caller
context; it must **not** be able to issue new queries (that would re-open the
fetch-then-filter hole, break WASM isolation, and destroy the per-row performance
model — remote data is stage 2's job, before SQL execution). Illustrative shape
for D-04-02 to pin:

```wit
// guest-exported; host calls it with the SQL-sieve's candidate rows
authorize-rows: func(ctx: auth-context, rows: list<candidate-row>)
             -> list<row-decision>;
// auth-context = { caller-did, capabilities, scopes, claims, roles, env }
// row-decision = allow | deny | redact(fields)
```

Decisions D-04-02 must settle: **batch invocation** (pass a batch, not one call
per row — hot path); **opt-in per policy rule** (not global); and — since the
arch calls stage 4 an *"Override Filter"* (`system-requirements-spec.md:983`) —
whether it may only **further-restrict/redact** (safe default) or also **widen**
access beyond ReBAC (dangerous; must be explicitly capability-gated if allowed).
Default: restrict-only.

---

## Explicit Non-Goals

- Everything in M04A (identity threading, UCAN context, Universal Proxy,
  conversion, `query-raw`, aggregation, per-app KEK) — prerequisites, not scope.
- **Full DataFusion/Substrait federated query orchestration** (M5). B3 fetches a
  relationship *proof* mid-policy-evaluation; it is **not** a general federated
  query planner.
- Full MongoDB aggregation-operator compatibility beyond M04A's `AggregationPipeline`.
- `[PRD-SAF]` consent/dispute/moderation UX (retargeted — M04A Decision Register
  A.1) — FDAE is the *mechanism*, not the policy-authoring surface.
- Replication of policy or relationship state across nodes (M7). Note: DB
  replication uses **node-level** authz, *not* row-level UCANs inside the WAL
  stream (`system-requirements-spec.md:973`).

---

## Dependency Gates

M04B may begin **implementation** only when:

1. **M04A is closed** — B0 (identity field), B1 (SessionContext), and A1
   (Universal Proxy) shipped and green.
2. **ADR D-04-02 resolved** and written to `docs/decisions/` before Slice B2.
   D-04-02 *design* may proceed in parallel with M04A implementation; only B2/B3
   *code* is gated.
3. `cargo test --workspace` clean, zero clippy warnings on the branch M04B starts
   from.

**Slice order:** B2 (local sieve) → B3 (federated fetch, needs A1) → stage-4 ABAC
(may fold into B2's design).

---

## Current State Inventory (anchors re-verified on `main` @ `6c6e859`)

| Crate / File | What Exists / Gap |
|---|---|
| `crates/data_db/src/filter.rs` | ADR-0007 MongoDB-style JSON filter → parameterized SQLite compiler. **The FDAE security subquery must merge with this**, not replace it |
| `crates/data_db/src/sqlite.rs` | CRUD/query building, single-writer + reader-pool model. Query path is where the compiled `WHERE EXISTS` block is injected |
| `crates/rpc/src/native.rs` (post-M04A) | `NativeInvocation` will carry the caller identity Tier 3 filters against (M04A B0) |
| `crates/router` Universal Proxy (post-M04A A1) | Transport B3's cross-service proof-fetch rides |
| [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md) | "No result is a valid outcome" principle — unauthorized rows are *excluded*, not errored |
| — | **Gaps:** no FDAE policy model, no ReBAC→SQL compiler, no RLS/CLS, no cross-service fetch, no stage-4 ABAC |

---

## Migration Strategy

### `ServiceManifest` Extension
```toml
[services.my-svc.fdae]
policy_path = "fdae-policy.json"   # optional declarative ReBAC policy (D-04-02 schema)
```
`#[serde(default)]`; existing manifests parse cleanly.

### No Data Migration
Policy enforcement is additive at the query-compilation layer, not a stored-data
change. No existing `SynSvc` DB needs schema migration.

### WIT Boundary Versioning
`syneroym:data-layer` may gain an additive stage-4-ABAC guest export (per
D-04-02) — minor bump, non-breaking. `wasm32-wasip2` must stay unbroken.

---

## Ordered Implementation Slices

#### Slice B2: Local FDAE (SQL Pushdown Sieve)
**Blocked on:** ADR D-04-02. **Depends on:** M04A (B1 SessionContext, B0 identity).
**Requirement:** `[FND-IAM]`.
Compile declarative ReBAC policy into SQLite `WHERE EXISTS` / cycle-protected
`WITH RECURSIVE`; implement **Mode A** (Point-In-Time) and **Mode B** (Relational
Data Filtering) per `system-architecture.md:1842-1844`; RLS (row prune) + CLS
(column projection/masking); the `visited_track` cycle guard and
`sqlite3_progress_handler` watchdog with **default-deny** on timeout
(`:1847-1848`); strict parameterized binding (no injection). Merge the compiled
security subquery with the caller's ADR-0007 JSON filter at SQL generation.
Covers pipeline stages 1, 3, 4 (stage 2 is B3).

#### Slice B3: Federated FDAE (Cross-Service Parameter Fetch)
**Depends on:** B2, and M04A A1 (Universal Proxy). **Requirement:** `[FND-IAM]`.
Pipeline stage 2 (`system-requirements-spec.md:981`, `system-architecture.md:1841`):
pause evaluation, fetch remote relationship proofs/parameters via the Universal
Proxy, inject into local evaluation context, resume. Enforcement happens at the
**data-owning node**; a fetch timeout falls back to **deny**, not silent allow.

#### Stage-4 WASM ABAC
Wire the guest-exported `authorize-rows` after-step (shape per D-04-02, see FDAE
Enforcement Model). Pure predicate over provided inputs; batched; opt-in per rule;
restrict-only by default.

---

## Reference Scenario (M04B subset)

Continues from M04A (steps 20–21, 24–25):

22. A `data-layer::query` call is transparently filtered by FDAE's SQL pushdown
    sieve — unauthorized rows never reach the WASM guest (B2, Mode B).
23. A ReBAC check requiring a remote relationship proof triggers a cross-service
    fetch via the Universal Proxy mid-query (B3, pipeline stage 2).

---

## Failure and Security Tests

| Test | Expected Outcome |
|---|---|
| FDAE query for a resource the caller's ReBAC chain doesn't reach (Mode B) | Row excluded from results, not an error (ADR-0007 "no result is a valid outcome") |
| FDAE Point-In-Time check (Mode A) for an unreachable resource | Deny flag; no data leak |
| CLS: caller lacks column permission | Column masked/projected out; value never returned |
| FDAE policy with a cyclic ReBAC relationship in user data | `visited_track` breaks recursion; no infinite loop (`system-architecture.md:1847`) |
| Compiled FDAE query exceeds the policy time budget | Transaction rolled back, Default-Denied (`:1848`) |
| Cross-service FDAE parameter fetch times out | Falls back to deny, not silent allow |
| Stage-4 ABAC attempts to reach back into a data source | Not possible by construction — it receives rows + context only, no query handle |
| Stage-4 ABAC returns `redact(fields)` | Named fields removed from the row before it reaches the guest |

---

## Performance Budgets

| Metric | Budget | Method |
|---|---|---|
| FDAE pushdown query (100 records, single-hop ReBAC) | < 25 ms p99 (vs. M3A's unauthenticated 20 ms — +5 ms for policy compilation) | `criterion` integration bench |
| Federated FDAE fetch (one cross-service hop) | < 50 ms p99 (network-bound; a floor, not a hard SLA) | Integration test, two local nodes |
| Stage-4 ABAC over a candidate batch | Document measured; must not dominate Mode-B query latency | `criterion` micro-bench |

---

## Tests Summary

- **Unit:** ReBAC → `WHERE EXISTS`/`WITH RECURSIVE` translation, cycle
  protection, RLS + CLS SQL generation, security-subquery ⊕ JSON-filter merge
  (B2).
- **Integration:** Mode A / Mode B end-to-end (unauthorized rows excluded);
  federated FDAE cross-node fetch + timeout→deny (B3); stage-4 redact/deny.
- **Benchmarks (`criterion`):** FDAE pushdown query, stage-4 batch.
- **E2E (`mise run test:e2e`):** reference scenario steps 22–23 in a live
  substrate, ≥2 substrates for the federated case.

---

## Measurable Exit Criteria

- [ ] `cargo +nightly fmt --all` clean; `cargo clippy --workspace --all-targets --all-features` zero warnings; `cargo test --workspace` green; `mise run test:e2e` green (no M0–M04A regression); `wasm32-wasip2` unbroken after every slice.
- [ ] ADR D-04-02 exists in `docs/decisions/`.
- [ ] FDAE pushdown sieve implemented: Mode A + Mode B, RLS + CLS, cycle guard, watchdog default-deny, parameterized binding.
- [ ] Compiled FDAE security subquery merges correctly with the ADR-0007 JSON filter.
- [ ] Federated cross-service fetch (B3) works over the Universal Proxy; timeout→deny verified.
- [ ] Stage-4 ABAC wired: pure-predicate, batched, restrict-only default; redact/deny tested.
- [ ] Reference scenario steps 22–23 execute end-to-end.
- [ ] All Failure and Security Tests produce documented outcomes.
- [ ] Performance budgets verified; `criterion` output in `status.md`.
- [ ] `traceability-matrix.md` updated with M04B evidence for `[FND-IAM]` (data-aware authorization: pushdown sieve, RLS/CLS, 4-stage pipeline); `[PRD-SAF]` retargeted to `TBD` (M04A Decision Register A.1) if not already done at M04A closeout.
