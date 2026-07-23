# Slice B3 — Federated FDAE (Cross-Service Parameter Fetch): Implementation Plan

> **Status.** Draft, 2026-07-23. Anchors verified against `main` @ `65450a2`
> (B2 fully merged: Phases 1–5 green, `[FND-IAM]` (M4B) row = *In Progress
> (Slice B2 complete)*). This plan covers **Slice B3** per
> [task.md](task.md)'s slice order: B2 → **B3** → B4-fdae → B5-fdae.
>
> B3 has **two coupled deliverables**, one small and self-contained, one
> genuinely hard. Read §0 first — the split governs the whole plan.

---

## 0. Scope of B3

Two things ship together because the hard half is only *observable* once the
small half exists (a cross-service chain is the first place `caller ≠ anchor`
is real and e2e-testable — see [task.md](task.md) Slice B3, and ADR-0015 A5).

1. **`anchor_did` threading (ADR-0015 A5).** Surface the original principal —
   not the immediate proxying caller — on `SessionContext`, and let a policy
   bind `anchor` as a path terminal. This is the **confused-deputy defense**:
   a row policy on the data-owning node must filter by *who the chain acts for*,
   not by *which service is presenting the proof*. B2 shipped `caller` only and
   errors on `anchor` at [compile.rs:530](../../../../crates/fdae/src/compile.rs).
   Self-contained; touches `crates/ucan` + `crates/fdae`; fully unit-testable
   with no network.

2. **Pipeline stage 2 — cross-service parameter fetch.** When a policy relation
   lives on *another* service's node, the local SQL sieve cannot join across
   it. B2 compiles such a relation (`Relation.service: Some(_)`) to a **fail-
   closed** deny at [compile.rs:562](../../../../crates/fdae/src/compile.rs). B3
   makes it real: **pause evaluation, fetch the remote relationship proof via
   the Universal Proxy (M04A A1), inject the result as bound `?` parameters,
   resume** — enforced at the **data-owning node**, with **fetch timeout →
   deny** (never silent allow). This is the harder, cross-crate half:
   `crates/fdae` today has neither async nor a proxy dependency.

**Explicit non-scope (recorded, not dropped):**
- A5's full **`path` *list* binding** stays deferred — "no near-term consumer"
  ([task.md](task.md) Slice B3). B3 adds the `anchor` terminal only.
- **Full DataFusion/Substrait federated query orchestration** (M5). B3 fetches
  a *relationship proof* mid-evaluation; it is not a general federated planner
  ([task.md](task.md) Non-Goals).
- **Write-side Mode A** (B5-fdae) and **stage-4 ABAC** (B4-fdae) are separate
  slices.

---

## 1. Key architectural decisions (recommendations)

### 1.1 Keep `crates/fdae` async-free and proxy-free: **two-phase compile** — recommended

B2's §1.3 decision was "compilation is pure; threading lives in the store
method." B3 must not regress that: pulling `tokio` + a `ServiceProxy` handle
into `crates/fdae`'s `compile_read` would make the compiler async, couple it to
`crates/rpc`/`crates/router`, and make every existing pure unit test in
`compile.rs` async. Instead, **split the compile into plan → fetch → finalize**:

```
// crates/fdae/src/compile.rs — pure, sync, no new deps
plan_read(policy, session, service_id, collection, operation)
    -> Result<ReadPlan, PolicyError>

pub struct ReadPlan {
    /// Fully-compiled sieve when the policy is entirely local (the B2 case:
    /// no remote relations on any selected path). Ready to run as-is.
    local: Option<CompiledSieve>,
    /// Remote relationship fetches this policy needs before the local sieve
    /// can be finalized. Empty ⇒ `local` is `Some` and B3 adds nothing.
    fetches: Vec<RemoteFetch>,
    /// Deferred finalization closure/state: given the fetched id-sets, emit
    /// the final `CompiledSieve` with the remote hop replaced by a bound
    /// `id IN (?, ?, …)` predicate.
    pending: Option<PendingSieve>,
}

pub struct RemoteFetch {
    /// Logical service name from `Relation.service`, to be resolved to a DID
    /// by the *caller* (fdae stays free of the app-context registry).
    pub service: String,
    pub relation: String,
    /// The principal the remote must evaluate for — the **anchor**, not the
    /// caller (confused-deputy defense). See §3.
    pub principal_did: String,
    /// Correlation key so `finalize` can match a result back to its hop.
    pub slot: FetchSlot,
}
```

The **orchestration** (resolve logical→DID, issue the proxy call, enforce the
timeout, feed results to `finalize`) lives in the layer that *already holds a
`ServiceProxy`* — see §4 for where. This keeps `crates/fdae` a pure library and
keeps the async/network story in one place.

> **Alternative considered — async `compile_read` taking a
> `dyn RelationshipResolver`.** Rejected: it inverts the dependency (fdae →
> proxy), makes ~40 existing `compile::tests` async, and spreads timeout/fail-
> closed logic into the compiler. The two-phase split is strictly more testable
> (plan and finalize are both pure) at the cost of one extra struct.

### 1.2 The fetch returns an **id-set**, bound as an `IN`-list — recommended

For **Mode B** (relational filtering) the local sieve needs the *set of object
ids* on the far side of the remote relation that the anchor can reach, so it can
emit `<local_col> IN (?, ?, …)`. A boolean (Mode A shape) is insufficient for
row pruning. So the remote fetch is a **bounded id-set query**, evaluated under
the remote service's *own* FDAE policy (enforce at the data-owning node), with a
hard cap on cardinality (see §5, N+1/fan-out).

For **Mode A** (point-in-time, "can anchor reach row 12?") the same id-set
answers it — membership test against the returned set — so one fetch shape
serves both modes. The id-set travels **inside a signed, TTL'd record**
(ADR-0017 §6, §3.2 below) — the `IN`-list binds from the record's `ids`; the
signature/`valid-until` feed cache-safety and the decision trace.

### 1.3 Fetch rides `ProxyRouter` as `CallOrigin::Native` — already reserved

[proxy.rs](../../../../crates/rpc/src/proxy.rs)'s `CallOrigin::Native` doc
comment already names this exact consumer: *"the FDAE policy engine's
relationship-proof fetch (M04B B3) … enforcement for these lives at the data-
owning node."* No new transport — B3 constructs a `ProxyRequest` with
`origin: Native` and the anchor's proof, and the remote node re-verifies and
builds a fresh `CallerContext` keyed on the **anchor** (§3). The interim coarse
gate `TODO(M04B/FDAE)` in `router/src/proxy.rs` is the wire-in point.

---

## 2. The `anchor_did` half (ADR-0015 A5)

### 2.1 `SessionContext` gains `anchor_did` — `crates/ucan/src/session.rs`

```rust
pub struct SessionContext {
    pub subject_did: String,          // unchanged: the immediate caller (leaf audience)
    pub anchor_did: Option<String>,   // NEW: the original principal, when caller ≠ anchor
    pub capabilities: Vec<Capability>,
    pub claims: serde_json::Map<String, serde_json::Value>,
    pub verified_at_secs: u64,
}
```

`Option`, because a **direct** call (no proxy hop) has `anchor == caller`; the
policy compiler treats "`anchor` requested but `anchor_did == None`" as *fall
back to `subject_did`* (a direct caller **is** the anchor) — **not** a deny.
Confirm this reading in §7 D-B3-1: the alternative (anchor absent ⇒ deny) would
break every direct call against an `anchor`-terminal policy.

### 2.2 Populate it via an **explicit, signed anchor stamp** (D-B3-2 — RESOLVED)

> **Decision (2026-07-23).** We do **not** structurally *derive* the anchor by
> walking the chain and guessing which node is the principal. That is ambiguous
> — the principal sits in a *different structural slot* per root shape (the
> root's **audience** in admin-rooted, the root's **issuer** in owner-rooted),
> and no shape-walk satisfies both without a human-vs-service type tag that DIDs
> don't carry. Instead, adopt the industry pattern (OAuth 2.0 Token Exchange /
> On-Behalf-Of `sub` vs `act`; Kerberos S4U): **the principal stamps itself as
> the anchor at origination, and the stamp propagates immutably down the chain,
> protected by each issuer's signature.** This handles all three shapes with a
> *single* invariant and leaves the compiler reading one already-verified field.
>
> **This supersedes ADR-0015 A5's structural "audience of the first non-root
> token" wording** — that description was a derivation heuristic, and B3 replaces
> it with an explicit stamp. **Requires an ADR-0015 amendment** (A5 is Accepted;
> add a dated amendment block following ADR-0015's own prior-amendment precedent,
> which cites "the convention of ADR-0007/ADR-0011" — *not* `session-strategy.md`,
> which only mandates serializing shared-ADR edits, no supersession ritual). The
> same stale "audience of the first non-root token" wording also lives in
> **[task.md](task.md)'s own Slice B3 paragraph** and in **`access-control-design.md`**
> (which task.md already flags as needing a B3-ship update) — clean up all three,
> not just the ADR. See §7 D-B3-2 and the doc-hygiene list (§10).

**Token field.** `CapabilityToken`
([token.rs](../../../../crates/ucan/src/token.rs)) gains a **signed**
`anchor_did: Option<String>` (added to `signing_value()` so it is covered by the
issuer signature — a middle service cannot rewrite it without invalidating its
own token):

```rust
pub struct CapabilityToken {
    pub issuer_did: String,
    pub audience_did: String,
    pub anchor_did: Option<String>,   // NEW, signed: the original principal
    // capabilities, facts, expiry, proofs, signature …
}
```

**The propagation invariant** (enforced in `verify_chain` /
`granted_capabilities`, fail-closed). For every token in the verified chain, if
`anchor_did` is `Some(a)` then **exactly one** of:
1. `a == token.issuer_did` — **self-declaration.** The issuer signs the token, so
   asserting *itself* as the anchor is truthful by construction. This is how a
   principal originates: when `user_A` delegates to `service_1`, `user_A` sets
   `anchor_did = user_A`.
2. some proof `p` with `p.audience_did == token.issuer_did` carries
   `p.anchor_did == a` — **inheritance.** A service copies its parent's anchor
   unchanged when it delegates onward.

Any other value → the token is rejected (a service **cannot upgrade** the anchor
to a principal it wasn't delegated from — the confused-deputy defense). A
service *can* self-declare `anchor = itself` (downgrade to "acting as myself"),
which is harmless. `SessionContext.anchor_did = leaf.anchor_did` (already
verified) — no separate walk, no `derive_anchor` heuristic.

**Why this covers all three shapes with one rule** (user priority: owner-rooted
first, then admin-rooted, then system-as-itself — but the mechanism serves all
three simultaneously; the priority only orders which we lead with in tests/e2e):

| Shape | Chain | Anchor result |
|---|---|---|
| **Owner-rooted (primary)** | `user_A(root) →[anchor=user_A] svc_1 →[anchor=user_A] svc_2` | `user_A` ✓ (rule 1 at origin, rule 2 downstream) |
| **Admin/platform-rooted** | `root_admin → user_A →[anchor=user_A] svc_1 →[anchor=user_A] svc_2` | `user_A` ✓ (`user_A` self-stamps when it first delegates to a service; the `root_admin→user_A` grant carries no anchor) |
| **System-as-itself** | `svc →[anchor=None or svc] …` | `None`→ falls back to `subject_did = svc` (D-B3-1); or self-stamped `svc`. A system **cannot** stamp `anchor = user_A` without `user_A`'s delegation — correct: "cron for user_A" *requires* user_A's up-front consent token (exactly OAuth's model) |

**`issue()` API.** Add an anchor argument (or an `issue_with_anchor` helper) so
an originating principal sets `anchor = self` and a proxying service passes
through its parent's anchor. Default `None` keeps a bare direct grant
anchor-less (→ fallback). Note the "no migrations pre-release" policy: change the
token shape **in place**, no compat shim — but the new signed field changes the
signature payload, so any pre-existing persisted/in-flight tokens are invalid
(acceptable pre-release; call it out so nobody treats it as a regression).

### 2.3 Compiler: `anchor` terminal — `crates/fdae/src/compile.rs`

Replace the B3-stub at [compile.rs:527-534](../../../../crates/fdae/src/compile.rs):

```rust
fn terminal_value(terminal: &str, session: &SessionContext) -> Result<String, PolicyError> {
    match terminal {
        "caller" => Ok(session.subject_did.clone()),
        "anchor" => Ok(session.anchor_did.clone().unwrap_or_else(|| session.subject_did.clone())),
        other => Err(PolicyError::Semantic(format!("unknown path terminal '{other}'"))),
    }
}
```

`policy.rs` already **accepts** `anchor` at parse time
([policy.rs:387](../../../../crates/fdae/src/policy.rs),
`accepts_anchor_terminal_at_parse_time`); only the compile-time stub changes.
Update `remote_relation_fails_closed_at_compile_time`'s sibling test naming so
the `anchor`-now-supported change is visible, and flip
`compile.rs`'s "not implemented in this slice (B3)" comment.

---

## 3. The cross-service fetch half (pipeline stage 2)

### 3.1 What "fetch a relationship proof" means concretely

A remote relation names another service that *owns* the relationship data
(`Relation.service: Some("hr-svc")`). To prune local rows by it, the sieve needs
the set of far-side ids the **anchor** can reach on that service. B3:

1. **plan** (§1.1) walks the selected permission paths; each hop with
   `relation.service.is_some()` becomes a `RemoteFetch{service, relation,
   principal_did: anchor, slot}` and the path is compiled with a placeholder
   `IN (:slot)` predicate instead of a local join.
2. **resolve** the logical `service` → service DID via the existing app-context
   registry (`system-architecture.md:1881-1883` — the same resolver B2's
   docs point at; fdae stays out of it, the orchestration layer does this).
3. **fetch**: issue a `ProxyRequest{ target_service: <DID>, interface:
   "data-layer", method: <relationship-proof method, §3.2>, params: {relation,
   principal: anchor}, caller: <anchor's CallerContext/proof>, origin: Native }`
   through the `ServiceProxy`. **The remote node re-verifies the anchor's proof
   and runs *its own* FDAE policy** — this is the "enforce at the data-owning
   node" invariant; the local node does not trust an id-set it computed itself.
4. **finalize**: bind the returned id-set into the `:slot` `?` params and hand
   the finished `CompiledSieve` to `data_db` exactly as B2 does.

### 3.2 The relationship-proof wire method — **decision D-B3-3 (§7)**

Two viable shapes; the plan must pick one before coding:
- **(a) Reuse `data-layer::query`** against the remote relation's backing
  collection with a projection of just the id column. Zero new WIT surface;
  the remote's existing FDAE sieve filters it for the anchor for free. Risk:
  the caller must know the remote's *physical* collection/column, which the ADR
  deliberately keeps out of policies (logical vocabulary only).
- **(b) A new additive `data-layer::resolve-relation` WIT export** — logical,
  matches the ADR's "policies never carry connection strings," and lets the
  remote map logical→physical with *its own* `definitions:`. Minor, non-breaking
  WIT bump ([task.md](task.md) WIT Boundary Versioning). **Recommended (b)** — it
  is the only shape consistent with the logical-vocabulary invariant, and it
  keeps the remote free to evolve its schema. Cost: one new WIT method + host
  impl + guest binding regen, and `wasm32-wasip2` must stay unbroken.

**Return shape — a *signed, TTL'd record*, not a bare `list<id>`
(ADR-0017 §6).** The ADR is explicit: *"A stage-2 proof is therefore a signed,
TTL'd record: `hr-svc` asserts, at time T, `manager_of(engineering) = david`,
valid 60s. Signing is what makes the cache safe, and it feeds the trace."* So
`resolve-relation` returns something like `{ asserter: service-did, relation,
principal, ids: list<id>, valid-until-secs, signature }`, signed by the remote's
identity. Two distinct reasons, and **only the first is contingent on caching**:
1. **Cache safety** — a proof reused later (D-B3-6, deferred) must be self-
   authenticating out of band of the original connection. *This* justifies
   deferring the cache, not the signature.
2. **Trace provenance** — even for an *immediate, uncached* B3 fetch, the
   `DecisionTrace` must record *which service asserted what, valid how long*.
   Checked `crates/fdae/src/trace.rs`: `DecisionTrace` has **no** remote-fetch
   field today, and §3.3's only trace addition is the *failure* path — a
   successful fetch currently leaves no provenance at all. That is an
   unacknowledged narrowing of ADR-0017 §6, so B3 **must** add remote-fetch
   provenance to the trace regardless of caching.

Nuance worth stating (not a reason to skip signing): the fetch already rides an
**authenticated** proxy channel, so transport auth covers *integrity for
immediate use*. Signing is what buys **(1)** later-cache safety and **(2)**
cryptographic (not merely connection-level) provenance in the trace. Recommend
shaping the signed record **now** — the signing infra already exists
(`Identity::sign_json` / `verify_json_signature`), and doing it now makes the
D-B3-6 cache a pure additive follow-up with **no** wire-format churn. See §7
D-B3-3 (updated) and D-B3-6.

### 3.3 Fetch must carry the **anchor**, and the timeout is **deny**

- The `principal` in the fetch is `session.anchor_did` (falling back to
  `subject_did` for a direct caller) — never the proxying service. A remote
  policy that filtered by the *caller* service would be the confused deputy.
- **Timeout / transport error / remote-deny → the whole read denies closed**
  (Mode B ⇒ empty result, Mode A ⇒ `false`), mirroring B2's watchdog default-
  deny. This is Failure/Security matrix **row 6**. Budget: reuse the proxy's
  existing per-call `Duration` (proxy.rs `ProxyRequest`), with an FDAE-specific
  ceiling; a fetch that overruns is a deny, logged via the existing
  `fdae::DecisionTrace` (`tracing`) with a new `remote_fetch_timeout` reason.
- **A *successful* fetch is also traced** (§3.2 reason 2): add remote-fetch
  provenance to `DecisionTrace` — asserter service DID, relation, principal, and
  `valid_until_secs` — so the deny path and the allow path both leave a record.

---

## 4. Where the orchestration lives (the `ServiceProxy` seam)

`plan_read`/`finalize` are pure and live in `crates/fdae`. The **async fetch
loop** needs a `ServiceProxy` handle + the app-context registry, both of which
already exist on the host read path B2 built:

- **WASM read path:** `HostState.fdae_policy` (B2 Phase 3/4) already carries the
  compiled policy at `host_capabilities.rs`; the same `HostState` reaches a
  `ServiceProxy` for the guest `syneroym:proxy` host function. The stage-2 loop
  slots in **between** `plan_read` and the existing `data_db` call.
- **Native dispatch path:** `router`'s native `data-layer` dispatch
  (`dispatch_json_rpc_once`, the B2 Phase-4 enforcement ingress) holds the
  `ProxyRouter`. Same insertion point.

Recommended shape: a small **`fdae-runtime`-style helper** (a function, not a
new crate) in the host/router layer — `resolve_fetches(plan, proxy, registry,
anchor, deadline) -> Result<Vec<FetchResult>, Deny>` — that both ingresses
call, so the timeout/fail-closed/tracing policy is written once. Do **not** put
it in `crates/fdae` (keeps that crate proxy-free per §1.1).

> **D-04-02-h interaction — two ingresses, not one.** B2 left guest-originated
> reads carrying a capability-less synthesized identity
> (`CallerContext::service_system`), so they compile to `deny_all()` and return
> empty. task.md's D-04-02-h names **two** ingresses, and anchor threading closes
> them differently:
>
> - **Ingress (ii) — guest self-proxy** (`host_capabilities.rs:670`,
>   `proxy.rs:224-231/251-265`). A guest calls its *own* native `data-layer`
>   through `syneroym:proxy`; there *is* a proxy hop, so the anchor rides the
>   presented proof and the callee re-verifies it. This ingress has a **natural**
>   fix under the stamp: forward the original principal's chain, recover
>   `anchor_did`, filter by it. **In-scope-able for B3.**
> - **Ingress (i) — direct WASM host-function read** (`engine.rs:711-716`,
>   `prepare_wasm_execution`, reaching the store via `HostState`). There is **no**
>   proxy hop and **no** forwarded proof to re-verify — the guest just calls a
>   host function. So the anchor can't be "recovered from a chain" here; it must
>   be **threaded from the component's *triggering* invocation** (the inbound
>   request that caused the guest to run) into `HostState`, then consulted. That
>   is a meatier `crates/router`↔`crates/sandbox_wasm` cross-cut than (ii).
>
> **Decision D-B3-4 (§7):** does B3 *close* D-04-02-h or only *unblock* it — and
> **per-ingress**? Recommendation: close **(ii)** in B3 (natural under the
> stamp), and **unblock but explicitly defer (i)** unless the triggering-anchor
> thread proves cheap — recording the deferral, not silently dropping it. This
> directly governs reference-scenario **step 22** (§6, §9) and which of the two
> pinned regression tests flip (§6).

### 4.1 The three named `TODO(M04B/FDAE)` wire-in points — disposition

task.md's Current State Inventory names three in-code `TODO(M04B/FDAE)` seams.
State where each lands, so none is silently ignored:

- **`router/src/proxy.rs:192`** — the interim coarse Tier-3 gate. **B3 (this
  slice)**, §1.3/§4 — the fetch orchestration replaces it.
- **`route_handler/dispatch.rs:84`** — **already reworded** in code to
  `TODO(B7b / post-B7)`, and now explicitly disclaims itself as "**NOT** an
  FDAE/M04B policy question." task.md's inventory is **stale** here; the plan
  correctly ignores it (it is a grant-layer admission check, not Tier-3 ReBAC).
- **`control_plane/src/service.rs:162`** — **still live and still tagged
  `M04B/FDAE`** verbatim; gates `inject-kek`/security ops on `substrate/admin`.
  But it is a **platform-ability** check — literally the sibling of
  `has_node_wide_ability`'s `caller.has_capability(substrate(node), …)` right
  above it — **not** row/column ReBAC. It therefore needs **no** B3 deliverable:
  it is a Tier-2 grant-layer gate belonging with the **write-side / grant-layer
  work (B5-fdae, or a B7 follow-on)**, the same class as the reassigned
  `dispatch.rs` seam. **Out of B3 scope — named and excluded, not ignored.** The
  `M04B/FDAE` tag on it should be reworded when that slice lands (mirroring the
  `dispatch.rs` cleanup).

---

## 5. N+1 / fan-out containment

A remote hop inside a **recursive** relation, or multiple remote relations on
one permission, can multiply fetches. B2's H5 (recursive-CTE row blowup) is a
sibling risk. Guardrails for B3:

- **Cap fetch count per query** (e.g. one fetch per distinct
  `(service, relation)` slot, de-duplicated — not per candidate row). The
  two-phase plan naturally de-dupes: `RemoteFetch`es are collected from the
  *policy paths*, which are bounded by `MAX_PATH_HOPS = 32` (B2 H4).
- **Cap returned id-set cardinality** (bound the `IN`-list; overflow → deny,
  not a giant query). Pick the bound in D-B3-3.
- **No remote hop *inside* a recursive CTE in B3** — a remote relation as a
  recursive `from_key`/`to_key` self-join would require iterative cross-node
  fetches (a distributed transitive closure). Defer that shape: `plan_read`
  should **error closed** if a `recursive: true` relation is also
  `service: Some(_)`, with a clear "unsupported in B3" message. Confirm this is
  already structurally impossible or add the guard (D-B3-5, §7).

---

## 6. Tests

**Unit (`crates/ucan`):** the anchor-stamp propagation invariant (§2.2) over a
table of chain shapes — owner-rooted (primary), admin/platform-rooted, system-
as-itself/direct, 3-hop pass-through, and the **attack cases**: a middle service
rewriting `anchor_did` to an un-delegated principal → rejected; a self-declared
downgrade → accepted; signature covers `anchor_did` (tamper → verify fails).
`SessionContext.anchor_did == leaf.anchor_did` after `from_verified_chain`.

**Unit (`crates/fdae`):** `plan_read` splits local vs remote correctly; the
`anchor` terminal compiles to the right bound value; a remote+recursive relation
errors closed; placeholder `IN (:slot)` emission; `finalize` binds an id-set.

**Integration (`crates/data_db` / `crates/sandbox_wasm`):** Mode A + Mode B with
a *stubbed* `ServiceProxy` returning a fixed **signed** record (no real second
node) — proves the plan→fetch→finalize→SQL wiring and the `caller ≠ anchor`
filtering. Timeout/error stub → deny (matrix row 6). Verify the trace records
successful-fetch provenance (asserter DID, `valid_until_secs`), not just the
timeout path.

**D-04-02-h regression tests (flip iff D-B3-4 resolves "close").** These two
pin today's over-restrictive empty-result behavior and must be *flipped* (not
deleted) when their ingress is closed — the same treatment task.md gives
D-04-02-g's pinned tests ("flip the assertion"):
- Ingress (ii): `crates/router/tests/proxy_dispatch.rs:459`
  `guest_self_proxy_data_layer_returns_empty_when_policy_present` — flip if B3
  closes (ii) per the D-B3-4 recommendation.
- Ingress (i): `crates/sandbox_wasm/tests/data_layer_integration.rs:173`
  `test_deployed_policy_yields_empty_guest_originated_query_d04_02_h` — flip
  **only if** B3 also closes (i); otherwise it stays asserting empty, with the
  deferral recorded.

**E2E (`mise run test:e2e`, ≥2 substrates):** reference scenario **steps
22–23** (task.md's exit criteria names *both*, not 23 alone):
- **Step 23** — a ReBAC check requiring a remote relationship proof triggers a
  real cross-service fetch via the Universal Proxy mid-query, filtered by the
  anchor. The first genuine `caller ≠ anchor` end-to-end assertion.
- **Step 22** — its still-open half ("unauthorized rows *never reach the WASM
  guest*") is the one task.md says "resolves alongside B3's `anchor_did`." It
  closes for a guest-originated read **exactly to the extent D-B3-4 closes the
  ingress**: (ii) self-proxy in B3; (i) direct host-function read only if that
  ingress is also taken. Commit to the (ii) half of step 22 and state the (i)
  half's disposition explicitly (§9).

Rebuild all five `wasm32-wasip2` `test-components` first (B2 Phase 5 process).

**Perf:** [task.md](task.md) budget — *Federated FDAE fetch (one cross-service
hop) < 50 ms p99*, network-bound, "a floor not a hard SLA." Integration test,
two local nodes; document measured in `status.md`.

**Failure/Security matrix:** flip **row 6** (cross-service fetch timeout →
deny) from ⛔ Deferred to ✅ with evidence.

---

## 7. Ambiguities / decisions — RESOLVE BEFORE/DURING CODING

- **D-B3-1 — `anchor` requested but `anchor_did == None`.** Recommend: fall
  back to `subject_did` (a direct caller *is* the anchor), **not** deny.
  Pin with a test. (§2.1)
- **D-B3-2 — anchor mechanism. ✅ RESOLVED (2026-07-23): explicit signed
  stamp**, not structural derivation. `CapabilityToken` gains a signed
  `anchor_did`; a principal self-declares it and it propagates immutably
  (verify-enforced, fail-closed), covering owner-rooted/admin-rooted/system
  with one invariant. **Supersedes ADR-0015 A5's "audience of the first non-root
  token" wording → needs an ADR-0015 amendment recording the supersession.**
  Follow-up: A5's full `path` *list* binding stays deferred (§0). (§2.2)
- **D-B3-3 — relationship-proof wire method, *return shape*, + cardinality
  bound.** Recommend a new additive `data-layer::resolve-relation` WIT export
  (logical vocabulary) over reusing `query`; **return a signed, TTL'd record
  (ADR-0017 §6), not a bare `list<id>`** — so the trace gets cryptographic
  provenance now and the D-B3-6 cache is a pure add later; pick the `IN`-list
  cap. (§3.2, §5)
- **D-B3-4 — close vs. unblock D-04-02-h, *per ingress*.** Recommend: close
  **(ii) self-proxy** in B3 (anchor rides the forwarded proof); **unblock but
  defer (i) direct host-function** read (anchor must come from the triggering
  invocation, a bigger cross-cut) unless cheap. Governs step 22 + which pinned
  test flips. Record any deferral, don't drop it. (§4, §6)
- **D-B3-5 — remote-inside-recursive relation.** Defer with an explicit
  error-closed guard in `plan_read`; confirm it isn't already impossible. (§5)
- **D-B3-6 — fetch result caching/TTL.** ADR-0017 §6 calls batching + TTL'd
  caching "load-bearing … or Mode B does not scale." B3 may still ship **no
  cache** first (correctness over scale), *provided* the §3.2 signed-record
  shape lands now so caching is a pure add with no wire churn. Confirm no
  correctness dependency on caching. (§3.2, §5)
- **D-B3-7 — fail-closed *granularity* of the anchor invariant.** Today's
  `verify_chain` fails closed **per capability** (an unbacked capability is
  silently dropped from the returned `Vec`, never an `Err`); structural failures
  (bad signature, expiry, audience) are hard `Err`. A token asserting an
  `anchor_did` it cannot substantiate — which of the two? **Recommend hard
  `Err`** (reject the whole verification): the anchor is a single chain-wide
  *provenance* assertion, not one authority claim among many, so there is no
  "keep the valid siblings" case a per-capability drop would serve; Err avoids
  the partial-view/inheritance subtlety (a *dropped* anchor must also become
  un-inheritable by descendants, which a mutate-the-view drop makes fiddly to
  reason about); and a well-formed chain **never** carries an unsubstantiated
  anchor, so legitimate callers are never punished. Alternative (per-token drop
  to `None` → caller-only semantics) is *also* fail-closed but silent; if chosen,
  it must still emit a `DecisionTrace` so the drop is observable. **Pick before
  writing the invariant** — it is the crux of the confused-deputy defense. (§2.2)

---

## 8. Suggested execution order (phases)

1. **Phase 1 — anchor stamp (self-contained, no network).** Signed
   `CapabilityToken.anchor_did` + the propagation invariant in `verify_chain`
   (D-B3-2); `issue()`/helper API; `SessionContext.anchor_did = leaf.anchor_did`;
   `anchor` terminal in `crates/fdae` (D-B3-1). Fail-closed granularity per
   D-B3-7. Full unit coverage incl. the attack cases (§6). Record the **ADR-0015
   A5 amendment** + the task.md/`access-control-design.md` prose cleanup (§10).
   Ships the confused-deputy *vocabulary* before the fetch exists; merges green
   on its own.
2. **Phase 2 — two-phase compile (`plan_read`/`finalize`, pure).** `RemoteFetch`
   / `ReadPlan` / `PendingSieve` in `crates/fdae`; remote relation → placeholder
   `IN (:slot)`; remote-inside-recursive guard (D-B3-5). Pure unit tests with a
   hand-built id-set into `finalize`. No async yet.
3. **Phase 3 — WIT `resolve-relation` + host impl (D-B3-3).** Additive WIT
   method returning a **signed, TTL'd record** (not a bare `list<id>`); guest
   binding regen, `wasm32-wasip2` unbroken.
4. **Phase 4 — orchestration seam (§4).** `resolve_fetches` helper on both
   ingresses (WASM host path + native dispatch); anchor-carrying `ProxyRequest`
   with `origin: Native`; timeout→deny **and** successful-fetch provenance in
   `DecisionTrace`. Integration tests with a stubbed proxy. Address D-04-02-h
   per D-B3-4 — close ingress (ii), flip its pinned test; disposition (i).
5. **Phase 5 — e2e + perf + docs.** Two-substrate e2e (reference **steps
   22–23**, step 22 to the extent D-B3-4 closes an ingress), 50 ms-hop perf in
   `status.md`, Failure/Security matrix row 6 → ✅, and **update
   `traceability-matrix.md`** (`[FND-IAM]` M4B row → "In Progress (Slices B2, B3
   complete)", mirroring B2's precedent — *not* Complete; B4/B5 remain).
   Exit-criteria sweep (fmt/clippy/test/e2e).

---

## 9. Exit criteria (B3 subset of task.md's Measurable Exit Criteria)

- [ ] `cargo +nightly fmt --all` clean; `clippy --workspace --all-targets
      --all-features` zero warnings; `cargo test --workspace` green;
      `mise run test:e2e` green; `wasm32-wasip2` unbroken.
- [ ] Signed `anchor_did` stamp + propagation invariant land; `anchor` terminal
      compiles (D-B3-1/-2); **ADR-0015 A5 amended** to record the supersession.
- [ ] Federated cross-service fetch works over the Universal Proxy; response is
      a **signed, TTL'd record** (ADR-0017 §6); **timeout → deny** verified
      (Failure/Security row 6); a *successful* fetch records provenance in
      `DecisionTrace` (asserter DID, `valid_until_secs`).
- [ ] Reference scenario **steps 22–23** execute end-to-end across ≥2 substrates
      (step 22 to the extent D-B3-4 closes an ingress; state (i)'s disposition).
- [ ] Federated-hop perf documented in `status.md` (< 50 ms p99 floor).
- [ ] D-04-02-h dispositioned per D-B3-4 (per-ingress: closed, or explicitly
      still-open with the reason recorded — not silently dropped); the pinned
      regression test for any closed ingress is *flipped*, not deleted.
- [ ] **`traceability-matrix.md` `[FND-IAM]` (M4B) row updated** to reflect B3
      completion ("In Progress (Slices B2, B3 complete)"), mirroring B2's
      closeout — not flipped to Complete (B4-fdae/B5-fdae remain).
- [ ] Doc-hygiene (§10): ADR-0015 A5 amended; the stale "audience of the first
      non-root token" wording in task.md + `access-control-design.md` cleaned up.

---

## 10. Doc-hygiene — supersessions & stale references to clean up

The explicit-stamp decision (D-B3-2) invalidates the same "audience of the first
non-root token" wording in **three** places; fixing only the ADR leaves the
other two rotting:

- **[ADR-0015](../../../decisions/0015-ucan-capability-model.md) A5** — add a
  dated amendment block (follow ADR-0015's own prior-amendment precedent citing
  "the convention of ADR-0007/ADR-0011"). This is the authoritative fix.
- **[task.md](task.md) Slice B3 paragraph (~line 476)** — still says "populated
  in `from_verified_chain` as the audience of the first non-root token." Reword
  to the stamp.
- **`access-control-design.md` (~:996)** — task.md *already* flags this line as
  needing a B3-ship update (its "B7 is the first real consumer" A5 note); the
  stamp change is that update.

Also on B3 completion:
- **`route_handler/dispatch.rs`** — task.md's Current State Inventory still lists
  its `TODO` as `M04B/FDAE`, but the code already reworded it to `B7b / post-B7`.
  Correct the inventory (stale, not a code change).
- **`control_plane/src/service.rs:162`** — reword its `TODO(M04B/FDAE)` when the
  grant-layer/write slice (B5-fdae or B7 follow-on) that owns it lands; out of
  B3 scope (§4.1), but recorded so it isn't lost.
