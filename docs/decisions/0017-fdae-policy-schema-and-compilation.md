# D-04-02: FDAE Policy Schema & Compilation Strategy

**Status**: Accepted (2026-07-20; drafted 2026-07-16 as the M04B blocking ADR).
Accepted with three open items resolved at acceptance — see the "Resolved at
acceptance" block under Decision §9.

**Context**:

Milestone 4B implements the Federated Data-Aware Authorization Engine: declarative
ReBAC policy compiled into SQLite so unauthorized rows and columns never reach the
WASM guest (`system-requirements-spec.md:971-985`,
`system-architecture.md:1826-1848`). M04A built the identity and capability
foundation — verified caller identity threaded through native dispatch (Slice B0),
a normalized `SessionContext` from a verified UCAN chain (B1), and the Universal
Proxy (A1). This ADR decides the policy artifact and its compilation.

It is the **Layer 2 half** of
[`docs/planning/access-control-design.md`](../planning/access-control-design.md);
[ADR-0015](0015-ucan-capability-model.md) (as amended 2026-07-16) is the Layer 1
half. The two were designed as a pair and should be read as one: **a capability
says what you were handed, a policy says what the data allows, and the effective
answer is the intersection.**

Prior material: `docs/archive/authorization-engine-spec.md` is the fuller
exploratory source the architecture's FDAE section was condensed from. It is
non-authoritative (AGENTS.md) and this ADR departs from it in three places, each
of which **deletes** a concept — see Decision §1.

**Decision**:

## 1. The policy document: `version` + one section

One document per service, referenced from the manifest (`policy_path`,
`#[serde(default)]` so existing manifests parse), versioned and
JSON-Schema-validated at deploy. Author-time validation against a typed schema
catches most policy bugs before they can deny anything in production — the Cedar
lesson.

```yaml
version: "fdae/v1"

definitions:
  document:
    table: documents
    relations:
      creator:          { target: user, join_column: creator_uuid }
      parent_dept:      { target: department, join_column: owner_dept_id }
      management_chain: { target: user, from_key: id, to_key: manager_id,
                          recursive: true }
    permissions:
      view:
        allows: [data-layer/read]
        operator: union
        paths:
          - [creator, caller]
          - [creator, management_chain, caller]
      manage:
        allows:   [data-layer/read, data-layer/write, rpc/move]
        includes: [view]
        paths:    [[creator, caller]]
```

Three deletions from the archived spec:

- **`data_sources` is deleted entirely.** The archived spec has policies carrying
  raw connection strings (`connection: "file:app_state.db?mode=ro"`), which
  contradicts the platform model — one host-managed DB per service, guests never
  touch a database (`system-architecture.md:1829`). A relation is either **local**
  (this service's own DB) or **remote** (`service: <logical-name>`, resolved
  through the app-context registry that already exists). This removes a config
  section, a class of misconfiguration, and a credential-leak surface.
- **`hierarchies` folds into `relations`** as `recursive: true`. It was never a
  separate kind of thing — a self-join needing `WITH RECURSIVE`.
- **App abilities fold into `permissions`** (ADR-0015 A2). A permission declares
  both which operations it covers (`allows:`) and which rows it reaches
  (`paths:`), so there is one definition site.

`public:` is a permission with no `paths:` — every row, for anyone **holding that
permission**. It says nothing about whether a credential is required (§2.1).

## 2. Permissions carry operations *and* rows

`allows:` lists the platform operations the permission covers; `paths:` gives the
relational rule. A grant naming `can: app/document.manage` (ADR-0015 A2) is
admitted at Tier 2 by `allows:` and filtered at Tier 3 by `paths:` — **one
declaration, two jobs**.

Entailment is **declared** (`includes:`), never derived. No inference rules to
reason about, and no way for a naming convention to imply authority.

Two rules keep the intersection intact:

- **If a policy exists for a resource, it always applies.** A grant cannot opt
  out — otherwise `can: data-layer/read` is a bypass and the intersection
  collapses.
- **If no policy exists, the grant alone decides.** This is what makes M04B
  additive: today's policy-less services keep working unchanged.

### 2.1 Defaults, per layer — and the granularity that makes them usable

**Granularity is the object type, not the policy file.** Writing a policy for one
collection does not conscript the others. Stated precisely:

- **The grant layer is default-deny.** No capability, no access. Always.
- **The policy layer is default-*absent*, not default-deny.** A resource with no
  `definitions:` entry gets no row filtering. A resource *with* a policy gets
  default-deny *within* it (§8).

**This is Postgres's model**, and saying so is more useful than deriving it:
`GRANT` is the capability, `CREATE POLICY` (RLS) is the `permissions:` block, and
a table without RLS enabled is visible to anyone holding `GRANT`. RLS is opt-in
per table. Same split, same reason. (Postgres's `FORCE ROW LEVEL SECURITY` has no
equivalent here — see Open.)

**The common case — 100 objects, 5 needing rules — enumerates nothing.** A single
`*`-selector grant (ADR-0015 A1) covers all 100; the five with `definitions:`
entries get filtered on top. A grant cannot express "all except these five"
(grants have no exclusion operator), but it never needs to: the wildcard grant
hits those five policies and they narrow it.

**Anonymous callers are admitted by interface, not by policy.** `caller = None`
is a legitimate, already-shipped state: native interfaces reject it (the B0 gate
in `crates/router/src/route_handler/dispatch.rs`), WASM guests accept it (the
guest arm never consults `caller`). So a static site declares nothing — no
policy, no rows, no capability needed by visitors. The shape is *anonymous
visitor → WASM guest → guest reads its own data as itself*. FDAE engages exactly
when the **visitor's** identity decides which rows they see — which is when
access control is what was wanted. Cost stays proportional to requirement.

## 3. Operators: `union`, `intersection`, `exclusion`

The archived spec offers only `union`, which cannot express "everyone in the
department **except** contractors" — a first-week requirement.

**Rejected: Cedar/IAM-style free-floating `deny`** that overrides an allow from
anywhere. It is the single biggest driver of "why was I denied" pain; `exclusion`
scoped inside a named permission covers the real cases and stays compilable to
SQL.

## 4. Compilation: one block, two modes

The compiled security block is the same artifact in both modes; only what it
wraps differs.

- **Mode A — Point-In-Time.** "May Alice delete document 12?" Take the block, add
  `AND documents.id = ?`, reduce to a boolean. A new explicit `check`-style host
  function — not every action is a query (delete, publish, invoke all want a point
  check with no rows to filter).
- **Mode B — Relational Filtering.** "Show me all documents I can see." Wrap the
  guest's own query with the same `WHERE EXISTS (...)`. SQLite prunes at index
  level; unauthorized rows never materialize. **Transparent** — it is what happens
  on every `data-layer::query`, no new API.

**RLS vs. CLS compile differently**: RLS is the row-pruning subquery above; CLS is
column projection/masking, driven by the intersected `fields` caveats
(ADR-0015 A3).

**Merging with the caller's filter**: two compilers, one `AND` at the end. The
caller's ADR-0007 JSON filter (`crates/data_db/src/filter.rs`), the chain's
conjoined `where` caveats, and the compiled security block all `AND` into one
statement. This is sound precisely because all three are intersective.

## 5. Non-SQL resources: one evaluator, N key extractors

Non-SQL resources do not have their own relationship graph — they have **keys into
the SQL graph**. `topic/orders/42/status` → extract `42`, evaluate on `orders`.
`blob/sha256:…` → blobs are referenced as record fields, so reach it through the
record. An external system → fetch the value and bind it as `?` (§6). The chain
always terminates in SQL, so there is **one evaluator plus a per-interface key
extractor**, not one evaluator per data-source type.

Content filters are the separate half: a `where` caveat over a *message payload*
is a predicate over JSON, not a relationship question. That needs a second
**backend** for the same DSL (`filter.rs` compiles to SQL today; the same document
could be evaluated in-memory). Coherent and cheap, but no M04B consumer —
**`where` caveats are data-layer-only for now** (ADR-0015 A3), and the in-memory
backend is the door left open.

## 6. Stage-2 cross-service fetch: batched, signed, TTL'd

Pipeline stage 2 (`system-requirements-spec.md:981`): local SQL resolves
`department:engineering`, but `manager_of(engineering)` lives in `hr-svc`. The
engine pauses, fetches via the Universal Proxy (M04A A1), injects, resumes.

**Mode B has a scaling cliff here**: 100 documents across 50 departments must not
become 50 proxy calls. You cannot push down across a service boundary, so stage 2
**must** batch and cache or Mode B does not scale.

Caching a bare value is unsafe — no expiry (so staleness is undetectable, see §8),
no provenance, nothing to show in the trace. **A stage-2 proof is therefore a
signed, TTL'd record**: *`hr-svc` asserts, at time T, `manager_of(engineering) =
david`, valid 60s.* Signing is what makes the cache safe, and it feeds the trace.

Enforcement happens at the **data-owning node**, re-verified, never trusted from
the caller's node. A fetch timeout falls back to **deny**, never silent allow.

*(The Macaroon third-party-caveat/discharge pattern is the same structure and is
worth reading before implementation. Prior art, not the argument — the argument is
batching and safe caching.)*

## 7. Stage-4 ABAC: may look things up; restrict-only

The after-step is a guest-exported function the host calls with the sieve's
candidate rows. Illustrative shape:

```wit
authorize-rows: func(ctx: auth-context, rows: list<candidate-row>)
             -> list<row-decision>;
// row-decision = allow | deny | redact(fields)
```

**It may issue read-only lookups.** M04B's task doc originally banned this citing
fetch-then-filter, WASM isolation, and performance. Only the third survives: stage
4 sees rows the sieve **already authorized**, and a component with a read-only host
import *is* the standard capability model. The N+1 cost is real but is a tradeoff
to accept knowingly for cases SQL cannot express — not a prohibition. Bounded by
ADR-0005's existing fuel quota.

**Restrict-only is a security property, not a default.** If stage 4 could *widen*,
a function querying under the service's authority could return rows the policy
denied — a real escalation. Restricted to `allow`/`deny`/`redact` over
already-approved rows, lookups are safe.

**Batched** (a batch per call, never one call per row) and **opt-in per rule**.

**Both escape hatches run under the service's own identity**, never the caller's.
That is not an escalation — the service owner authored the policy and could equally
have written the same call into their service code — and running under the caller's
authority breaks most real policies (the caller usually cannot read the org chart).
Every hatch must be **declared in the policy** and **appear in the trace** (§9).

## 8. Safety rails

Carried from `system-architecture.md:1847-1848`, with one correction: the archived
spec hardcodes a 15 ms watchdog, contradicting the architecture's own "the default
must be conservative but not hard-coded."

- `visited_track` path-concatenation cycle guard on every recursive block.
- `sqlite3_progress_handler` watchdog + **configurable** time budget →
  **default-deny** on timeout, transaction rolled back.
- Strict `?`/`:name` binding; no string concatenation, ever.
- Default-deny **within a policy** (§2.1) — an operation no `allows:` covers, or
  a row no `paths:` reaches, is denied. Not "default-deny overall": a resource
  with no policy is unfiltered, and the grant layer is what denies by default.

## 9. The decision trace

**Not optional** — it is what makes the intersection affordable, and it ships with
the first slice. Layered authorization's dominant failure is that a denial has two
possible homes (AWS IAM is formally correct and nearly unusable without a policy
simulator).

```
denied
  tier: 3 (data-plane)
  held: app/document.view          # via grant: did:key:z6MkAlice -> orders-svc
  operation_admitted: true         # `allows:` covers data-layer/read
  rows_reached: false              # `paths:` did not reach document 42
  path_failed: [creator -> management_chain -> caller]
  caveats_applied: [where {region: "EU"}]
```

`operation_admitted` / `rows_reached` together answer *what do I go edit*. `held`
names **which** grant was evaluated — necessary because independent grants unite
(ADR-0015 A4), so "denied" is meaningless without it.

The unification in §2 shrinks this problem: for an app permission there is one
definition site, and the split is not *grant vs. policy* but *operation admission
vs. row reachability* — both answered by the same `permissions:` block.

**Consequences**:

- **Enables**: Tier 3 row/column filtering compiled into SQLite; Mode A and Mode B
  from one block; a policy artifact that is analyzable and pushdown-compilable
  because it is declarative and restricted (the OPA/Rego anti-lesson) while the
  escape hatches (§6, §7) remain strictly more expressive than Rego.
- **Costs**: grants that name app permissions bind late, so a policy edit changes
  outstanding grants' meaning (argued for in ADR-0015 A2 — pinning would be worse,
  since tightening must be immediate). Stage 2 introduces a network dependency
  mid-evaluation; §6's batching and TTL'd caching are load-bearing, not
  optimizations.
- **Defers**: full MongoDB aggregation-operator compatibility; policy/relationship
  state replication (M7); the in-memory filter backend (§5).

**Resolved at acceptance (2026-07-20)**:

- **Parameter binding — what the SQL compiler binds as `?`.** The spec's
  "normalized scopes and claims" (`system-requirements-spec.md:976,978`) maps
  onto the two shipped `SessionContext` fields: **"scopes" is loose prose for
  `capabilities`** — not a separate dimension, and `Capability {with, can,
  caveats}` is its precise form (there is no `scopes`/`roles`/`env` field, and
  none is added). `capabilities` and `claims` bind *differently*:
  - `claims` (a `serde_json::Map` of scalars) bind directly as `?`.
  - a whole `Capability` does **not** bind as one `?`. Its scalar **`caveats`**
    (e.g. `{region: "EU"}` → `WHERE region = ?`) join `claims` as bound values;
    its `with`/`can` instead **select which permission/`WHERE EXISTS` branch
    compiles** (a gate, not a bound value).

  So a capability contributes both a *branch selector* (`with`/`can`) and *bound
  values* (`caveats`). No change to M04A's `SessionContext` type. The bindable
  `claims` keys and `caveat` leaves are named by the policy's `definitions:`/
  `permissions:` blocks (§1, §2), not hard-coded here.
- **Default permission when a grant names a platform ability and a policy
  exists** (§2). **Default-deny** unless the policy declares a default
  permission. Conservative and fail-closed; consistent with §8's safety rails.
- **`strict: true` mode.** Confirmed **off by default and additive** (a resource
  with no `definitions:` entry stays grant-only, as today), with an author-time
  warning when a known collection lacks a definition, and opt-in `strict: true`
  at the policy top level to deny any undefined resource. Because it is purely
  additive and only ever tightens, its **implementation is sequenced *inside*
  the first sieve slice, not ahead of it** — it does not gate the slice's start.
  The "additive-and-easy vs. fail-closed-by-construction" trade for whether
  `strict` should eventually flip to default-on is left to the point third-party
  developers author these policies.

**Open — must be settled before or during implementation**:

- **Stale relationship data (Zanzibar's "new enemy" problem).** Bob is removed
  from a group, a replica has not caught up, a stale node authorizes him. Zanzibar
  solved this with zookies (consistency tokens). M7 replication makes it real.
  **Recorded as a deliberate deferral so it is a decision, not an oversight
  discovered in M7.** §6's TTL'd proofs bound the equivalent window for
  cross-service data.
- **Tier 1 may be mis-addressed in the code.** `route_handler/dispatch.rs`'s
  `TODO(M04B/FDAE)` says which callers may reach a native service is enforced by
  FDAE "until then any verified identity passes." This ADR's position is that
  Tier 1 is a µs-scale capability check in the grant layer, not a policy-engine
  question — meaning that TODO belongs to the grant layer, not M04B, and the live
  gap (**today any verified identity reaches any native service**) is wider than
  the milestone docs imply. Reconcile in B7, which already touches this boundary.

**Alternatives considered**:

- **Rego/OPA for the data plane.** Rejected: a general-purpose policy language
  cannot be analyzed, pushed down, or guaranteed to terminate — and pushdown is the
  entire point (`system-requirements-spec.md:974`). Kept as the *anti-lesson* that
  justifies the declarative default; §7's hatch covers the expressivity case.
- **Zanzibar's centralized tuple store.** Rejected: it extracts exactly the
  data-streaming tax FDAE's data-source bindings exist to avoid — every hierarchy
  would have to be continuously normalized into one store. Zanzibar's *operators*
  and relationship model are adopted (§3); its storage model is not.
- **Policy-only, with UCAN as pure transport authn** (Fork A of the design doc).
  Simpler — one place to look, no trace needed. Rejected: loses offline
  verification, attenuated delegation, cross-substrate grants, and B7's revocable
  deploy grant, which would then need a bespoke mechanism anyway.
- **Grants pinning a policy version.** Rejected: policy *tightening* would never
  reach outstanding grants — a hole could never be closed. See ADR-0015 A2.
