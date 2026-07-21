# Milestone 4B: FDAE Data-Aware Authorization (M04B-fdae-policy)

> **Provenance.** Split on 2026-07-13 from a combined M4 planning draft along
> the **capability-plumbing vs. data-aware-policy-engine** boundary. The
> authentication + capability foundation this milestone builds on —
> verified caller-identity threading, UCAN context, the Universal Proxy — is the
> sibling milestone
> [M04A-proxy-and-auth-foundation](../M04A-proxy-and-auth-foundation/task.md),
> which is now **closed** — every slice (A0′, B0, A1, B1, B4, B5, B6, B7a, B7b)
> shipped and green (matrix rows `[PLT-DAT]`/`[FND-IAM]`/`[FND-SEC]` = Complete).
> Code anchors re-verified against `main` @ `64f3571` (2026-07-20). The one
> M04A follow-on **not** delivered — the `ControllerAgreement` creation tool —
> was deferred to M5 (matrix row, 2026-07-18) and is **out of M04B scope**: FDAE
> depends on B0/B1/A1, not on that tool. See Dependency Gates below.
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
| `[FND-IAM]` (policy engine) | Declarative Zanzibar-style policy schema; ReBAC→SQL compilation (`WHERE EXISTS`/`WITH RECURSIVE`); RLS + CLS; the 4-stage hybrid pipeline; stage-4 WASM ABAC (`system-requirements-spec.md:971-985`, `system-architecture.md:1826-1848`) | Row **exists** — `[FND-IAM]` (M4B: FDAE), status **Planned** (`traceability-matrix.md`). Exit criteria flip it to Complete with evidence. |

M04A carries the `[FND-IAM]` *foundation* row (identity threading, UCAN context,
Admin capability). M04B carries the `[FND-IAM]` *data-aware-authorization* row.

---

## Dependency on M04A & Co-design Seams

**M04B implementation follows M04A**, which is closed — the three hard
dependencies below all **shipped and are green** (verify against the anchors,
don't re-plan them):

- **B0's caller identity** — `NativeInvocation.caller: CallerContext` in
  [`crates/rpc/src/native.rs`](../../../../crates/rpc/src/native.rs) carries the
  verified `caller_did`. Tier 3 filters *against* it (before B0, the query path
  keyed on `creator_id = self.service_id`, the DB owner, not the caller).
- **B1's `SessionContext`** — `{subject_did, capabilities, claims,
  verified_at_secs}` in
  [`crates/ucan/src/session.rs`](../../../../crates/ucan/src/session.rs),
  reached via `CallerContext.session`. The pushdown compiler binds from it as
  SQL `?` parameters. The spec's "normalized scopes and claims"
  (`system-requirements-spec.md:976,978`) maps onto these two fields —
  **"scopes" is loose prose for `capabilities`**, not a missing dimension (no
  `scopes`/`roles`/`env` field exists, and `Capability {with, can, caveats}` is
  the richer form). `claims` is a `serde_json::Map`, trusted only when the leaf's
  issuer is a trusted root. What the compiler binds vs. what it uses to *select a
  policy branch* is an **unresolved decision** (see Decision Register D-04-02-a).
- **A1's Universal Proxy** — Slice B3's stage-2 cross-service relationship-proof
  fetch rides it; the internal fetch path is already reserved at
  [`crates/rpc/src/proxy.rs:51`](../../../../crates/rpc/src/proxy.rs).

**M04B *design* (ADR D-04-02) runs in parallel with M04A implementation.** Two
contracts are built in M04A but consumed here; design them jointly (D-04-01 ↔
D-04-02 as a pair):

1. **SessionContext / capability representation** — shape M04A's B1 output so the
   pushdown compiler can bind scopes/claims as `?` parameters.
2. **Universal Proxy request shape** — ensure M04A's A1 can carry a
   relationship-proof fetch (Slice B3), not only an ordinary call.

---

## Decision Register

### B. FDAE policy schema & compilation — ADR **Accepted**, B2 unblocked

- **D-04-02 — FDAE Policy Schema & Compilation Strategy** →
  [ADR-0017](../../../decisions/0017-fdae-policy-schema-and-compilation.md)
  (**Accepted 2026-07-20**; drafted 2026-07-16 from
  [`access-control-design.md`](../../access-control-design.md); co-designed with
  [ADR-0015](../../../decisions/0015-ucan-capability-model.md)'s 2026-07-16
  amendment, which is the grant-layer half). Accepted with the parameter-binding
  (D-04-02-a), default-permission (-b), and `strict:` (-c) items resolved at
  acceptance — see the ADR's "Resolved at acceptance" block. B2 is unblocked.
  Notable departures from the checklist below, each **deleting** a concept: the
  physical registry / `data_sources` is dropped entirely (a relation is local or
  names a service — policies never carry connection strings); `hierarchies`
  folds into `relations` as `recursive: true`; and app abilities fold into
  `permissions`, which now declare both the operations they cover and the rows
  they reach. Stage-4 ABAC **may** issue read-only lookups (restrict-only and
  fuel-metered) — the prohibition below rested on two arguments that do not
  hold; only the N+1 performance one does. See ADR-0017 §1, §2, §7.

  The core design work of this milestone. Must settle:
  - The declarative Zanzibar-style policy config format (YAML/JSON) and the
    **typed policy model** it deserializes into (no runtime string lexers —
    `system-architecture.md:1832-1836`), versioned + JSON-Schema-validated at
    deploy (as `app-config` already is).
  - **Naming.** A portable *logical* vocabulary (object types, relations,
    permissions, field names). ADR-0017 **drops the standalone physical
    registry / `data_sources`** the arch doc sketched (`:1834`): a relation is
    either local or names a service DID; policies never carry connection
    strings. Logical→service-DID resolution reuses the existing app-context
    registry (`system-architecture.md:1881-1883`); logical→table/column binding
    lives in the policy's own `definitions:` block (ADR-0017 §1, §5). *(This
    supersedes the earlier "resolved against a physical registry binding"
    framing, which the ADR deleted.)*
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

  ~~Resolve as an ADR (per `session-strategy.md` §6) before Slice B2.~~ Drafted
  as ADR-0017 (above). **Designed as a pair with M04A's D-04-01**
  ([ADR-0015](../../../decisions/0015-ucan-capability-model.md), written
  2026-07-13, amended 2026-07-16) — its `SessionContext`/`Capability` types are
  the inputs this compiler binds as SQL `?` parameters.

### Sub-decisions — a/b/c resolved at ADR acceptance; d/e remain

Carried from ADR-0017 §9, plus one (`-a`) surfaced by re-verifying against
`main`. The first three were **resolved at ADR-0017's acceptance (2026-07-20)**
and no longer gate B2; d/e remain as a deferral and a B7 hand-off.

- **D-04-02-a — What the compiler binds as `?`.** ✅ **Resolved.** *"Scopes" is
  not a missing dimension* — the spec's "scopes" (`system-requirements-spec.md:976,978`)
  is loose prose for the shipped `capabilities` field, and `Capability {with,
  can, caveats}` is the richer form (no `scopes`/`roles`/`env` field, none
  added). `claims` (scalars) and a capability's scalar **`caveats`** bind as `?`;
  the capability's `with`/`can` instead **select which permission/`WHERE EXISTS`
  branch compiles**. No `SessionContext` change. See ADR-0017 "Resolved at
  acceptance".
- **D-04-02-b — Default permission when a grant names a platform ability and a
  policy exists.** ✅ **Resolved: default-deny** unless the policy declares a
  default (ADR-0017 §2, "Resolved at acceptance").
- **D-04-02-c — `strict: true` mode.** ✅ **Resolved: off by default, additive**,
  with an author-time warning; implementation **sequenced inside B2, not ahead of
  it** (it only ever tightens, so it can't block the slice start). Whether
  `strict` eventually flips default-on is left to the third-party-authoring point.
- **D-04-02-d — Stale relationship data / Zanzibar "new enemy"** (ADR-0017 §9).
  **Deliberately deferred to M7** (replication); recorded here so it is a
  decision, not an M7 surprise. §6's TTL'd proofs bound the cross-service window.
  **Not a M04B gate — documented deferral.**
- **D-04-02-e — Tier-1 native-service admission ownership** (ADR-0017 §9;
  `route_handler/dispatch.rs`'s `TODO(M04B/FDAE)`). ADR-0017's position: Tier 1
  is a µs-scale *grant-layer* capability check, **not** a policy-engine question —
  so that TODO does **not** belong to M04B, and "today any verified identity
  reaches any native service" is a wider live gap than the milestone docs imply.
  **Reconcile in B7 (grant layer), explicitly out of M04B scope.**
- **D-04-02-f — Creation authorization for the write path.** ⛳ **Open — gates
  Slice B5-fdae, not B2.** FDAE's read side (RLS/CLS, B2/B3) protects
  *confidentiality*; the *integrity* side — Mode-A authorization of single-row
  mutations (`put`/`patch`/`delete`/`batch_mutate`) — is deferred to Slice
  B5-fdae. `patch`/`delete` of an existing row map cleanly to Mode A ("may caller
  write row `id`?"), but **`put`-create has no row to evaluate `[creator,
  caller]` against**: row-reachability ReBAC cannot express *"who may create a row
  in this collection,"* which is a **collection-scoped** permission the current
  policy model lacks. Must settle: whether creation is governed by a new
  collection-level permission kind (an ADR-0017 §1 schema amendment), and how
  `batch_mutate` authorizes per-mutation. **Until B5-fdae lands, single-row
  write/delete integrity is unenforced** (a caller who cannot *see* a row via RLS
  can still `delete(id)`/`patch(id)` it — pre-existing, since host write paths run
  under service authority and carry no capability gate today). Surfaced during
  Slice B2 Phase-2 review.
- **D-04-02-g — Multi-capability caveat semantics (additive vs.
  intersective).** ⛳ **Open — not a B2 blocker (over-restrictive, not a
  leak).** `compile_read` collects `entitling_caps` as *every* capability
  whose `grants()` covers the operation (caveats play no part in `grants()`),
  then flattens each one's `caveats.where` into `CompiledSieve.where_caveats`
  — a single list ANDed together by `data_db`'s `merge_sieve`, with no
  per-OR-branch association back to the capability that earned it. Concrete
  failure: a caller holding both an unrestricted `read` capability and a
  second, narrower-caveated one (e.g. `region: EU`) on the same resource gets
  **the intersection** of both caveats, not the union each capability should
  independently grant — the unrestricted capability's access is narrowed by
  the mere presence of the second one. Capabilities are meant to be additive;
  this is accidentally intersective. Correct semantics need each path/OR-branch
  to carry *its own* entitling capability's caveat — `(P1 AND caveat₁) OR (P2
  AND caveat₂)` — which the current flat `where_caveats: Vec<Json>` shape
  cannot express; fixing it is a `crates/fdae` (Phase 1) `CompiledSieve`
  contract change, not a Phase 2 `data_db` one. The same root cause makes CLS
  `fields.deny` lists union across capabilities too (`compile_cls`) — the RLS
  variant was pinned in Phase 2; the CLS variant is now **live** (Phase 3
  ships the host-side field-strip that actually applies `masked_fields` to a
  returned payload, so a caller holding an unrestricted capability alongside
  a second, `fields.deny`-caveated one on the same resource now observably
  gets their unrestricted grant's payload stripped too — previously this was
  latent, since Phase 2 exposed `masked_fields` but never applied it).
  **Pinned, not silently dropped:**
  `tests_fdae.rs::two_capabilities_with_conflicting_caveats_currently_narrow_to_zero_rows`
  (RLS, Phase 2) and
  `host_capabilities.rs::tests::fdae_d04_02_g_extra_caveated_capability_narrows_cls_strip`
  (CLS, Phase 3) both assert today's (undesired) behavior explicitly, with a
  comment directing whoever fixes this to flip the assertion. Surfaced during
  Slice B2 Phase-2 review (independent re-review pass).

---

## FDAE Enforcement Model (design seed for D-04-02)

*Reference material for the ADR — not the ADR itself. Grounded in the request
lifecycle on `main` @ `64f3571`.*

Access control is a **synthesis of cryptographic capabilities and relational
data state** (`system-requirements-spec.md:976`): **UCAN capabilities** gate
Tiers 0–2 (built in M04A); **FDAE ReBAC** is Tier 3 (this milestone).

**Stage 0 — Context init (produces the truth, not itself a gate).** M04A B1
builds the verified `SessionContext` `{subject_did, capabilities, claims,
verified_at_secs}` (`crates/ucan/src/session.rs`), wrapped by `CallerContext`
`{caller_did, creator_id, session, proof}` (`crates/rpc/src/native.rs`).
Host-injected and unspoofable (`system-architecture.md:1830`); never
deserialized-and-trusted from the wire — `claims` are honored only when the
leaf's issuer is a trusted root. Everything below reads it. *(The arch/spec
prose also names "scopes"/"roles"/"env"; those are **not** implemented fields —
see Decision Register D-04-02-a.)*

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

The after-step runs on the SQL-sieve's candidate rows + caller context. Per
**ADR-0017 §7**, it **may** issue read-only lookups (restrict-only and
fuel-metered) — the earlier blanket "no query handle, pure predicate over
provided inputs" prohibition was **relaxed** by the ADR: of its three original
arguments (fetch-then-filter hole, WASM-isolation break, N+1 performance), only
the N+1 performance one survives, and fuel-metering + restrict-only contain it.
So stage 4 is **not** a general query planner (that is stage 2's job, before SQL
execution), but neither is it barred from lookups. Illustrative shape for
D-04-02 to pin:

```wit
// guest-exported; host calls it with the SQL-sieve's candidate rows
authorize-rows: func(ctx: auth-context, rows: list<candidate-row>)
             -> list<row-decision>;
// auth-context = { subject-did, capabilities, claims }   // mirrors SessionContext
// row-decision = allow | deny | redact(fields)
```

Decisions D-04-02 must settle: **batch invocation** (pass a batch, not one call
per row — hot path); **opt-in per policy rule** (not global); the **fuel/time
budget** for any §7 lookup; and — since the arch calls stage 4 an *"Override
Filter"* (`system-requirements-spec.md:983`) — that it may only
**further-restrict/redact**, never **widen** access beyond ReBAC. Default and
enforced position: **restrict-only** (a widen path would need an explicit,
separately capability-gated design; not in M04B scope).

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

1. ~~**M04A is closed**~~ — **SATISFIED (2026-07-20).** B0 (caller identity),
   B1 (`SessionContext`), and A1 (Universal Proxy) all shipped and green; matrix
   rows Complete. The deferred `ControllerAgreement` creation tool (M5) does
   **not** gate M04B — FDAE binds B0/B1/A1, not that tool.
2. ~~**ADR D-04-02 resolved**~~ — **SATISFIED (2026-07-20).** ADR-0017 is
   **Accepted**, with sub-decisions **D-04-02-a/-b/-c** resolved at acceptance
   (parameter binding, default-deny, `strict:` off-by-default-and-sequenced).
   D-04-02-d (M7 deferral) and -e (B7 grant-layer) were never B2 blockers.
3. `cargo test --workspace` clean, zero clippy warnings on the branch M04B starts
   from (main is currently green @ `64f3571`).

**Slice order:** B2 (local sieve, read side) → B3 (federated fetch, needs A1) →
B4-fdae (stage-4 ABAC, depends on B2; may fold into it) → B5-fdae (write-side
Mode-A enforcement, depends on B2's `check_access` + D-04-02-f).

---

## Current State Inventory (anchors re-verified on `main` @ `64f3571`)

| Crate / File | What Exists / Gap |
|---|---|
| `crates/data_db/src/filter.rs` | ADR-0007 MongoDB-style JSON filter → parameterized SQLite compiler: `compile_filter(Option<&str>) -> Option<CompiledFilter>`. **The FDAE security subquery must merge with `CompiledFilter`** (one `AND` at SQL generation), not replace it |
| `crates/data_db/src/sqlite.rs` | CRUD/query building, single-writer + reader-pool model. Query path is where the compiled `WHERE EXISTS` block is injected |
| `crates/rpc/src/native.rs` | **Shipped (B0):** `NativeInvocation.caller: CallerContext` carries the verified `caller_did` + `SessionContext` Tier 3 filters against |
| `crates/rpc/src/proxy.rs:51` | **Shipped (A1):** Substrate-internal proxy path already reserved for "the FDAE policy engine's relationship-proof fetch" — B3's stage-2 transport |
| In-code seams | `TODO(M04B/FDAE)` markers mark the wire-in points: `router/src/route_handler/dispatch.rs` (Tier-1 admission — see D-04-02-e), `router/src/proxy.rs` (interim coarse gate → FDAE), `control_plane/src/service.rs` (security-op authz) |
| [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md) | "No result is a valid outcome" principle — unauthorized rows are *excluded*, not errored |
| — | **Gaps:** no FDAE policy model, no ReBAC→SQL compiler, no RLS/CLS, no cross-service fetch, no stage-4 ABAC — no `fdae` crate or `policy`/`rebac` module exists in `crates/data_db/src/` |

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

#### Slice B2: Local FDAE (SQL Pushdown Sieve) — Phase 1 ✅ (2026-07-20, PR #86); Phase 2 ✅ (2026-07-20); Phase 3 ✅ (2026-07-21)
**Unblocked** (ADR D-04-02 Accepted; a/b/c resolved). **Depends on:** M04A (B1
SessionContext, B0 identity).
**Requirement:** `[FND-IAM]`.
Compile declarative ReBAC policy into SQLite `WHERE EXISTS` / cycle-protected
`WITH RECURSIVE`; implement **Mode A** (Point-In-Time) and **Mode B** (Relational
Data Filtering) per `system-architecture.md:1842-1844`; RLS (row prune) + CLS
(column projection/masking); the `visited_track` cycle guard and
`sqlite3_progress_handler` watchdog with **default-deny** on timeout
(`:1847-1848`); strict parameterized binding (no injection). Merge the compiled
security subquery with the caller's ADR-0007 `CompiledFilter` at SQL generation
(one `AND`, two compilers). Covers pipeline stages 1 and 3 (stage 2 is B3;
stage 4 is Slice B4-fdae). Applies the ADR-0017 resolutions: bind `claims` +
capability `caveats` while `with`/`can` select the branch (D-04-02-a);
**default-deny** when no permission is named (-b); `strict:` off by default and
implemented within this slice (-c).

**Phase 1** (`crates/fdae`: policy model, JSON Schema, ReBAC→SQL compiler) —
merged `main` @ PR #86. **Phase 2** (`crates/data_db` integration: `query`/
`get`/`aggregate`/`delete_many` threaded with an `Option<QueryAuth>`, sieve
spliced into SQL generation, new `check_access` Mode-A primitive, watchdog
matrix wired) — done on `feat/m04b-slice-b2-data-db`. **Phase 3** (WIT
`check-access` + `HostState.fdae_policy` + real `QueryAuth` construction on
the WASM read path + host-side CLS field-stripping, proven by
`sandbox_wasm` host tests that inject a `Policy` by hand) — done on the same
branch; `HostState.fdae_policy` stays `None` in production until Phase 4
(deploy/persist/manifest plumbing) loads a real one, so FDAE still enforces
nothing for a live deployed caller. The Failure/Security matrix's CLS "value
never returned" row is now satisfied. Full evidence: `status.md`.

#### Slice B3: Federated FDAE (Cross-Service Parameter Fetch)
**Depends on:** B2, and M04A A1 (Universal Proxy). **Requirement:** `[FND-IAM]`.
Pipeline stage 2 (`system-requirements-spec.md:981`, `system-architecture.md:1841`):
pause evaluation, fetch remote relationship proofs/parameters via the Universal
Proxy, inject into local evaluation context, resume. Enforcement happens at the
**data-owning node**; a fetch timeout falls back to **deny**, not silent allow.

Also lands **ADR-0015 A5's `anchor_did`** (accepted in the ADR, implemented
nowhere — B7 shipped with the `DelegationCertificate`'s `master_did` and
deferred `anchor_did` to real UCAN chains, B7.md:1119-1124). B3 is A5's first
real consumer: cross-service chains are the first place `caller ≠ anchor` is
real and e2e-testable, and a row policy on the data-owning node must filter by
the **original principal (`anchor`)**, not the proxying service (`caller`) — the
confused-deputy defense. Adds `SessionContext.anchor_did: Option<String>`
(populated in `from_verified_chain` as the audience of the first non-root token)
and the compiler's `anchor` path-terminal (B2 ships `caller` only and errors on
`anchor`). A5's full `path` *list* binding stays deferred (no near-term
consumer). *(This supersedes access-control-design.md:996's "B7 is the first
real consumer" line for A5 specifically.)*

#### Slice B4-fdae: Stage-4 WASM ABAC
**Depends on:** B2 (candidate rows come from the sieve). May fold into B2's
design if it stays small. **Requirement:** `[FND-IAM]`.
Wire the guest-exported `authorize-rows` after-step (shape per D-04-02, see FDAE
Enforcement Model). Batched (one call per candidate batch, not per row); opt-in
per policy rule; **restrict-only** (may redact/deny, never widen). Per ADR-0017
§7 it **may** issue read-only lookups, but only fuel-/time-metered — enforce the
budget and Default-Deny on overrun.

#### Slice B5-fdae: Write-Side Tier 3 (Mode-A Write Authorization)
**Depends on:** B2 (the `check_access` Mode-A primitive) **and D-04-02-f**
(creation authorization). **Requirement:** `[FND-IAM]`.
B2/B3 deliver read-side Tier 3 (confidentiality: RLS/CLS on `query`/`get`/
`aggregate`). This slice closes the **integrity** half: authorize single-row
mutations against the caller's ReBAC policy so a row a caller cannot reach is
also one they cannot write or delete. Today `put`/`patch`/`delete`/`batch_mutate`
run under service authority (`creator_id = component_id`), never consult
`caller.session`, and carry no capability gate — so single-row write/delete
bypasses Tier 3, an asymmetry with B2's already-filtered `delete_many`
(surfaced in Slice B2 Phase-2 review).
- **`patch`/`delete`/`batch_mutate`-delete** of an *existing* row → Mode-A
  `check_access` (op = `data-layer/write`) at the host before executing;
  unreachable → `permission-denied`, not a silent write.
- **`put`-create** → blocked on **D-04-02-f**: row-reachability cannot express
  "who may create," so this needs the collection-scoped create-permission
  decision first (an ADR-0017 §1 schema amendment) before it can be enforced.
- Thread `caller.session` into the host write methods (they don't today); add the
  write-path rows to the Failure/Security matrix (unreachable write → deny; create
  without create-permission → deny).

**Known limitation until this slice lands:** FDAE protects read confidentiality
and bulk-delete, but single-row write/delete integrity is unenforced —
deployments relying on FDAE for write integrity must wait for B5-fdae.

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
| CLS: caller lacks column permission | Column masked/projected out; value never returned — ✅ satisfied by Slice B2 Phase 3's host-side `strip_masked_fields` |
| FDAE policy with a cyclic ReBAC relationship in user data | `visited_track` breaks recursion; no infinite loop (`system-architecture.md:1847`) |
| Compiled FDAE query exceeds the policy time budget | Transaction rolled back, Default-Denied (`:1848`) |
| Cross-service FDAE parameter fetch times out | Falls back to deny, not silent allow |
| Stage-4 ABAC attempts to **widen** access beyond ReBAC | Rejected — restrict-only enforced; a widen decision cannot grant a row the sieve excluded (ADR-0017 §7) |
| Stage-4 ABAC read-only lookup (§7) exceeds its fuel/time budget | Aborted, row Default-Denied; the lookup cannot run unmetered |
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
- [x] ADR D-04-02 ([ADR-0017](../../../decisions/0017-fdae-policy-schema-and-compilation.md)) **Accepted** (2026-07-20), with D-04-02-a/-b/-c resolved.
- [ ] FDAE pushdown sieve implemented: Mode A + Mode B, RLS + CLS, cycle guard, watchdog default-deny, parameterized binding.
- [ ] Compiled FDAE security subquery merges correctly with the ADR-0007 JSON filter.
- [ ] Federated cross-service fetch (B3) works over the Universal Proxy; timeout→deny verified.
- [ ] Stage-4 ABAC wired: pure-predicate, batched, restrict-only default; redact/deny tested.
- [ ] Reference scenario steps 22–23 execute end-to-end.
- [ ] All Failure and Security Tests produce documented outcomes.
- [ ] Performance budgets verified; `criterion` output in `status.md`.
- [ ] `traceability-matrix.md` `[FND-IAM]` (M4B: FDAE) row flipped **Planned → Complete** with evidence (pushdown sieve, RLS/CLS, 4-stage pipeline, federated fetch, stage-4 ABAC). *(Row already present; `[PRD-SAF]` already retargeted to `TBD` at M04A closeout — no action unless it regresses.)*
- [ ] Sub-decisions D-04-02-a/-b/-c (resolved at ADR acceptance) reflected in the shipped schema/compiler; D-04-02-d/-e recorded as deferral/B7 hand-off, not silently dropped.
