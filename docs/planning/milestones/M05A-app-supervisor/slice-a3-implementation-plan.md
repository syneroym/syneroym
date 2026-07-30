# Slice A3 Implementation Plan — Multi-Substrate Placement and the Substrate Inventory

**Status:** 📋 Planned (2026-07-30). Not started. Milestone:
[task.md](task.md) slice **A3**. Design of record:
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§1/§5 (push, best-effort delivery) and
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1/§3/§6
(member master, instance key derived per hosting node, master-signed endpoint
record). Depends on **A0, A1, A2, P0 — all Complete**. Gates A4 and A5.

**The one-sentence summary.** A manifest gains a placement selector naming a
substrate **alias**; an operator-held inventory file maps each alias to a DID,
an address, a credential, and a declared capability set; `roymctl app deploy`
resolves the two, deploys each service to *its own* substrate one call at a
time, records a journal action row per (service, substrate), and leaves the
deployment `Degraded` — not rolled back — when some of those calls fail.

**Read §0 first.** Planning found **fifteen** places where `task.md`'s A3
paragraph describes a tree that does not exist, leaves a decision unmade, or
understates the work. **Seven** of them change what A3 has to build. §1's
decisions (D-A3-1 … D-A3-22) take §0's recommended resolutions as given; where
a decision is genuinely the requester's, it is listed again in §12.

**Review round 2 (2026-07-30), all four findings incorporated.** Two were gaps
the *round-1 fixes themselves* opened, which is the reason for a second pass:

- **D-A3-12's refusal was blind to `Degraded`.** Round 1 made `Degraded`
  first-class (D-A3-18) but left the refusal keyed on the last `ACTIVE` record,
  so the sequence A3 itself introduces — partial deploy leaves `Degraded`, the
  operator then edits placement — passed with no refusal and left a service
  running on two nodes republishing its record: exactly the two-publisher state
  D-A3-12 exists to prevent. Now sourced from `COMPLETED` action rows across
  every record for the instance and compared on `substrate_did` (D-A3-22), which
  also moves the check after the preflight and adds one journal query.
- **The post-apply check overclaimed.** It dials `api_url`, and §0.12's own
  premise is that `api_url` need not be the registry a substrate publishes into.
  So it proves "the registry at this URL cannot resolve M", never "substrate X
  cannot". Kept — it is the only signal available before a user-facing call
  fails — but reworded as an explicit heuristic whose warnings name the URL
  probed, and renamed `probe_registry_reachability`.

Plus: §9's test 6 discharges **two** backlog rows, not one (it is also A0's
missing `CallOrigin::Guest`-over-a-real-hop evidence), which is what prices §12
question 8.

**Review round 1 (2026-07-30), all eleven findings incorporated.** Four were
correctness gaps in this plan, not in `task.md`: `recover_applying` gates on
`Applying` and would have gone blind exactly when `Degraded` exists (§0.2's
own surface, fixed in §4.4); the journal record was created *before* the two
checks that can bail, so a refusal would strand a phantom record (§7.2's order
fixed); the fallback client was built unconditionally, so a fully-placed
deploy needed a default substrate it never uses (D-A3-20); and D-A3-12's error
told the operator to run `svc remove --svc-id <id>` with an id nothing can give
them (§7.2). One was a bad undercount — `map_deployment_plan_to_wit` has
**13** call sites, not two (§8.3). The largest addition is §0.12: cross-substrate
resolution silently requires **one registry namespace across the whole
inventory**, which nothing in `task.md` or the original draft said.

**Two facts in this plan were verified by running code, not by reading it**,
because both would have sent an implementer down a wrong path:

- An externally-tagged enum nested inside a struct that uses
  `#[serde(flatten)]` round-trips correctly through **both** TOML and JSON
  (probed against the real `ServiceSpec` shape). So `PlacementSelector` can be
  an enum from day one — §2.2's design does not need a fallback.
- `toml::to_string` emits plain values before tables regardless of Rust field
  declaration order (probed with a non-empty `resolved_dependencies` alongside
  `topology_mode`). So a new field on `PlannedService` may be declared wherever
  it reads best; there is no ordering hazard to design around.

---

## §0 — What `task.md` gets wrong, leaves open, or understates

Same discipline as A0 §6 / A1 §6 / A2 §0 / P0 §0: recorded here rather than
silently worked around, so `task.md` can carry a dated correction at sign-off.

`task.md`'s entire A3 text is one sentence with six semicolons. Everything
below is what that sentence does not say.

### 0.1 (Scope-changing) `--mint-masters` is single-substrate *by construction*, and multi-substrate placement breaks it silently

This is the largest finding and the one most likely to be discovered late, as a
confusing deploy rejection.

[`substitute_and_certify_members`](../../../../apps/roymctl/src/commands/member_identity.rs#L172)
takes **one** `client` and uses it for every service in the plan. Two things it
produces are per-*hosting-substrate*, not per-plan:

1. **The instance certificate.** The substrate derives the instance key as
   `node_identity.derive_service_identity(&caller.caller_did, &service_id)`
   ([orchestration.rs:562](../../../../crates/control_plane/src/service/orchestration.rs#L562)),
   so it depends on the **node** *and* the **calling DID**. Deploy's
   install verification recomputes that derivation and rejects a mismatch with
   `"instance certificate certifies '<x>', not the key this substrate would
   derive"`
   ([orchestration.rs:731-745](../../../../crates/control_plane/src/service/orchestration.rs#L731)).
   A certificate minted by querying node A is therefore **rejected at deploy by
   node B**, always.
2. **The endpoint record.** `EndpointInfo.substrate_id` is set from
   `client.service_id()`
   ([member_identity.rs:242](../../../../apps/roymctl/src/commands/member_identity.rs#L242)).
   `substrate_id` is signed, and it is what indirect resolution follows to find
   the hosting node's mechanisms
   ([dht_registry.rs:386](../../../../crates/core/src/dht_registry.rs#L386)). A
   record naming the wrong substrate does not fail loudly — it points every
   resolution of that member at a node that is not hosting it.

**Consequence:** A3 cannot just "record the resolved substrate on
`PlannedService`" and route the deploy call. Certificate minting and record
signing must both move to a per-(member, substrate) loop, and the client used
to mint must be the same client used to deploy (because the derivation includes
the caller DID). `task.md`'s A3 paragraph does not mention identity at all.

**Recommendation:** §6 rebuilds this path, and moves the per-substrate half into
`crates/sdk` so the two-substrate e2e can prove it (§9). Without the move, the
single most breakable thing in the slice would be covered by unit tests only.

### 0.2 (Scope-changing) The journal has never written an action row

`append_action` and `update_action_state` have **zero** production callers —
verified by grep over the whole tree; the only reference to any of the three
action APIs is `get_completed_actions`, read by
[`recover_applying`](../../../../crates/app_orchestration/src/reconcile.rs#L62),
whose `retain` filter is consequently always a no-op over an empty list.

So "per-(service, substrate) journal action records" is **not** "add a column to
`deployment_actions`". It is building the writer for the first time, and making
`recover_applying`'s filter mean something. Sized accordingly in §4.

### 0.3 (Scope-changing) `deploy-plan` is all-or-nothing on the wire, so per-service records need a call-shape decision

`ControlPlaneService::deploy_plan` loops over `plan.services` and returns `Err`
on the first failure
([orchestration.rs:594-641](../../../../crates/control_plane/src/service/orchestration.rs#L594)),
with no indication of which services already succeeded. Today
`roymctl app deploy` makes exactly **one** such call for the whole app.

That is incompatible with failure-matrix row 12 ("partial app deploy, 3 of 5 —
failed services retried"), which requires knowing *which* three. Three options:

| | Shape | Cost |
|---|---|---|
| (a) | One `deploy-plan` per substrate | Row granularity is a lie: a service that deployed before the failure is recorded `FAILED` |
| (b) | Per-service `orchestrator/deploy` | **WIT change** — `deploy` carries no `app-context`, so bindings would be lost |
| (c) | One `deploy-plan` per (service, substrate), each carrying a single-service plan | N RPCs instead of 1; no wire change; exact granularity |

**Recommendation: (c).** Deploy is a cold path, the extra calls are on an
already-open connection, and it is the only option that makes the journal rows
`task.md` asks for truthful. A fourth option — changing `deploy-plan`'s WIT
return to a per-service result list — is cleaner on the wire but touches the
component boundary for no behavior nothing else needs; recorded in §12 as the
requester's call.

### 0.4 (Scope-changing) One credential per invocation cannot address N substrates

`--as` and `--ucan` are **global** CLI flags, and
[`client_for`](../../../../apps/roymctl/src/commands.rs#L124) builds exactly one
client from them. Post-P0 every substrate fails closed without an owner-rooted
capability, so a deploy spanning substrates owned by different controllers — or
reached with different app-scoped grants — cannot work with one credential.

`task.md`'s "the deploy capability held on each" reads like inventory *metadata*.
It is not: it is the credential the client must actually present, and it is
load-bearing for §0.1's certificate too (the derivation includes the caller
DID). §3's inventory entry therefore carries `identity` and `ucan` per alias,
and §7 builds one client per alias from them.

**Also worth stating in the docs, since it is not obvious:** the credential for
each substrate needs `orchestrator/deploy` **and** `orchestrator/status` —
`resolve-instance-identity` is `ORCHESTRATOR_STATUS`-gated
([orchestration.rs:551](../../../../crates/control_plane/src/service/orchestration.rs#L551))
and `--mint-masters` calls it — **and** `orchestrator/undeploy`, because
`deploy`'s own failure path rolls back by calling `undeploy` with the same
caller (this is exactly `deferred-backlog.md`'s row targeted at "A3, when the
substrate inventory starts issuing grants").

### 0.5 (Scope-changing) Moving a service between substrates is a silent two-publisher state

Nothing in `task.md` covers editing `placement` and redeploying. The naive
behavior is: the service deploys onto the new substrate, and the instance on the
old substrate **keeps running and keeps replaying its endpoint record on every
heartbeat** — precisely the flap `deferred-backlog.md`'s *"a relocated-away
substrate keeps trying to publish a member's record"* row describes, and which
A1's compare-and-swap admission rule bounds but does not stop.

**Recommendation:** A3 detects it and **fails with an explicit error** naming
both substrates and telling the operator to undeploy from the old one first.
Emitting `Remove` + `Add` instead (a real relocation) is more work than it
looks — it needs the old substrate's client, an `undeploy` capability there, and
an ordering rule — and "no relocation" is already a milestone non-goal. Detect,
refuse, and record a backlog row targeted at A5.

**What "already deployed there" is read from matters** (D-A3-22): the
`COMPLETED` journal action rows, not the last `ACTIVE` plan. A partial deploy
rests at `Degraded` — and a *first* partial deploy leaves no `ACTIVE` record at
all — so an `ACTIVE`-sourced check is blind precisely when some services have
landed and the operator is most likely to be editing placement. The action rows
also carry `substrate_did` per service, which is what identifies where something
actually went; the stored plan JSON carries only the alias it was asked to go to.

### 0.6 (Scope-changing) No harness anywhere exercises the app-plan path

`roymctl app deploy` is the **only** `deploy_plan` caller in the tree — no
integration test, no e2e, no perf harness, no smoke test. A2's proof of
host-side resolution went through the router (`proxy_dispatch.rs`), not through
a compiled plan.

So A3's two-substrate coverage has nothing to extend: the harness has to be
built, modelled on
[master_endpoint_record_e2e.rs](../../../../crates/substrate/tests/master_endpoint_record_e2e.rs)'s
own `Node` (and *not* on `tests/common`'s `SubstrateTestContext`, which
deadlocks with two live nodes — that file's module doc says so). §9 sizes it.
This also drives §6's "move the per-substrate certification into `sdk`"
recommendation: `sdk` is a dev-dependency of `crates/substrate`, `roymctl` is a
binary and cannot be linked from a test.

### 0.7 (Ambiguous) "resolved `substrate` recorded on `PlannedService`" — the alias or the DID?

Both readings are defensible. Recording the **DID** means `compile()` needs the
inventory, and the journal then stores operator-environment data that changes
whenever the inventory is edited. Recording the **alias** keeps `compile()` pure
and inventory-free, and "resolved" still means something: the per-service
override and the manifest default have been folded into one value.

**Recommendation: the alias** (D-A3-4). The DID lands on the journal **action
row**, which is where `task.md` asks for per-(service, substrate) data anyway.

### 0.8 (Ambiguous) "reachability" in the inventory — a static address or a probed state?

A4 is *"health, read-only"* and owns polling. If A3's inventory stored a
reachability *state*, A3 and A4 would own the same field.

**Recommendation:** the inventory holds only how to *reach* a substrate
(`api_url`, defaulting to the invocation's `--api-url` — see §0.12 for why it is
*not* called `registry_url`), and A3 does a
**one-shot preflight** — `wait_for_ready` against every target before any deploy
call — so an unreachable substrate is a clean up-front error rather than a
partial application. No stored or polled reachability state in A3.

### 0.9 (Understated) "capabilities" cannot be probed — nothing reports them

The orchestrator interface exposes `deploy`, `undeploy`, `list`, `readyz`,
`ping`, `deploy-plan`, `resolve-instance-identity` and nothing else
([control-plane.wit](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit)).
Container support is a **compile-time Cargo feature**
(`control_plane/Cargo.toml`'s `podman_sandbox`), invisible on the wire.

**Recommendation:** v1 capabilities are **operator-declared** in the inventory
file and checked client-side at placement time; absent means "no constraint".
Declared data can drift from reality, and the failure mode when it does is a
late substrate-side deploy error — acceptable, and strictly better than today
(no check at all). Adding a capability list to a substrate status response is a
natural A4 item, noted in §13.

### 0.10 (Ambiguous) Aliases in the manifest couple an app to one operator's topology

`task.md` decides this ("by **alias**, never a bare DID"), so it is not
reopened here — but the consequence should be written down: a manifest carrying
`placement` is no longer portable to an operator whose aliases differ.

What keeps that from biting: `[placement]` **absent** means "the substrate you
deployed to", exactly today's behavior, so every existing manifest and every
single-substrate deploy is unaffected. This matches `task.md`'s own migration
note. No new CLI override flag is proposed (§12 asks whether one is wanted).

### 0.11 (Stale) The reconciler's diff is computed and thrown away

[app.rs:126](../../../../apps/roymctl/src/commands/app.rs#L126) is
`let _diff = reconciler.compute_diff(target_plan)?;` — the deploy sends the
whole plan regardless. So "reconcile" is nominal on the deploy path today.

A3 does **not** change this (D-A3-13): resume is driven by completed *action
rows*, not by the diff, and consuming the diff properly is A5's reconcile loop.
Recorded so nobody reads §4 and assumes the diff became load-bearing.

### 0.12 (Scope-changing) Cross-substrate resolution silently requires one registry namespace across the whole inventory

The largest thing neither `task.md` nor this plan's first draft said, and the
one that fails *after* a clean deploy rather than during it.

A substrate publishes and replays endpoint records through **its own
configured** registry client, built from `config.substrate.registry_url` /
`enable_bep0044_dht`
([runtime.rs:457-466](../../../../crates/substrate/src/runtime.rs#L457)), and
resolves inbound callers' master anchors through the same one
([handshake.rs:24-29](../../../../crates/router/src/handshake.rs#L24)). The
inventory's per-alias URL is something different: it is how **`roymctl`**
reaches that substrate. Nothing on the wire reports which registry a substrate
is configured with.

Three consequences, none of which surface at deploy time:

1. `backend`'s record lands in **substrate B's** registry. If substrate A is
   configured with a different one, `frontend`'s dependency call fails at
   resolution — after a deploy that reported success.
2. `refresh_anchor_or_warn` publishes master anchors to the single global
   `--registry-url`. Every substrate that *receives* a call from a member must
   resolve that member's anchor through its own registry, so an anchor in the
   wrong registry fails the handshake closed on a substrate that never saw it.
3. The two URLs have confusingly similar names. `--registry-url` (where anchors
   go) and a per-entry `registry_url` (how `roymctl` dials) are unrelated.

The existing two-node harness already solves this the only way available —
`shared_registry_url`, so node B points at node A's registry
([master_endpoint_record_e2e.rs:44-46](../../../../crates/substrate/tests/master_endpoint_record_e2e.rs#L44)).

**Recommendation, three parts:** (a) rename the inventory field to `api_url`,
matching the global `--api-url` flag it overrides, so the anchor URL is the only
thing called "registry" (D-A3-17); (b) state "one shared registry namespace, or
BEP0044 DHT enabled on every substrate" as a documented **precondition** of
multi-substrate placement; (c) add a post-apply **probe** — §7.2's final step
looks every member master up through every `api_url` in play and warns when a
record or anchor is missing or names the wrong substrate.

**Part (c) is a heuristic, and must be worded as one.** `roymctl` can only dial
the URLs it was given, and this section's whole premise is that those need not be
the registries the substrates publish into or resolve anchors through. So the
probe proves *"the registry at this URL cannot resolve member M"*, never
*"substrate X cannot"*. It is still worth having: in the ordinary layout each
substrate hosts the registry role `roymctl` dials, so the two coincide, and this
is the only signal available before a user-facing call fails. Its warnings
therefore name **the URL probed**, not a substrate. Turning it into a real
preflight refusal needs a substrate to report its own registry configuration —
§11's A4/A5 row, alongside §0.9's capability reporting.

### 0.13 (Stale) Spawned child plans are compiled and then discarded, so the placement cascade is unobservable

`compile` pushes child plans before the root
([compiler.rs:147](../../../../crates/app_orchestration/src/compiler.rs#L147)),
and `app deploy` takes `compiled.plans.last()`
([app.rs:114](../../../../apps/roymctl/src/commands/app.rs#L114)) — so a
`Spawn` dependency's whole plan is computed and thrown away. This predates A3
and is not A3's to fix (deploying child plans needs cross-plan binding wiring
that nothing has asked for yet).

**Consequence for D-A3-3:** the root-default cascade into spawned children is
correct but currently **unobservable** — no operator can reach it. Keeping it is
still right (three lines, and the alternative is leaving a wrong default in
place for whoever fixes `plans.last()`), but it is documented behavior plus two
unit tests, not a user-visible feature. Backlog row in §11.

### 0.14 (Understated) "keep retrying" is a manual re-run in A3

`task.md`'s A3 line says partial failure keeps retrying. There is no loop until
A5, so in A3 "retrying" means **the operator runs the same command again** and
the resume path skips what already landed (D-A3-10). Nothing retries on its own.

Worth stating rather than leaving in a backlog row, because it sets what
`Degraded` costs between A3 and A5: an app stays partially deployed until a
human notices. Confirmed as intended in §12.

### 0.15 (Coverage) Failure-matrix row 10 is not met by A3, and should not be claimed

Row 10 is *"deploy retried after a lost response → idempotent no-op for
identical (instance, service, content hash)"*. A3 builds the retry path but
cannot satisfy the row: a lost response leaves the action row `IN_PROGRESS`, not
`COMPLETED`, so the re-run re-sends; and `deploy_with_context` has no
content-hash dedup, so it redeploys — which overwrites correctly, but restarts
the service rather than being a no-op.

**Recommendation:** state plainly that row 10 is **A5's**, with a backlog row,
rather than letting A3's resume mechanism look like it covers it. A3 *does*
cover row 12 (partial deploy), which is the row its journal rows exist for.

---

## §1 — Decisions

| # | Decision |
|---|---|
| **D-A3-1** | Placement is declared in the manifest as `[placement] substrate = "<alias>"`, at the manifest level (the default) and optionally per service (the override). Serde shape is an **externally-tagged enum** `PlacementSelector`, so a later `pool = "..."` variant slots in without reshaping the schema (`deferred-backlog.md`'s *"placement is declared, not scheduled"* row asks for exactly this). Verified to round-trip through `#[serde(flatten)]` in both TOML and JSON. |
| **D-A3-2** | `SubstrateAlias` is a validated newtype: non-empty, no `/`, and **rejects anything starting with `did:`** — that is `task.md`'s "never a bare DID" made enforceable at parse time rather than by convention. |
| **D-A3-3** | The root manifest's default **cascades into spawned child apps** that declare none; a child's own `[placement]` overrides it. Rejected: no cascade, which would silently split one app across the CLI target and an alias for no reason the operator expressed. **Currently unobservable** — `app deploy` discards every plan but the last, so no child plan is ever deployed (§0.13). Kept anyway, as documented behavior with unit coverage, so the default is already right whenever `plans.last()` is fixed. |
| **D-A3-4** | `PlannedService.substrate: Option<SubstrateAlias>` holds the **alias**, after the default has been applied. `None` = "the substrate you deployed to". The **DID** is recorded on the journal action row instead (§0.7). |
| **D-A3-5** | The inventory is an operator-held TOML file, default `<roymctl --dir>/substrates.toml`, overridable with `--inventory`. It is read **only** when the plan actually places something by alias; a plan with no placement never needs the file to exist. |
| **D-A3-6** | An inventory entry's `identity` / `ucan` **override** the global `--as` / `--ucan` for that substrate; the globals remain the fallback target's credential. A single global credential cannot be right for N independently-owned substrates (§0.4). |
| **D-A3-7** | Capabilities are operator-declared and checked client-side before any deploy; absent = unconstrained (§0.9). |
| **D-A3-8** | Reachability is a **one-shot preflight** (`wait_for_ready` per target) before the first deploy call, not stored state (§0.8). |
| **D-A3-9** | One `deploy-plan` call per (service, substrate), each carrying a single-service plan (§0.3 option (c)). No WIT change. |
| **D-A3-10** | Partial failure sets the deployment record to a new `DeploymentState::Degraded`, prints every failure, and exits **non-zero**. No rollback, no undeploy of what succeeded. A re-run resumes: any (service, substrate) with a `COMPLETED` action row for the same deployment record is skipped. **"Retrying" is that manual re-run** — nothing retries on its own until A5 (§0.14). Failure-matrix row 10 (lost-response dedup on a content hash) is **not** claimed by A3 (§0.15). |
| **D-A3-11** | The resume key is `(action_type, logical_ref, substrate_did)` — the **DID**, not the alias, so re-pointing an alias at a different node correctly forces a redeploy. `recover_applying`, which has no inventory, compares the alias instead; it is a diagnostic for `app reconcile`, not the resume path. |
| **D-A3-12** | A redeploy whose placement moved a service to a different substrate is a **hard error** naming both substrates (§0.5). |
| **D-A3-13** | The reconciler's diff stays unused on the deploy path (§0.11). |
| **D-A3-14** | The apply loop goes behind a one-method `PlanApplier` trait in `crates/sdk`. This is ADR-0021 §5's *"narrow 'apply this action to that substrate' trait"* arriving one slice early — justified because partial-failure behavior is otherwise untestable without two live substrates mid-test, and because A5 then replaces a body rather than a call graph. |
| **D-A3-15** | The journal's `PRAGMA user_version` ladder is replaced with unconditional `CREATE TABLE IF NOT EXISTS`, matching the precedent and reasoning already written into [`registry_store.rs:53-61`](../../../../crates/data_db/src/registry_store.rs#L53) ("pre-release: no compat shims, no version ladders"). **Consequence:** an existing `deployments.db` keeps the old `deployment_actions` shape and its inserts will fail; it must be deleted. Documented, not migrated. |
| **D-A3-16** | Per-substrate certificate and endpoint-record construction moves from `roymctl` into `crates/sdk` (§6), so the two-substrate e2e can exercise it. Member-master **file** resolution (`<dir>/identities/member-*.key`) stays in `roymctl` — that is a CLI storage convention, not an SDK concern. |
| **D-A3-17** | The inventory's reachability field is named **`api_url`**, matching the global `--api-url` it overrides, leaving `--registry-url` (anchors) as the only thing called "registry" (§0.12). One shared registry namespace across the inventory — or BEP0044 DHT on every substrate — is a documented **precondition**, and §7.2's last step probes it after apply and **warns** on a miss. The probe is a **heuristic, and says so** — it can only dial the URLs `roymctl` knows, which need not be the registries the substrates themselves publish into (§0.12's own premise); it is right in the ordinary layout where each substrate hosts the registry role `roymctl` dials, and its warnings name the URL actually probed rather than claiming to speak for a substrate. A warning, not `Degraded`: the deploy genuinely landed, what is wrong is the operator's registry topology, and `Degraded` would send the resume path redeploying services that are already fine. The non-heuristic fix needs a substrate to report its own registry config — §11's A4/A5 row. |
| **D-A3-18** | `recover_applying` accepts `Degraded` as well as `Applying`, and `app reconcile`'s "no ACTIVE or APPLYING state" message is reworded. Without this the recovery surface goes blind on exactly the state A3 introduces. |
| **D-A3-19** | The journal record is created **after** every check that can bail (the placement-change refusal and the inventory preflight). A record written before a refusal is a phantom: it becomes the next run's resume target and a fake recovery plan for `app reconcile`. |
| **D-A3-20** | The fallback client is built **lazily** — only when some `PlannedService.substrate` is `None`. A fully-placed app must not require a default substrate it never touches, and `get_substrate_did` fails outright when there is no `--substrate` and no `substrate.key` ([commands.rs:98-103](../../../../apps/roymctl/src/commands.rs#L98)). Consequence: `Commands::App` passes the raw `Option<String>` through and the deploy handler resolves it on demand (§7.3). |
| **D-A3-21** | The module is `substrate_inventory.rs`, not `inventory.rs`: this crate already has `StaticInventory` ([resolver.rs:261](../../../../crates/app_orchestration/src/resolver.rs#L261)), the logical-name → member-set registry, and two unrelated "inventories" in one crate is a naming trap. |
| **D-A3-22** | D-A3-12's refusal is sourced from **`COMPLETED` action rows across every record for the instance**, compared on `substrate_did`, and runs **after** the preflight (which is what resolves those DIDs). Not from the last `ACTIVE` plan: `Degraded` is the resting state of a partial deploy and a first partial deploy leaves no `ACTIVE` record at all, so an ACTIVE-only source is blind in exactly the sequence D-A3-10 creates — and the services that *did* land are still running. Needs one new journal query, `get_completed_actions_for_instance` (§4.3). |

---

## §2 — Phase 1: placement in the manifest and the compiler

Pure `app_orchestration` work. No I/O, no client, independently mergeable.

### 2.1 `crates/app_orchestration/src/models.rs` — new types

Add after the `DependencyName` wrapper (line 122):

```rust
define_string_wrapper!(
    SubstrateAlias,
    "Operator-chosen name for a substrate in the deploy inventory.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("SubstrateAlias cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("SubstrateAlias cannot contain '/'"));
        }
        // Placement names an inventory alias, never a bare DID: an alias is
        // the indirection that lets one manifest deploy against different
        // operators' topologies, and a DID written here would defeat it.
        if s.starts_with("did:") {
            return Err(anyhow!(
                "SubstrateAlias '{s}' looks like a DID; placement names an inventory alias"
            ));
        }
        Ok(())
    }
);
```

Add near `TopologyMode`:

```rust
/// How a service's hosting substrate is chosen.
///
/// One variant today. It is an enum rather than a bare alias so a later
/// pool- or constraint-based selector is an added variant instead of a
/// schema change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementSelector {
    /// Place on the substrate registered in the deploy inventory under this
    /// alias.
    Substrate(SubstrateAlias),
}

impl PlacementSelector {
    pub fn alias(&self) -> &SubstrateAlias {
        match self {
            Self::Substrate(alias) => alias,
        }
    }
}
```

### 2.2 `models.rs` — three struct fields

```rust
pub struct ServiceSpec {
    #[serde(flatten)]
    pub config: ServiceConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<LogicalServiceName>,
    /// Overrides the manifest-level default for this service only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSelector>,
}

pub struct SynAppManifest {
    pub id: AppBlueprintId,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default for every service this manifest declares, and for every
    /// spawned child manifest that declares none of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSelector>,
    #[serde(default)]
    pub services: BTreeMap<LogicalServiceName, ServiceSpec>,
    #[serde(default)]
    pub dependencies: BTreeMap<DependencyName, AppDependencySpec>,
}

pub struct PlannedService {
    pub service_id: ServiceId,
    pub logical_ref: LogicalServiceRef,
    /// The substrate this service is placed on, after the manifest default
    /// and any per-service override have been folded together. `None` means
    /// the substrate the deploy was aimed at, which is what every manifest
    /// written before placement existed still means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substrate: Option<SubstrateAlias>,
    #[serde(flatten)]
    pub config: ServiceConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_dependencies: BTreeMap<LogicalServiceName, Vec<ServiceId>>,
    #[serde(default)]
    pub topology_mode: TopologyMode,
}
```

`SynAppManifest::validate()` needs **no** change: alias well-formedness is
enforced by the newtype at parse time, and alias *existence* cannot be checked
without the inventory, which the manifest layer deliberately does not know
about.

### 2.3 `crates/app_orchestration/src/compiler.rs` — cascade and record

`compile`'s public signature is **unchanged**. `compile_recursive` gains one
parameter:

```rust
fn compile_recursive<'a>(
    instance_id: &'a AppInstanceId,
    manifest: &'a SynAppManifest,
    catalog: &'a dyn ManifestCatalog,
    inherited_placement: Option<&'a PlacementSelector>,
    blueprint_stack: &'a mut Vec<AppBlueprintId>,
    compilation_stack: &'a mut Vec<AppInstanceId>,
    plans: &'a mut Vec<DeploymentPlan>,
) -> Pin<Box<dyn Future<Output = Result<()>> + 'a + Send>>
```

`compile` passes `None`. Inside the body, immediately after the cycle checks:

```rust
// D-A3-3: this manifest's own default wins; otherwise the root's cascades in.
let default_placement = manifest.placement.as_ref().or(inherited_placement);
```

The `Spawn` arm passes `default_placement` down. The per-service loop sets:

```rust
services.push(PlannedService {
    service_id,
    logical_ref,
    substrate: spec
        .placement
        .as_ref()
        .or(default_placement)
        .map(|p| p.alias().clone()),
    config: spec.config.clone(),
    resolved_dependencies,
    topology_mode: TopologyMode::default(),
});
```

Borrow note: `default_placement` borrows `manifest`, which lives for `'a`;
`inherited_placement` is already `'a`. No lifetime work beyond adding the
parameter.

---

## §3 — Phase 2: the substrate inventory

New file `crates/app_orchestration/src/substrate_inventory.rs` (D-A3-21). Pure
parsing and lookup — no client, no network — so it is unit-testable and reusable
by A5's supervisor without dragging in the SDK.

### 3.1 File format

```toml
# <roymctl --dir>/substrates.toml
#
# Precondition (§0.12): every substrate listed here must publish and resolve
# endpoint records in the SAME registry namespace -- one shared HTTP registry,
# or BEP0044 DHT enabled on all of them. A substrate publishes through its own
# configured registry, not through the `api_url` below, and nothing on the wire
# reports which one that is, so this cannot be checked before deploying. It is
# verified after (§7.2's last step).
[substrates.edge-1]
did = "did:key:z6MkExampleNodeA"
# Optional. How `roymctl` reaches this substrate. Overrides the global
# --api-url. NOT the registry this substrate publishes into, and unrelated to
# --registry-url, which is where master anchors go.
api_url = "http://localhost:7961"
# Optional. Local identity to act as against this substrate; overrides --as.
identity = "operator"
# Optional. Signed CapabilityToken JSON; overrides --ucan. Requires `identity`.
ucan = "grants/edge-1.json"
# Optional. Absent means "no constraint".
capabilities = ["wasm", "tcp"]

[substrates.edge-2]
did = "did:key:z6MkExampleNodeB"
identity = "operator"
capabilities = ["wasm", "container"]
```

`ucan` is resolved relative to `<roymctl --dir>` when relative, matching how
`client_for` already resolves `identities/<name>.key`.

### 3.2 Types

```rust
/// Operator-declared map of substrate aliases to the substrates a manifest's
/// placement selectors may name.
///
/// Deliberately not discoverable: nothing on the wire reports a substrate's
/// DID, address, or what it can run, so this is the operator's own record of
/// the fleet they hold deploy capability on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubstrateInventory {
    #[serde(default)]
    pub substrates: BTreeMap<SubstrateAlias, SubstrateEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubstrateEntry {
    pub did: String,
    /// How the *client* reaches this substrate; overrides the global
    /// `--api-url`. Deliberately not called `registry_url`: the registry a
    /// substrate publishes endpoint records into is its own configured one,
    /// which nothing here can see or set (§0.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Local identity to present against this substrate. Overrides `--as`
    /// (D-A3-6): one global credential cannot be right for N substrates
    /// owned by different controllers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ucan: Option<PathBuf>,
    /// Service types this substrate can run. Operator-declared, because
    /// nothing reports it: container support is a compile-time Cargo
    /// feature, invisible over the wire. Absent = unconstrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<BTreeSet<ServiceType>>,
}
```

### 3.3 Functions

```rust
impl SubstrateInventory {
    /// Parses and validates. Every `did` must be a `did:key:` -- an alias
    /// pointing at a malformed DID fails here rather than as an opaque
    /// registry lookup miss at deploy.
    pub fn from_toml(s: &str) -> Result<Self>;

    /// Reads and parses, with the path in every error message.
    pub fn load(path: &Path) -> Result<Self>;

    /// Looks up one alias, or errors naming the missing alias, the file it
    /// was looked for in, and every alias the file does define.
    pub fn get(&self, alias: &SubstrateAlias, source: &Path) -> Result<&SubstrateEntry>;
}

/// Every alias the plan places a service on, with the service types placed
/// there. The whole input to the deploy-time preflight: unknown aliases and
/// capability mismatches are both decided from this before any deploy call
/// is made.
pub fn placement_demand(
    plan: &DeploymentPlan,
) -> BTreeMap<SubstrateAlias, BTreeSet<ServiceType>>;

/// Checks every alias in `demand` against the inventory. Returns *all*
/// problems, not the first -- an operator fixing a five-substrate inventory
/// should not have to re-run five times.
pub fn check_placement(
    inventory: &SubstrateInventory,
    demand: &BTreeMap<SubstrateAlias, BTreeSet<ServiceType>>,
    source: &Path,
) -> Result<()>;
```

`check_placement` pseudo-code:

```
problems = []
for (alias, types) in demand:
    entry = inventory.substrates.get(alias)
    if entry is None:
        problems.push("no substrate '<alias>' in <source> (known: a, b, c)")
        continue
    if let Some(caps) = &entry.capabilities:
        for t in types not in caps:
            problems.push("substrate '<alias>' does not declare '<t>' (declares: ...)")
if problems non-empty: bail with all of them, one per line
```

### 3.4 `crates/app_orchestration/src/lib.rs`

```rust
pub mod substrate_inventory;

pub use models::{
    /* ...existing... */ PlacementSelector, SubstrateAlias,
};
pub use substrate_inventory::{
    SubstrateEntry, SubstrateInventory, check_placement, placement_demand,
};
```

---

## §4 — Phase 3: journal action records and `Degraded`

All in `crates/app_orchestration/src/journal.rs`, plus one caller in
`reconcile.rs`.

### 4.1 `DeploymentState`

```rust
pub enum DeploymentState {
    Planned,
    Applying,
    Active,
    /// Some services applied and some did not. No rollback (ADR-0021 §5 /
    /// task.md non-goals): rolling back a stateful service is itself
    /// destructive, so the deployment stays here until a re-run completes
    /// the missing actions.
    Degraded,
    RollingBack,
    RolledBack,
}
```

`Display` → `"DEGRADED"`; `FromStr` → the matching arm. No other exhaustive
match on this enum exists in the tree (verified).

### 4.2 Schema

Replace the whole `init_schema` version ladder with unconditional creation
(D-A3-15), carrying the same explanatory comment shape as
[`registry_store.rs:53-61`](../../../../crates/data_db/src/registry_store.rs#L53):

```rust
fn init_schema(conn: &Connection) -> Result<()> {
    // Unconditional, not gated on `PRAGMA user_version`: pre-release, schema
    // changes are made in place with no version ladder, and `IF NOT EXISTS`
    // is already idempotent.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS deployments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            instance_id TEXT NOT NULL,
            plan_json TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_deployments_instance_id ON deployments(instance_id);
         CREATE TABLE IF NOT EXISTS deployment_actions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            deployment_id INTEGER NOT NULL,
            action_type TEXT NOT NULL,
            logical_ref TEXT NOT NULL,
            substrate_alias TEXT,
            substrate_did TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY(deployment_id) REFERENCES deployments(id)
         );
         CREATE INDEX IF NOT EXISTS idx_deployment_actions_dep_id
            ON deployment_actions(deployment_id);",
    )?;
    Ok(())
}
```

`substrate_alias` is nullable (a service with no placement); `substrate_did` is
`NOT NULL` (a deploy always went *somewhere*).

### 4.3 Action APIs

```rust
/// One (service, substrate) unit of work inside a deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRecord {
    pub action_type: String,
    pub logical_ref: String,
    pub substrate_alias: Option<String>,
    pub substrate_did: String,
}

pub fn append_action(
    &self,
    deployment_id: i64,
    action_type: &str,
    logical_ref: &str,
    substrate_alias: Option<&str>,
    substrate_did: &str,
    state: ActionState,
) -> Result<i64>;

pub fn get_completed_actions(&self, deployment_id: i64) -> Result<Vec<ActionRecord>>;

/// Every completed action for an app instance, across **all** its deployment
/// records, oldest first.
///
/// The per-record query above answers "what does this run still owe?"; this
/// one answers "where has this service actually landed, ever?", which is a
/// different question and spans records: a plan edit starts a new record, so
/// the run that placed a service can be two records back. The
/// placement-change refusal (D-A3-22) is its only caller.
pub fn get_completed_actions_for_instance(
    &self,
    instance_id: &AppInstanceId,
) -> Result<Vec<ActionRecord>>;
```

```sql
SELECT a.action_type, a.logical_ref, a.substrate_alias, a.substrate_did
  FROM deployment_actions a
  JOIN deployments d ON d.id = a.deployment_id
 WHERE d.instance_id = ?1 AND a.state = 'COMPLETED'
 ORDER BY a.id ASC
```

Oldest-first ordering is what lets the caller take the **last** match per
service with `rfind`, so a service deployed, undeployed, and redeployed
elsewhere reports its current home rather than its first one.

`update_action_state` is unchanged. `ActionState::Pending` gains no writer in
A3 — the executor writes `InProgress` directly, since nothing enqueues work
ahead of applying it until A5's loop exists.

### 4.4 `reconcile.rs` — the state gate, then the filter that finally filters

**The state gate first (D-A3-18).**
[reconcile.rs:55](../../../../crates/app_orchestration/src/reconcile.rs#L55)
is `record.state == DeploymentState::Applying`. `Degraded` is the resting state
of every partially-failed deploy, so leaving this unchanged makes
`roymctl app reconcile` answer *"No ACTIVE or APPLYING state found"* for exactly
the deployments an operator would run it on:

```rust
if let Some(record) = latest
    && matches!(record.state, DeploymentState::Applying | DeploymentState::Degraded)
{
```

The method keeps the name `recover_applying` — it recovers an *unfinished*
deployment, and `Degraded` is unfinished. Renaming it would churn A5's callers
for nothing.

**Then the filter.** `recover_applying`'s `retain` closure updates to the new
record type and to `PlannedService.substrate` (alias, per D-A3-11 — this path has
no inventory):

```rust
actions.retain(|a| {
    let (a_type, l_ref, alias) = match a {
        ReconcileAction::Add(svc) => ("ADD", &svc.logical_ref, svc.substrate.as_ref()),
        ReconcileAction::Remove(r) => ("REMOVE", r, None),
        ReconcileAction::Update { new, .. } => ("UPDATE", &new.logical_ref, new.substrate.as_ref()),
    };
    let l_str = l_ref.to_string();
    !completed.iter().any(|c| {
        c.action_type == a_type
            && c.logical_ref == l_str
            && c.substrate_alias.as_deref() == alias.map(SubstrateAlias::as_str)
    })
});
```

`ReconcileAction::Remove` carries only a `LogicalServiceRef` and so has no
alias; that stays true in A3 and is why the comparison must tolerate `None` on
both sides rather than requiring a match.

---

## §5 — Phase 4: the mapper and the apply loop

### 5.1 `crates/sdk/src/mapper.rs` — map a subset, compute modes from the whole plan

```rust
/// Maps exactly the services in `services`, while computing every
/// dependency's topology mode from the **whole** `plan`.
///
/// The split matters: a dependency's `mode` belongs to the dependency, which
/// may be placed on a different substrate and therefore absent from
/// `services`. Deriving modes from the subset would silently default every
/// cross-substrate dependency to `Singleton`.
pub fn map_deployment_plan_to_wit(
    plan: &DeploymentPlan,
    services: &[&PlannedService],
    instance_certificates: &BTreeMap<ServiceId, String>,
    registry_certificates: &BTreeMap<ServiceId, String>,
    emit_bindings: bool,
) -> anyhow::Result<WitDeploymentPlan>
```

Note the **two** changes in that signature: a new `services` argument, and
`plan` going from by-value to a borrow. §8.3 lists all **13** call sites, each of
which needs both.

Body changes:

- `target_modes` is built from `plan.services` (unchanged logic, now always the
  full set).
- `for svc in plan.services` becomes `for svc in services`, and `svc` is now
  `&PlannedService`, so the moves inside become clones:
  `svc.config.env.clone().into_iter().collect()`, `svc.config.args.clone()`,
  `svc.config.source.clone()` where it was moved. Deploy is a cold path; the
  clones are not worth avoiding with a second code path.
- `plan_instance_id` comes from `plan.app_instance_id.to_string()` as before.
- `PlannedService.substrate` is **not** mapped onto the wire. A substrate has
  no use for the placement of services it is not hosting, and publishing it
  would hand every node a partial topology map of the app for nothing — the
  opposite of ADR-0021's least-privilege consequence.

**This is a latent bug fix, not only a refactor:** `TopologyMode` is always
`Singleton` today, so the wrong-default path is currently invisible. §9 pins it
with a test that would fail under the naive "filter the plan, then map" shape.

### 5.2 `crates/sdk/src/deploy.rs` — new module

```rust
/// Applying one deployment call to one substrate.
///
/// ADR-0021 §5's narrow "apply this action to that substrate" boundary,
/// introduced here rather than at A5 for two reasons: partial-failure
/// behavior is otherwise not testable without killing a live substrate
/// mid-test, and A5's durable outbox implementation then replaces this
/// trait's body instead of restructuring its callers.
#[async_trait::async_trait]
pub trait PlanApplier: Send + Sync + fmt::Debug {
    async fn apply(&self, plan: WitDeploymentPlan) -> Result<(), String>;
}

#[async_trait::async_trait]
impl PlanApplier for SyneroymClient {
    async fn apply(&self, plan: WitDeploymentPlan) -> Result<(), String> {
        self.deploy_plan(plan).await.map_err(|e| e.to_string())
    }
}

/// A connected deploy target: the alias it was named by (`None` for the
/// invocation's own `--substrate`), the substrate's DID, and the applier.
#[derive(Debug, Clone)]
pub struct DeployTarget {
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub applier: Arc<dyn PlanApplier>,
}

#[derive(Debug)]
pub struct ApplyRequest<'a> {
    pub plan: &'a DeploymentPlan,
    pub targets: &'a BTreeMap<SubstrateAlias, DeployTarget>,
    /// The invocation's own `--substrate`, for services with no placement.
    /// `None` when every service is placed by alias -- a fully-placed app
    /// must not require a default substrate it never touches (D-A3-20).
    pub fallback: Option<&'a DeployTarget>,
    pub instance_certificates: &'a BTreeMap<ServiceId, String>,
    pub registry_certificates: &'a BTreeMap<ServiceId, String>,
    pub emit_bindings: bool,
}

#[derive(Debug, Clone)]
pub struct ServiceFailure {
    pub logical_ref: LogicalServiceRef,
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub deployed: Vec<LogicalServiceRef>,
    pub skipped: Vec<LogicalServiceRef>,
    pub failures: Vec<ServiceFailure>,
}

impl ApplyReport {
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Pairs every service with the target it is placed on, in the plan's own
/// topological order. Fails closed on any alias the caller did not build a
/// target for, before a single deploy call is made -- an unknown alias must
/// never produce a half-applied app.
pub fn resolve_targets<'a>(
    plan: &'a DeploymentPlan,
    targets: &'a BTreeMap<SubstrateAlias, DeployTarget>,
    fallback: Option<&'a DeployTarget>,
) -> anyhow::Result<Vec<(&'a PlannedService, &'a DeployTarget)>>;

pub async fn apply_plan(
    req: ApplyRequest<'_>,
    journal: &DeploymentJournal,
    deployment_id: i64,
) -> anyhow::Result<ApplyReport>;
```

`apply_plan` pseudo-code:

```
placed   = resolve_targets(req.plan, req.targets, req.fallback)?   // fails closed
completed = journal.get_completed_actions(deployment_id)?
report    = ApplyReport::default()

for (svc, target) in placed:
    l_ref = svc.logical_ref.to_string()

    # D-A3-11: keyed on the DID, so an alias re-pointed at a different node
    # correctly redeploys rather than being skipped as already done.
    if completed.any(c => c.action_type == "ADD"
                       && c.logical_ref  == l_ref
                       && c.substrate_did == target.substrate_did):
        report.skipped.push(svc.logical_ref.clone())
        continue

    action_id = journal.append_action(
        deployment_id, "ADD", &l_ref,
        target.alias.as_ref().map(SubstrateAlias::as_str),
        &target.substrate_did,
        ActionState::InProgress)?

    # A mapping error (an unreadable WASM artifact, an oversized document) is
    # this one service's failure, not the whole app's: the same
    # no-rollback-keep-going rule a substrate-side deploy failure gets.
    outcome = match map_deployment_plan_to_wit(
                  req.plan, &[svc],
                  req.instance_certificates, req.registry_certificates,
                  req.emit_bindings) {
        Err(e)  => Err(e.to_string()),
        Ok(wit) => target.applier.apply(wit).await,
    }

    match outcome:
        Ok(())  => journal.update_action_state(action_id, Completed)?;
                   report.deployed.push(svc.logical_ref.clone())
        Err(e)  => journal.update_action_state(action_id, Failed)?;
                   report.failures.push(ServiceFailure { .., error: e })

Ok(report)
```

`resolve_targets` pseudo-code:

```
missing = []
out     = []
for svc in &plan.services:                 # already topologically sorted
    match (&svc.substrate, fallback):
        (None, Some(f))     => out.push((svc, f))
        # Unreachable from the CLI, which builds the fallback exactly when some
        # service needs it -- but the type admits it, so it errors rather than
        # unwraps. A5's supervisor is the caller that could get this wrong.
        (None, None)        => bail "service '<ref>' has no placement and no
                                     default substrate was supplied"
        (Some(alias), _)    => match targets.get(alias):
            Some(t) => out.push((svc, t))
            None    => missing.push((svc.logical_ref, alias))
if missing non-empty: bail naming every (service, alias) pair
Ok(out)
```

Iterating `plan.services` (not grouping by substrate) preserves the plan's
topological order across substrates. Correctness does not depend on it —
bindings are resolved per call, not at deploy — but a dependency deploying
before its dependent is what every existing single-substrate deploy already
does, and there is no reason to stop.

### 5.3 `crates/sdk/src/lib.rs`

```rust
pub mod deploy;
```

plus re-exports of `ApplyReport`, `ApplyRequest`, `DeployTarget`, `PlanApplier`,
`ServiceFailure`, `apply_plan`, `resolve_targets` alongside the existing
`pub use`s.

---

## §6 — Phase 5: per-substrate member identity (§0.1's fix)

### 6.1 What moves into `crates/sdk/src/deploy.rs`

Two functions move verbatim-in-behavior from
`apps/roymctl/src/commands/member_identity.rs`:

```rust
/// Queries `client`'s substrate for the instance key it would derive for
/// `service_id` under the connecting caller's identity, and issues a
/// `service-instance`-scoped certificate over it from `master`.
///
/// **Bound to this client, not just this master.** The substrate derives the
/// key from its own node identity *and* the calling DID, so a certificate
/// minted through one client is rejected at deploy by any other substrate,
/// and by the same substrate reached as a different caller.
pub async fn certify_instance(
    client: &SyneroymClient,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate>;

/// Mints, per placed member, the instance certificate its hosting substrate
/// will accept and the endpoint record that points at that substrate.
///
/// Returns `(instance_certificates, registry_certificates)`, both keyed by
/// the member master `ServiceId`, ready for `ApplyRequest`.
pub async fn certify_placed_members(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    /// `None` when every service is placed by alias (D-A3-20).
    fallback: Option<&Arc<SyneroymClient>>,
    expires_hours: u64,
) -> Result<(BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>)>;
```

`certify_placed_members` pseudo-code:

```
# Two services sharing one master would need two endpoint records under one
# service_id pointing at different substrates -- a permanent compare-and-swap
# fight at the registry. Impossible from today's compiler (one master per
# PlannedService), so this is an assertion, not a supported case.
seen = {}
for svc in &plan.services:
    if seen.insert(svc.service_id, svc.logical_ref).is_some():
        bail "member master <did> is placed twice (<ref-a>, <ref-b>)"

certs   = {}
records = {}
for svc in &plan.services:
    master = masters.get(&svc.service_id)?          # else bail naming the service
    client = match (&svc.substrate, fallback) {
        (Some(alias), _) => clients.get(alias)?     # else bail naming the alias
        (None, Some(f))  => f
        (None, None)     => bail "service '<ref>' has no placement and no
                                  default substrate was supplied"
    }

    cert = certify_instance(client, master, svc.service_id.as_str(), expires_hours).await?
    certs.insert(svc.service_id.clone(), cert.to_json()?)

    record = EndpointInfo {
        service_id:    svc.service_id.to_string(),
        substrate_id:  client.service_id().to_string(),   # <-- the hosting node
        endpoint_type: EndpointType::Service,
        mechanisms:    vec![],
        nickname:      None,
        is_private:    false,
        ttl:           None,
        not_after:     now + DEFAULT_ENDPOINT_NOT_AFTER_SECS,
    }.sign(master)?
    records.insert(svc.service_id.clone(), serde_json::to_string(&record)?)

Ok((certs, records))
```

### 6.2 What stays in `apps/roymctl/src/commands/member_identity.rs`

`member_master_name`, `resolve_member_master`, `resolve_or_mint_member_master`,
and `refresh_anchor_or_warn` are unchanged. `substitute_and_certify_members`
becomes a thin composition:

```rust
pub async fn substitute_and_certify_members(
    dir: &Path,
    plan: &DeploymentPlan,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    /// `None` when every service is placed by alias (D-A3-20).
    fallback: Option<&Arc<SyneroymClient>>,
    registry_url: Option<&str>,
) -> Result<(DeploymentPlan, BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>)>
```

```
# 1-2: unchanged -- resolve-or-mint one master per service, build `new_plan`
#      with every service_id and resolved_dependencies entry substituted.
#      `masters` is keyed by the master's ServiceId, as today.
# 3: NEW -- the per-substrate half
(certs, records) = deploy::certify_placed_members(
    &new_plan, &masters, clients, fallback, DEFAULT_INSTANCE_CERT_EXPIRES_HOURS).await?
# 4: anchors, once per master (unchanged intent, moved after certification)
for (master_did, master) in &masters:
    refresh_anchor_or_warn(registry_url, master).await?
Ok((new_plan, certs, records))
```

**Anchors stay on the single global `--registry-url`, and that is a
precondition, not an oversight (§0.12).** Every substrate that *receives* a call
from a member resolves that member's master anchor through its **own** configured
registry ([handshake.rs:24-29](../../../../crates/router/src/handshake.rs#L24)),
so an anchor published to one registry in a split-registry fleet fails the
handshake closed on every substrate that cannot see it. Publishing to N
registries is not the fix — `roymctl` knows only the URLs *it* dials, which need
not be the ones the substrates use. So: one shared namespace is required, and
`refresh_anchor_or_warn`'s existing no-`--registry-url` warning gains a second
sentence saying the anchor must live in the registry **every** substrate in the
inventory resolves through. §7.2's post-apply step is what catches a violation.

The `client: &SyneroymClient` first parameter is **gone** — that is the whole
point of §0.1. Every caller must supply the alias-keyed client map instead.

### 6.3 `apps/roymctl/src/commands/identity.rs` — `certify-instance`

`IdentityCommands::CertifyInstance`'s arm calls
`member_identity::certify_instance(...)`; that function now lives in the SDK, so
the call becomes `syneroym_sdk::deploy::certify_instance(...)`. Behavior is
unchanged; this is an import move. (Per AGENTS.md's import rule: `use
syneroym_sdk::deploy;` and call `deploy::certify_instance(...)`, not an inline
fully-qualified path.)

---

## §7 — Phase 6: `roymctl app deploy`

### 7.1 New flag

```rust
AppCommands::Deploy {
    /* ...existing fields... */
    /// Substrate inventory mapping the aliases a manifest's `placement`
    /// selectors name to DIDs, addresses, credentials, and declared
    /// capabilities. Defaults to `<dir>/substrates.toml`. Only read when
    /// the plan actually places a service by alias.
    #[arg(long)]
    inventory: Option<PathBuf>,
}
```

### 7.2 Handler flow

```
instance_id = AppInstanceId::try_new(...)?
manifest    = (legacy .wasm wrapper | from_toml)          # unchanged
catalog     = LocalFilesystemCatalog::new(manifest dir)   # unchanged
compiled    = compile(instance_id, &manifest, &catalog).await?   # now fills `substrate`
target_plan = compiled.plans.last() else bail             # unchanged

journal = DeploymentJournal::open(parent_dir, db_name)?

# ======================================================================
# Everything that can bail runs BEFORE the journal is written (D-A3-19).
# A record created ahead of a refusal is a phantom: it becomes the next
# run's resume target and a fake recovery plan for `app reconcile`.
# ======================================================================

# --- inventory + preflight (D-A3-5/7/8) --------------------------------
# Runs before the placement-change refusal, because the refusal needs the
# resolved substrate DIDs the preflight produces (D-A3-22).
demand = placement_demand(target_plan)
clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> = {}
if !demand.is_empty():
    inv_path = inventory.unwrap_or(dir.join("substrates.toml"))
    inv      = SubstrateInventory::load(&inv_path)?
    check_placement(&inv, &demand, &inv_path)?            # all problems at once
    for alias in demand.keys():
        entry = inv.get(alias, &inv_path)?
        # D-A3-6: the entry's credential wins over the global flags.
        # `mut`: `wait_for_ready` takes `&mut self`, so every client is
        # connected *before* it is wrapped in the `Arc` the applier needs.
        let mut c = client_for(
                entry.did.clone(),
                entry.api_url.as_deref().unwrap_or(api_url),
                dir,
                entry.identity.as_deref().or(run_as),
                entry.ucan.as_deref().map(|p| resolve_under(dir, p)).or(ucan_path))?
        c.wait_for_ready(PREFLIGHT_TIMEOUT).await
            .with_context(|| "substrate '<alias>' (<did>) is not reachable")?
        clients.insert(alias.clone(), Arc::new(c))

# --- the fallback target, built lazily (D-A3-20) -----------------------
# Only a service with no placement needs it. A fully-placed app must not
# require a `--substrate`/`substrate.key` it never touches, and
# `get_substrate_did` hard-fails when neither exists.
fallback = if target_plan.services.iter().any(|s| s.substrate.is_none()) {
    let did = super::get_substrate_did(substrate_opt, dir)?;   # §7.3
    let mut fb = client_for(did, api_url, dir, run_as, ucan_path)?;
    fb.wait_for_ready(PREFLIGHT_TIMEOUT).await?;
    let fb = Arc::new(fb);
    Some((fb.clone(), DeployTarget {
        alias: None, substrate_did: fb.service_id().to_string(),
        applier: fb.clone() }))
} else { None }

# Both halves are borrowed from the one `Option` above, so §6's certification
# and §5's apply agree on whether a default substrate exists at all.
fallback_client = fallback.as_ref().map(|(c, _)| c)          # Option<&Arc<SyneroymClient>>
fallback_target = fallback.as_ref().map(|(_, t)| t)          # Option<&DeployTarget>

targets = clients.map(|(alias, c)| (alias, DeployTarget {
    alias: Some(alias), substrate_did: c.service_id().to_string(), applier: c.clone() }))

# --- placement change refusal (D-A3-12, sourced per D-A3-22) -----------
# Keyed on COMPLETED action rows across EVERY record for this instance, not on
# the last ACTIVE plan. A partially-failed deploy leaves the record `Degraded`
# (or leaves no ACTIVE record at all, on a first deploy), while the services
# that did land are still running -- so an ACTIVE-only source misses exactly
# the sequence A3 introduces. Action rows also carry the substrate DID per
# service, which is what actually identifies where something landed.
# `resolve_targets` is pure and gets called again inside `apply_plan`; calling it
# twice is deliberate, not a missed hand-off. Threading the result through would
# tie the refusal to the substituted plan, and the refusal must run before
# member substitution -- it compares logical refs and placement, neither of
# which substitution touches.
placed = resolve_targets(target_plan, &targets, fallback_target)?
landed = journal.get_completed_actions_for_instance(&instance_id)?   # §4.3
for (svc, target) in &placed:
    l_ref = svc.logical_ref.to_string()
    # Most recent COMPLETED row for this service, whichever record it came from.
    if let Some(prev) = landed.iter().rfind(|r| r.action_type == "ADD"
                                             && r.logical_ref == l_ref):
        if prev.substrate_did != target.substrate_did:
            # The deployed service_id is the member MASTER DID under
            # --mint-masters, and the journal's plan JSON stores the compiler's
            # fabricated id instead -- so the message has to resolve the real
            # one from the member master's key file, or it names an id the
            # operator cannot obtain.
            name = member_identity::member_master_name(&svc.logical_ref, 0)?
            svc_id = match member_identity::resolve_member_master(dir, &name) {
                Ok(id) => substrate::derive_did_key(&id.public_key()),  # deployed id
                Err(_) => svc.service_id.to_string(),   # no master: plan id is real
            }
            bail "service '<ref>' is already deployed on substrate
                  <prev.substrate_alias-or-did> and this run would place it on
                  <target.alias-or-did>. A3 does not relocate -- the old instance
                  would keep running and keep republishing its endpoint record.
                  Undeploy it first:
                    roymctl --substrate <prev.substrate_did> --as <that substrate's
                      identity> svc remove --svc-id <svc_id>
                  then redeploy."

# ======================================================================
# Past this point nothing bails before the journal is consistent.
# ======================================================================

# --- resume (D-A3-10) -------------------------------------------------
# Without this, every retry appends a fresh record whose action table is
# empty, so nothing is ever skipped and a partial deploy re-sends work that
# already landed.
record_id = match journal.get_latest(&instance_id)? {
    Some(rec) if matches!(rec.state, Applying | Degraded) && &rec.plan == target_plan
        => rec.id,                      # resume: same plan, unfinished record
    _   => { let id = journal.append(target_plan, Planned)?;
             journal.update_state(id, Applying)?; id }
}
# A stored plan that *differs* deliberately starts a new record: the completed
# actions of a superseded plan say nothing about this one.

# --- masters (§6) ------------------------------------------------------
(deploy_plan, instance_certs, registry_certs) = if mint_masters {
    member_identity::substitute_and_certify_members(
        dir, target_plan, &clients, fallback_client, registry_url).await?
} else {
    (target_plan.clone(), {}, {})
}
if !mint_masters && any resolved_dependencies non-empty: eprintln warning   # unchanged

# --- apply -------------------------------------------------------------
report = deploy::apply_plan(
    ApplyRequest { plan: &deploy_plan, targets: &targets, fallback: fallback_target,
                   instance_certificates: &instance_certs,
                   registry_certificates: &registry_certs,
                   emit_bindings: mint_masters },
    &journal, record_id).await?

# --- post-apply registry verification (D-A3-17, §0.12) -----------------
# The one failure this slice cannot catch any earlier: a substrate publishes
# through its OWN configured registry, which nothing on the wire reports, so a
# split-registry fleet deploys cleanly and then cannot resolve. Runs only when
# the plan places services on more than one distinct substrate -- a
# single-substrate deploy has no cross-registry hop to get wrong.
if mint_masters && distinct target count > 1:
    probe_registry_reachability(&deploy_plan, &targets, fallback_target).await   # warns only

if report.is_complete():
    journal.update_state(record_id, Active)?
    println "Successfully deployed <n> service(s) for <instance_id>
             (<m> already applied, skipped)"
else:
    journal.update_state(record_id, Degraded)?
    for f in &report.failures:
        eprintln "  <logical_ref> on <alias-or-'--substrate'> (<did>): <error>"
    bail "<k> of <n> services failed to deploy; the app instance is DEGRADED.
          Nothing was rolled back. Re-run the same command to retry only the
          failed services."
```

`probe_registry_reachability` (new, `apps/roymctl/src/commands/app.rs`):

```
# For every registry URL the operator gave us, every member master should be
# resolvable: its endpoint record (so a dependent can find its address) and
# its master anchor (so a callee's handshake accepts its instance key).
#
# HEURISTIC (D-A3-17): these are the URLs `roymctl` dials, which need not be the
# registries the substrates themselves use. Every message below therefore names
# the URL probed, and none of them claims to speak for a substrate.
for url in distinct api_urls(targets + fallback):
    reg = RegistryClient::new(false, Some(url))
    for svc in &plan.services:
        # bounded retry: deploy publishes immediately (runtime.rs's publisher
        # is handed to the control plane for exactly this), but allow a couple
        # of seconds so a slow write is not reported as a topology fault.
        match retry_for(3s, || reg.lookup(svc.service_id, false)):
            Err(_) => warn "the registry at <url> cannot resolve member '<ref>'
                            (<did>). If that is the registry a substrate hosting
                            one of this app's services uses, its dependency
                            calls to '<ref>' will fail at call time. Every
                            substrate in the inventory must publish into and
                            resolve through one registry namespace (or enable
                            the DHT)."
            Ok(rec) if rec.info.substrate_id != <expected hosting did> =>
                warn "the registry at <url> resolves member '<ref>' to substrate
                      <found>, not <expected> -- a stale record from an earlier
                      placement is still winning there."
        match retry_for(3s, || reg.resolve_master_anchor(svc.service_id)):
            Err(_) => warn "the registry at <url> holds no master anchor for
                            '<ref>' (<did>). A substrate resolving through it
                            will reject this member's calls at the handshake.
                            Publish it with `roymctl identity publish-anchor
                            --master <name> --registry-url <url>`."
```

Iterating **distinct URLs** rather than per-target matters in the common case:
several aliases usually share one registry, and per-target iteration would emit
the same warning once per alias.

Four properties worth stating because they are easy to lose:

- **Every bail-able check runs before the journal is written** (D-A3-19), so a
  refusal leaves no record behind.
- **`wait_for_ready` runs before any deploy call**, so an unreachable substrate
  produces a clean failure with nothing applied — not a partial app.
- **`bail` at the end means a non-zero exit.** A partial deploy that exited `0`
  would be reported as success by every wrapper script an operator writes.
- **The registry probe warns, it does not fail** (D-A3-17), and it is a
  heuristic over the URLs `roymctl` was given, not a statement about what a
  substrate uses. What it found is a
  fleet-configuration fault, not a failed deploy: the services are running and
  `Degraded` would send the next run redeploying them for nothing.

### 7.3 `apps/roymctl/src/commands.rs` — pass the substrate DID lazily

`Commands::App`'s arm currently resolves the DID eagerly, before the handler
runs:

```rust
Commands::App { command } => {
    let substrate_did = get_substrate_did(substrate_opt, &dir)?;   // <-- eager
    app::handle(&command, &api_url, substrate_did, &dir, ...).await?;
}
```

`get_substrate_did` bails with *"Substrate DID not provided and substrate.key
not found"* ([commands.rs:98-103](../../../../apps/roymctl/src/commands.rs#L98)),
so a fully-placed deploy — and `app reconcile`, which needs no substrate at
all — fails before doing anything (D-A3-20). Change the arm to pass the raw
option through:

```rust
Commands::App { command } => {
    app::handle(&command, &api_url, substrate_opt, &dir, ...).await?;
}
```

`app::handle`'s third parameter becomes `substrate_opt: Option<String>`, and the
deploy arm calls `super::get_substrate_did(substrate_opt, dir)?` only inside
§7.2's `fallback` branch. `get_substrate_did` is a private free function in
`commands.rs` and is already visible to its child modules — no visibility
change needed.

`client_for` is unchanged and is now called once per target; its
`--ucan requires --as` guard applies per entry, which is the behavior wanted (an
entry with `ucan` but no `identity` and no global `--as` is rejected before
connecting).

`app reconcile`'s "no state found" message is reworded alongside D-A3-18:
`"No ACTIVE, APPLYING or DEGRADED state found for {instance_id}"`
([app.rs:227](../../../../apps/roymctl/src/commands/app.rs#L227)).

---

## §8 — Call-site sweep

Every site that must change, so nobody re-derives the list.

### 8.1 `PlannedService` literals (new `substrate` field)

| File | Line | Note |
|---|---|---|
| `crates/app_orchestration/src/compiler.rs` | 138 | The real one (§2.3) |
| `crates/app_orchestration/src/models.rs` | 598, 712 | tests → `substrate: None` |
| `crates/app_orchestration/src/journal.rs` | 313 | test → `substrate: None` |
| `crates/app_orchestration/src/reconcile.rs` | 142 | test → `substrate: None` |
| `crates/sdk/src/mapper.rs` | 317, 544, 551 | tests → `substrate: None` |

**Not affected — verified, so nobody re-checks:** `crates/sdk/src/mapper.rs:263`
and `crates/control_plane/src/service/orchestration.rs:1625,1707` construct the
**WIT** `PlannedService` (`syneroym_wit_interfaces::...::orchestrator::
PlannedService`), which A3 does not change.

### 8.2 `ServiceSpec` / `SynAppManifest` literals (new `placement` field)

| File | Line | Change |
|---|---|---|
| `crates/app_orchestration/src/catalog.rs` | 63, 66 | `placement: None` on both |
| `apps/roymctl/src/commands/app.rs` | 79, 96 | the legacy `.wasm` wrapper → `placement: None` |

### 8.3 `map_deployment_plan_to_wit` — **13 call sites**, and *two* changes each

The signature changes in two ways at once, and every site needs both:

1. `plan: DeploymentPlan` → `plan: &DeploymentPlan` (by value → borrow), so
   every site drops a `plan` move and passes `&plan`.
2. A new second argument, the subset to emit. A whole-plan call becomes
   `&plan.services.iter().collect::<Vec<_>>()`.

| File | Lines |
|---|---|
| `crates/sdk/src/mapper.rs` (tests) | 344, 366, 386, 398, 437, 462, 480, 491, 513, 570, 604, 623 — **12** |
| `apps/roymctl/src/commands/app.rs` | 157 — removed; the executor calls the mapper now |

Twelve of the thirteen are in `mapper.rs`'s own test module, so the churn is
mechanical but real. (`mapper.rs:397` is the *name* of
`map_deployment_plan_to_wit_maps_absent_fdae_to_none`, not a call — the call is
on 398.) A local helper in the test module keeps the diff small:

```rust
/// Whole-plan mapping, which is what every test but the subset one wants.
fn map_all(
    plan: &DeploymentPlan,
    instance_certificates: &BTreeMap<ServiceId, String>,
    registry_certificates: &BTreeMap<ServiceId, String>,
    emit_bindings: bool,
) -> anyhow::Result<WitDeploymentPlan> {
    let all: Vec<&PlannedService> = plan.services.iter().collect();
    map_deployment_plan_to_wit(plan, &all, instance_certificates, registry_certificates, emit_bindings)
}
```

There are **no callers outside these two files** (verified), so the signature is
free to change with no compatibility shim.

### 8.4 `substitute_and_certify_members` / `certify_instance`

| File | Line | Change |
|---|---|---|
| `apps/roymctl/src/commands/app.rs` | 136-146 | New arguments (§7.2) |
| `apps/roymctl/src/commands/identity.rs` | ~245 | `certify_instance` now `syneroym_sdk::deploy::certify_instance` |
| `apps/roymctl/src/commands/svc.rs` | 222 | The `--master` arm calls `certify_instance` for a **single** service against the one client `svc deploy` already has. Only the import path changes; `svc deploy` stays single-substrate by definition (D-A3-5 does not apply to it) |

⚠️ **Branch conflict warning.** `apps/roymctl/src/commands/svc.rs` and
`crates/sdk/src/lib.rs` are both modified on the current branch
(`feat/m05a-svc-deploy-container-cli`, container-deploy CLI work in flight).
A3 touches both. Land the container work first, or expect a merge in
`svc.rs`'s `--master` arm and `lib.rs`'s `deploy_container`.

### 8.5 `get_completed_actions` (return type change) and its new sibling

| File | Line | Change |
|---|---|---|
| `crates/app_orchestration/src/reconcile.rs` | 62-71 | `ActionRecord` fields instead of a `(String, String)` tuple (§4.4) |

`get_completed_actions_for_instance` (§4.3) is new, so it has exactly one caller
— §7.2's placement-change refusal — and no sweep of its own.

### 8.6 `DeploymentState` (new variant)

No exhaustive `match` on this enum exists anywhere (verified), but two
**equality** comparisons do, and grepping for `match` alone misses them:

| File | Line | Change |
|---|---|---|
| `crates/app_orchestration/src/journal.rs` | `Display` / `FromStr` | The `Degraded` arms |
| `crates/app_orchestration/src/reconcile.rs` | 55 | `== Applying` → `matches!(.., Applying \| Degraded)` (D-A3-18, §4.4) |
| `apps/roymctl/src/commands/app.rs` | 227 | The "No ACTIVE or APPLYING state found" message (§7.3) |

### 8.7 `app::handle`'s substrate parameter (D-A3-20)

| File | Line | Change |
|---|---|---|
| `apps/roymctl/src/commands.rs` | 185-196 | `Commands::App`'s arm stops calling `get_substrate_did` and passes `substrate_opt` through |
| `apps/roymctl/src/commands/app.rs` | 57-64 | `substrate_did: String` → `substrate_opt: Option<String>`; resolved on demand in the `fallback` branch |

**Not affected — verified, so nobody re-checks:** `Commands::Svc`,
`Commands::Kek`, and `Commands::Secret` keep resolving the DID eagerly. Each of
them always talks to exactly one substrate, so there is no lazy case to serve and
changing them would be churn.

---

## §9 — Tests

### Unit — `crates/app_orchestration`

`models.rs`
- `a_manifest_default_placement_round_trips_through_toml_and_json`
- `a_per_service_placement_override_round_trips_through_toml_and_json` — the
  one that would catch a `#[serde(flatten)]` regression
- `a_substrate_alias_rejects_a_bare_did` / `..._an_empty_name` /
  `..._a_path_separator`
- `a_planned_service_round_trips_its_substrate`

`compiler.rs`
- `a_per_service_placement_overrides_the_manifest_default`
- `a_manifest_without_placement_leaves_every_service_unplaced`
- `the_root_manifests_placement_cascades_into_a_spawned_child` (D-A3-3)
- `a_spawned_childs_own_placement_wins_over_the_inherited_default`

`substrate_inventory.rs`
- `an_inventory_parses_every_optional_field`
- `an_unknown_alias_error_lists_the_aliases_the_file_does_define`
- `a_non_did_key_substrate_id_is_rejected_at_parse`
- `a_service_type_the_substrate_does_not_declare_is_rejected`
- `check_placement_reports_every_problem_not_only_the_first`

`journal.rs`
- `an_action_row_round_trips_its_alias_and_substrate_did`
- `an_action_row_with_no_alias_round_trips_a_null_alias`
- `the_degraded_state_round_trips_through_display_and_from_str`
- `completed_actions_for_an_instance_span_every_record_oldest_first` (D-A3-22) —
  two records for one instance, and the query returns both records' rows in id
  order, so `rfind` finds the service's *current* home
- `completed_actions_for_an_instance_ignores_another_instances_rows`

`reconcile.rs`
- `recover_applying_drops_an_action_already_completed_on_the_same_substrate`
- `recover_applying_keeps_an_action_completed_on_a_different_substrate` — the
  first test in the tree to prove the filter filters at all (§0.2)
- `recover_applying_recovers_a_degraded_deployment_not_only_an_applying_one`
  (D-A3-18) — the state gate, which is the half a `retain`-only fix would miss

### Unit — `crates/sdk`

`mapper.rs`
- `mapping_one_service_resolves_a_dependencys_mode_from_the_whole_plan` — set a
  dependency to `TopologyMode::Redundant`, map only the dependent, assert the
  emitted binding says `redundant`. Fails under the naive filter-then-map shape
  (§5.1).

`deploy.rs` (with a `FailingApplier` fake — the reason D-A3-14 exists)
- `resolve_targets_fails_closed_naming_every_unknown_alias`
- `apply_plan_deploys_each_service_to_its_own_target`
- `apply_plan_records_one_action_row_per_service_and_substrate`
- `apply_plan_continues_past_a_failure_and_reports_it` (matrix row 12)
- `apply_plan_skips_a_service_already_completed_on_the_same_substrate`
- `apply_plan_does_not_skip_when_the_alias_now_resolves_to_a_different_did`
- `a_mapping_error_fails_only_its_own_service`

### CLI — `apps/roymctl/tests/cli_args.rs`

- `app deploy --help` lists `--inventory`
- a manifest with placement and no inventory file fails naming the path and the
  alias
- `a_placement_change_is_refused_naming_the_deployed_service_id` — drive the
  refusal (§7.2) against a temp journal holding a `COMPLETED` action row on a
  different substrate DID, and assert the message contains a `did:key:` id, not
  the compiler's fabricated one
- `a_placement_change_is_refused_after_a_degraded_run_not_only_an_active_one`
  (D-A3-22) — the exact sequence the round-1 fix opened: record left `Degraded`
  with one `COMPLETED` row, placement then edited. An `ACTIVE`-sourced refusal
  passes this run silently and leaves the service running on two nodes
- `a_fully_placed_deploy_does_not_require_a_substrate_key` (D-A3-20) — in a temp
  `--dir` with an inventory but no `substrate.key` and no `--substrate`, assert
  the failure is *not* "substrate.key not found"

### Integration / e2e — `crates/substrate/tests/multi_substrate_placement_e2e.rs`

New two-real-substrate harness, `Node` copied from
`master_endpoint_record_e2e.rs` (**not** `tests/common`'s
`SubstrateTestContext` — §0.6). One `operator` identity owns both nodes. App:
`frontend` (node A) depends on `backend` (node B).

1. `a_two_substrate_app_deploys_each_service_to_its_placed_node` — after apply,
   node A's `list` contains only `frontend`'s master DID and node B's only
   `backend`'s.
2. `a_placed_members_endpoint_record_resolves_to_its_own_substrate` —
   `resolve_iroh_addr` on `backend`'s master DID reaches **node B**, not node A.
   This is what §0.1's `substrate_id` bug would break silently.
3. `a_certificate_minted_against_one_substrate_is_rejected_by_another` — the
   negative half of §0.1: mint against node A, deploy to node B, assert the
   `"not the key this substrate would derive"` rejection.
4. `an_unreachable_substrate_leaves_the_deployment_degraded_and_retryable`
   (matrix row 12): stop node B, apply, assert `frontend` deployed, `backend`
   failed, the record is `DEGRADED`, and the action rows show one `COMPLETED`
   and one `FAILED`; restart node B, apply again, assert `backend` deploys,
   `frontend` is **skipped**, and the record becomes `ACTIVE`.
5. `a_dependencys_record_resolves_through_the_dependents_own_registry` —
   `backend`'s master DID looked up through the registry **node A** is
   configured with, asserting the returned record's `substrate_id` is node B's
   DID. This is the cheap, direct proof of §0.12: it is what fails when the
   fleet's registries are split, and it is what §7.2's post-apply check
   automates. Shares the harness with tests 1-4 at nearly no cost, since
   `shared_registry_url` is already how the fixture is wired.

Test 4 is the slice's centrepiece — it is the only proof of the partial-failure
semantics `task.md` asks for, and the only proof the resume path works. Test 5 is
the cheapest proof of the one failure that is otherwise silent.

**One further test, larger, flagged for a decision (§12):**

6. `a_guest_reaches_a_dependency_placed_on_another_substrate` — a WASM guest on
   node A calling `backend` **by declared dependency name** over a real hop.
   Nothing proves placement + A2's host-side resolution + A1's records *compose*:
   A2 proved dependency resolution single-node through the router
   (`proxy_dispatch.rs`), and tests 2 and 5 above prove address resolution, not a
   call. This is the reference scenario's step 2, and it is what makes the
   milestone's exit criteria reachable.

   The cost is honest: it needs a WASM-guest two-node harness, which
   `deferred-backlog.md` already records as unbuilt and "judged out of
   proportion" once, in A2's own coverage row — building it here **discharges
   item (1) of that row**. If it is deferred again, the row must be updated to
   say A3 also declined it and why, not left as A2's residue.

### Gates

`cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
--all-features`, `cargo test --workspace`, `mise run test:e2e`.
`mise run bench:latency` and `test:smoke` are **not** affected: neither touches
`deploy_plan` or the app-plan path (verified — `roymctl app deploy` is its only
caller).

---

## §10 — Phase order and mergeability

| Phase | Content | Independently mergeable? |
|---|---|---|
| 1 (§2) | Manifest placement + compiler | **Yes.** Adds a field nothing reads yet. Zero behavior change |
| 2 (§3) | `substrate_inventory` module | **Yes.** New file, no callers yet |
| 3 (§4) | Journal actions + `Degraded` + the `recover_applying` state gate + `get_completed_actions_for_instance` | **Yes**, with §8.5's and §8.6's caller updates. Ships the `Degraded` *state* together with everything that has to see it — the recovery surface (D-A3-18) and the query the placement-change refusal reads (D-A3-22). A `Degraded` variant that half the tree is blind to is worse than no variant at all, which is the mistake round 1 made |
| 4 (§5) | Mapper subset + `apply_plan` | **Yes.** `apply_plan` has no caller until phase 6; the mapper change is a pure refactor plus a latent-bug fix, across 13 call sites (§8.3) |
| 5 (§6) | Per-substrate identity | **No** — must land with phase 6, since it changes `substitute_and_certify_members`'s signature |
| 6 (§7) | `roymctl` wiring: the lazy fallback (§8.7), the placement-change refusal, and the post-apply registry probe | **No** — the slice's behavior change; lands with phase 5 |

Phases 1-4 can merge in any order and are each small. Phases 5-6 are one commit.
The e2e's tests 1-5 (§9) land with 5-6 or immediately after; test 6, if it is
built at all (§12 question 8), is its own commit — it is a harness, not a
behavior change, and blocking the slice on it would be the wrong trade.

---

## §11 — Docs and backlog

**Docs**
- `docs/developer-guide.md` — an inventory file example, a two-substrate deploy
  walkthrough, the three abilities each entry's credential needs (§0.4), the
  **shared-registry precondition** and why it cannot be checked before deploying
  (§0.12), the `api_url`-versus-`--registry-url` distinction, and the "delete
  your old `deployments.db`" note (D-A3-15).
- `task.md` — dated corrections for §0.1-§0.15; A3's row flipped at sign-off.
- `status.md` — an A3 section with evidence, matching A0/A1/A2/P0's shape.

**Backlog rows to add**
- *Placement change is refused, not relocated* (D-A3-12) → **A5**.
- *Substrate capabilities are operator-declared, not probed* (§0.9) → **A4**,
  where a status query exists to carry them.
- *`Degraded` has no automatic exit, and nothing retries on its own* (§0.14) —
  only a re-run of `app deploy` clears it → **A5**.
- *`ActionState::Pending` still has no writer* — A3 writes `InProgress`
  directly; a planner that enqueues ahead of applying is A5's.
- *Multi-substrate placement requires one registry namespace, verified only
  after the fact* (§0.12) — a substrate publishes through its own configured
  registry, which nothing on the wire reports, so `roymctl` can warn after
  deploying but never refuse before. Closing it needs the substrate to report
  its registry configuration (natural fit alongside A4's status query) →
  **A4/A5**.
- *Failure-matrix row 10 is unmet: a deploy retried after a lost response
  redeploys rather than no-op'ing* (§0.15) — the action row is `IN_PROGRESS`, not
  `COMPLETED`, and `deploy_with_context` has no (instance, service, content
  hash) dedup → **A5**.
- *`app deploy` discards every compiled plan but the last, so a `Spawn`
  dependency's services are never deployed* (§0.13) — predates A3; makes
  D-A3-3's placement cascade unobservable → **TBD**.

**Backlog rows to update**
- *"A deploy-only grantee's rollback attempt is denied a second time"* — its
  target says "A3 (when the substrate inventory starts issuing grants)". A3's
  inventory **holds** grants, it does not issue them. Retarget to **A5** and
  add the operator guidance §0.4 names (grant `deploy` + `undeploy` + `status`
  together) to the developer guide.
- *"A relocated-away substrate keeps trying to publish a member's record"*
  (A3/A5) — A3 does **not** fix it; D-A3-12 refuses the move that creates it,
  which narrows the exposure but does not close the row.
- *"App Supervisor: placement is declared, not scheduled"* — stays open as the
  record of what A3 deliberately did **not** build (pools, constraints,
  solving); note that the selector shape it asked for shipped.
- *"No remote `ControllerAgreement` claim"* — add A3 as the consumer that makes
  it felt: every substrate in the inventory must be claimed on its own host
  before it can appear there.
- **Two rows are discharged by §9's test 6, not one** — which is what makes it
  worth its cost (§12 q8):
  - *"A2's own two-substrate e2e coverage is partial"*, item (1): the
    `dependency_binding_e2e.rs` two-node test proving the reference scenario's
    step 2 from the dependent's side.
  - *"No live, wire-level proof that a guest-origin call presents its certified
    instance identity across a real cross-node hop"* (the A0 row): the same
    harness, the same hop, and `CallOrigin::Guest` is precisely the arm a WASM
    guest's outbound call takes — the arm that row says is proven only at router
    level today.

  Either mark both resolved (if test 6 lands) or record that A3 declined it too,
  with the reason. Leaving them worded as A0's and A2's residue after three
  slices have passed on the same missing harness is how rows go stale.

---

## §12 — Questions for the requester

1. **Executor home.** §5 puts `apply_plan` in `crates/sdk`, so the
   two-substrate e2e can drive it (`sdk` is a dev-dependency of
   `crates/substrate`; `roymctl` is a binary and cannot be linked). The
   alternative is keeping it in `roymctl` and testing at CLI level only.
   Recommended as written. **Note for A5:** `sdk` is only a *dev*-dependency of
   `crates/substrate` today, so A5's supervisor role will have to promote it to
   a real one.
2. **§0.3's call shape.** One `deploy-plan` per (service, substrate)
   (recommended, no wire change) versus changing `deploy-plan`'s WIT return to a
   per-service result list (one RPC, cleaner wire, touches the component
   boundary).
3. **§0.7.** Confirm `PlannedService.substrate` records the **alias**.
4. **§0.5 / D-A3-12.** Confirm a placement change is a hard error rather than a
   `Remove` + `Add` relocation.
5. **D-A3-3.** Confirm the root manifest's placement default cascades into
   spawned child apps.
6. **§0.10.** Is a CLI override wanted (`--place <service>=<alias>`, or a
   default-alias flag) so a placement-carrying manifest can be retargeted
   without editing it? Not proposed; it is the natural answer if manifest
   portability turns out to bite.
7. **§0.12 / D-A3-17.** The split-registry check **warns** rather than failing.
   The argument for warning: the deploy landed, and `Degraded` would send the
   next run redeploying healthy services. The argument for failing: an app whose
   cross-substrate resolution cannot work is not usable, and a warning in a long
   deploy log is easy to miss. Recommended as a warning; say so if you want it
   fatal.
8. **§9's test 6.** Build the WASM-guest two-substrate dependency-call harness in
   A3, or defer it (updating **both** backlog rows in §11 to say A3 declined it
   too)? It is the largest single item in the slice, and it is the only test that
   proves placement, A2's resolution, and A1's records compose — while also
   discharging A0's outstanding `CallOrigin::Guest`-over-a-real-hop row. Two rows
   for one harness is the argument for building it now.
9. **§0.14.** Confirm that "keep retrying" meaning *the operator re-runs the
   command* is the intended reading through A3-A4. It sets what `Degraded` costs
   in the meantime: an app stays partially deployed until a human notices.

---

## §13 — What this hands A4 and A5

**A4 (health, read-only)** inherits the inventory as the list of substrates to
poll and the per-substrate credential to poll with — both already resolved and
preflighted here. Two of A3's findings land in A4's lap the moment a substrate
status query exists, and both become cheap there: §0.9's declared
`capabilities` can be checked against a reported set instead of trusted, and
§0.12's registry configuration can be *reported* instead of guessed, turning
A3's post-apply warning into a real preflight refusal.

**A5 (the supervisor loop)** inherits four things:

- `PlanApplier` (D-A3-14) — the trait ADR-0021 §5 names, already the only path
  a deploy takes. A6's outbox implementation replaces its body.
- The journal's action rows as the per-(service, substrate) delivery record the
  reconcile loop needs, and `Degraded` as a real state rather than a word.
- `certify_placed_members` (§6) — the per-substrate certificate and record
  minting the online-key posture automates. A5 changes *who holds the master*,
  not *how a certificate is bound to a node*.
- The refusal in D-A3-12 as the exact place relocation gets implemented, with
  the two-publisher failure it prevents already written down.

**One tension A5 must resolve, not discover:** A3's inventory stores a
credential per substrate, and P0's `security` gate is node-owner-only (P0's
D-P0-8 and its own backlog row). A supervisor that provisions secrets for its managed
services needs `substrate/admin` on every managed substrate, which entails
everything there and weakens failure-matrix row 14. A3 does not make this worse
— it just makes the fleet bigger.
