# ADR-0021: Binding Propagation by Push; No Live App Directory

**Status**: Accepted (2026-07-27), jointly with
[ADR-0020](0020-stable-logical-service-identity.md), which it depends on.
Supersedes the "Dynamic Pull (Server SynApp Mode)" design recorded in
[system-architecture.md](../system-architecture.md) §LFC-MGT and
[system-requirements-spec.md](../system-requirements-spec.md) §LFC-MGT.
Amended 2026-08-01 -- see the dated amendment note at the end of this
document.

**Context**:

The Active Control Plane was specified as a live registry that services query on
the hot path: "The client SDKs embedded within each `SynSvc` query this registry
at runtime to resolve logical IDs to physical addresses"
([system-architecture.md](../system-architecture.md) §LFC-MGT #3; the same design
in [system-requirements-spec.md](../system-requirements-spec.md) §LFC-MGT
"Active Control Plane Mode").

That places a control-plane service in the path of every cross-service call, and
it drags in four pieces of machinery: bootstrap discovery of the registry itself
(nothing can resolve the resolver); stale-versus-fail semantics on registry
outage, because the SRS separately promises the data plane survives one; read
authorization on what is effectively a topology map of the whole application; and
a new WIT interface so guests can resolve names at all.

Two observations make all four unnecessary.

**There are two mappings, not one.**

1. **logical service name → member master DIDs.** Application-scoped. This is
   `AppRegistry` / `TopologyEntry` / `StaticInventory`
   ([resolver.rs:168](../../crates/app_orchestration/src/resolver.rs#L168),
   [:213](../../crates/app_orchestration/src/resolver.rs#L213),
   [:251](../../crates/app_orchestration/src/resolver.rs#L251)).
2. **member master DID → endpoint.** Network-scoped, resolved per call:
   `ProxyRouter::invoke_remote` goes through
   `resolve_iroh_addr(&self.registry_client, &req.target_service)`
   ([proxy.rs:326](../../crates/router/src/proxy.rs#L326)).

**What is built versus what this design adds.** Mapping (2)'s *shape* — one
registry lookup, per call, on the DID the caller holds — is built and in use
today. What is **not** built is publishing a record under a master DID: the
registry verifies a record's signature against the key resolved from the
`service_id` it is keyed under
([registry.rs:234](../../crates/community_registry/src/registry.rs#L234)), so an
instance key cannot publish under its master, and `MasterAnchorPayload` is a
revocation list with no forward index. [ADR-0020
§6](0020-stable-logical-service-identity.md) closes that with delegation-signed
endpoint records, keeping resolution at exactly one lookup. This ADR's argument
depends on that section shipping first; without it, a relocated member is not
resolvable at all.

With it, the split matters as follows. **Reinstantiating** a member — relocation,
restart, crash recovery — changes only mapping (2), and the member republishes
its own record under an unchanged master DID. Nothing application-level
propagates. Mapping (1) changes only on a **membership change**: scale out or in,
*replacing* one member with a different one, or a topology-mode change. Note the
distinction between *replacing a member* (a different master joins the set, a
push) and *reinstantiating a member* (same master, no push) — it is the
load-bearing one for this whole ADR.

Membership changes are rare and operator-initiated. A live directory would
therefore be a hot-path dependency built to serve a cold event.

**Decision**:

## 1. Bindings are pushed into service configuration; no queryable app directory exists

The App Supervisor writes the current `TopologyEntry` for each declared
dependency into the dependent service's configuration. `StaticInventory` stays
the only `AppRegistry` implementation; the push is simply what populates it.

Everything above it is untouched and keeps working: `LogicalResolver`
([resolver.rs:457](../../crates/app_orchestration/src/resolver.rs#L457)), the
topology cache, `TopologyEpoch`, `cache_ttl`, and the whole selection layer —
round-robin, rendezvous hashing, `EntityTagSharding`, `RangeRoutingTable`. The
pushed value is a member *list*, and selection stays local to the caller, so
`Redundant` and `Sharded` topologies work under push exactly as they would have
under pull.

## 2. Guests name the logical service; the host resolves it

Today the guest supplies the target DID itself: `ProxyRequest { target_service:
service, ... }`, where `service` arrived as an argument on the guest's own call
([host_capabilities.rs:1110-1111](../../crates/sandbox_wasm/src/host_capabilities.rs#L1110)).
A guest that reads its binding once at startup keeps a stale DID until its
instance is recycled, and `app-config` offers only `get` / `get-section` with no
change notification
([app-config.wit](../../crates/wit_interfaces/wit/app-config/app-config.wit)).

So the guest names a **declared dependency** and the **host** resolves it through
`LogicalResolver` before constructing the `ProxyRequest`. A pushed binding then
takes effect on the next call, structurally: a guest cannot snapshot an
identifier it never holds.

**The guest passes the dependency name only — not a `LogicalServiceRef`.**
`LogicalServiceRef` is `{app_instance_id, service_name}`
([models.rs:128](../../crates/app_orchestration/src/models.rs#L128)); making
that the WIT-visible argument would let a guest name an arbitrary app instance,
contradicting this ADR's own least-privilege consequence below. The host
supplies `app_instance_id` from its own `HostState` and resolves the pair. A
cross-app `Bind` dependency (§7) is likewise named by its **local declared
name**, with the host holding the foreign `app_instance_id` from the deploy that
established the bind.

**Raw-DID targeting is kept as a second variant, not removed.** It has real
callers that are not dependency resolution: the guest's self-proxy into its own
service's native capabilities
([host_capabilities.rs:1105-1111](../../crates/sandbox_wasm/src/host_capabilities.rs#L1105)),
native dispatch, and external/`roymctl` callers that legitimately address a
specific DID. What changes is that resolving a *declared dependency* no longer
goes through it.

**This is a required part of the decision, not a refinement of it.** Without it,
push does not propagate — the write lands in the host's configuration store and
the guest never looks — and every membership change becomes a rolling restart of
every dependent. The alternatives (a config-watch mechanism in the WIT, or a
restart-on-binding-change policy) are both strictly more machinery for a strictly
worse result.

## 3. Binding writes are epoch-guarded

`TopologyEntry.epoch: TopologyEpoch`
([resolver.rs:67](../../crates/app_orchestration/src/resolver.rs#L67),
[:176](../../crates/app_orchestration/src/resolver.rs#L176)) already exists and is
already documented as incrementing on any membership or mode change. A substrate
compares the incoming epoch against the one it holds:

- **lower** → reject as stale. This is the regression case.
- **equal, identical content** → success, no-op. This is the ordinary retry §5
  says to expect, and it must be distinguishable from staleness or best-effort
  delivery cannot tell "you are behind" from "I already have this."
- **equal, different content** → reject as a *conflict*, reported distinctly.
  Two writers produced different member sets at the same epoch, which is the
  signal §4 exists to catch.
- **higher** → apply.

Without the guard a late-arriving retry silently regresses a mapping to a
superseded member set — invisible when it happens, diagnosed much later. Note
this is a distinct concern from deploy idempotency: a re-sent *deploy* is
deduplicated on (instance, service, content hash), while a re-sent *binding
write* is deduplicated on epoch plus content. Both are needed; neither covers the
other.

**Who mints the epoch (M05A A5c, §19.3/D-A5c-4).** This section names the
guard but not the number's owner. The App Supervisor mints and holds it,
**per dependent service**, not per dependency: one counter for
`(app_instance_id, dependent_logical_ref)`, carried on every binding that
dependent's own write emits, incremented before the write it will label —
never after, and never derived from what the substrate reports holding.
`roymctl app deploy`'s unmanaged, hand-deployed path keeps writing at epoch
`0`, which reads as "no supervisor has written here" rather than a
regression: a supervisor later adopting that instance starts its own counter
above it, so its first push is still `higher` and applies cleanly.

## 4. An app instance has exactly one writer, and it is stamped

Each app instance record on a substrate carries the managing supervisor's DID and
a monotonic generation. A substrate rejects binding writes and lifecycle actions
from a lower generation.

**Permission alone is not sufficient**, and this is the non-obvious part. Two
supervisors started by the *same* operator both hold valid authority, so a
permission check passes for both. Last-write-wins does not converge between them
— it flaps, silently, because binding writes are cheap and unlogged, and both
supervisors also remediate, producing double restarts and double deploys. The
generation stamp is what makes the second supervisor lose deterministically.

**The generation is issued by the operator, never self-incremented.** A
supervisor that swept a substrate, observed generation N, and claimed N+1 would
defeat the entire mechanism: a rebuilt supervisor and a rogue second one would
be indistinguishable, since both can read N and both can add one. So taking over
an app instance is an explicit `roymctl` adopt action that mints the next
generation under the owner's authority, and a supervisor that finds a **higher**
generation than its own on a substrate **stops managing that instance and
alerts** rather than bumping. Rebuilding a lost supervisor is therefore a
deliberate adopt, which is the correct posture anyway — it is exactly the moment
an operator should confirm that the old one is really gone.

To be precise about what this does and does not do: the generation is a
**tiebreaker among already-authorized writers**, not an authorization mechanism.
Authorization is the owner-rooted deploy capability. A party without that
capability is refused regardless of the generation it presents.

## 5. Push failure is sticky; delivery tracking is therefore mandatory

The honest cost of choosing push: a pull model degrades to *stale but
converging* — a missed update is repaired by the next fetch, and a restart fixes
it. Push degrades to *wrong until retried*, and a restart does not fix it. A
dependent that was unreachable when its dependency changed holds a dead binding
indefinitely.

So delivery tracking is not an optimization here, it is what makes the model
correct:

- **Before M5's async primitives:** synchronous best-effort delivery with
  in-process retry, behind a narrow "apply this action to that substrate" trait.
  A dependent that cannot be reached leaves the app instance `Degraded`, and the
  supervisor keeps retrying on its normal loop.
- **After M5:** that trait gains an outbox/DLQ-backed implementation for durable
  delivery and terminal-failure handling. Nothing above the trait changes.

The trait boundary exists specifically so this project does not grow a second,
competing retry mechanism that then has to be unwound.

## 6. What is deliberately not built, and how it returns

A pull-side directory is **re-addable as a second `AppRegistry` implementation**
with no redesign: the trait, the cache, the epoch, and the TTL all already exist
to support one. That is what makes it safe to decide push now on incomplete
information.

The trigger to revisit is measured, not aesthetic: if binding convergence across
dependents cannot be kept inside its budget by delivery retry alone, add the pull
path. Recorded in the milestone's exit criteria and in the deferred backlog so it
is picked up rather than remembered.

## 7. Cross-application bindings are best effort

`AppDependencySpec::Bind`
([models.rs:275](../../crates/app_orchestration/src/models.rs#L275)) lets app A
depend on a live service in app B. Under ADR-0020 that binding survives
relocation, so it can only break on genuine *replacement* of B's service — and no
directory exists for A to observe B through.

The rule: **A's owner owns the consequence.** A opted into depending on something
it does not control. What keeps that from being a shrug is that A's supervisor
probes its bound external dependencies on its normal poll loop, so a break
surfaces as an alert to A's owner rather than as user-visible failure discovered
later.

**That probe needs no capability A does not already hold**, and this is worth
stating because it is a different mechanism from how the supervisor watches its
*own* services. Those are watched with a substrate-level status query, which is
an owner-authorized operation on a substrate the supervisor manages. A bound
external dependency is on someone else's substrate, and no such authorization
exists across owners. But A does not need it: the bind already grants A the
right to *call* B's service, so the probe is simply a call A is entitled to
make.

**Whose credential, though.** The supervisor is a different principal from A's
services, so "a call A is entitled to make" is only available to it if it can
act *as* the depending member — which means holding that member's master key.
That splits by the posture chosen in
[ADR-0020 §3](0020-stable-logical-service-identity.md):

- **Online-key posture:** the supervisor holds the member master, calls as the
  member, and gets an active liveness signal. §7 works as described.
- **Attended posture:** the supervisor holds no master key and therefore has no
  credential for an active probe. It falls back to the passive signal — the
  failure of real traffic — which is credential-free and does work, but is
  strictly weaker: it surfaces the break only *after* a user-facing call has
  already failed, which is the outcome an active probe exists to pre-empt.

Two consequences worth naming rather than discovering. First, an operator
choosing the attended posture is also choosing passive detection for bound
external dependencies. Second, probing as the member means supervisor-generated
traffic lands in **B's** FDAE decision traces attributed to A's member — B's
operator will see it, and should not have to guess where it came from.

In either posture the supervisor learns liveness and reachability, which is all
§7 claims. It does **not** learn why B is unhealthy, or that B's owner is
mid-migration, and this ADR does not promise that.

## 8. Naming

The service is the **App Supervisor**. The registry half is gone, so "registry"
oversells it — and each obvious alternative is already spoken for in this
codebase: *registry* is the community registry (DID → endpoint), *App Registry*
is the trait above, *control plane* is the substrate's own deploy RPC crate, and
*controller* is the owning entity in a `ControllerAgreement`.

It exposes one interface, **`supervisor`**, consumed by `roymctl`: submit desired
state, retire, pause, force reconcile, adopt (§4), and read back status, alerts,
and convergence state. An earlier draft called it `app-control`, which is one
word-order away from `control_plane` and so fails the same
neighbouring-name test that rules out "registry" and "controller".

**The invariant is that there is no *service-facing directory* interface** — no
constituent service resolves anything by calling the supervisor, which is the
whole point of this ADR. There is an operator-facing read surface, and it is
required: an operator must be able to see health, alerts, and whether bindings
have converged. Saying "no read interface" flatly, as an earlier draft did,
contradicted the milestone's own exit criteria. Recorded in
[TERMINOLOGY.md](../TERMINOLOGY.md).

**Consequences**:

- The data plane has **no** control-plane dependency at call time. The SRS's
  control-plane/data-plane isolation guarantee is satisfied structurally rather
  than by cache-staleness rules, and the "registry is down, serve stale or fail?"
  question does not arise.
- Topology knowledge is least-privilege by construction: a service learns its own
  declared dependencies and nothing else, where a directory would have been able
  to answer questions about the whole app.
- No bootstrap discovery problem: nothing needs to find the supervisor, so the
  supervisor needs an identity but not a *discoverable* one. It is a client of
  substrates, not a server to services.
- No new WIT interface for guests, and no third `AppRegistry` implementation.
- The proxy target gains a dependency-name variant alongside raw-DID targeting —
  a real change in `crates/router` and `crates/sandbox_wasm`, and a prerequisite
  rather than a follow-up.
- **This ADR is incorrect without [ADR-0020 §6](0020-stable-logical-service-identity.md)
  (delegation-signed endpoint records).** Reinstantiation propagating nothing is
  the central claim, and it holds only once a member can republish its endpoint
  under an unchanged master DID. That work sequences before the resolution
  change.
- Correctness now depends on delivery, which depends on M5's outbox for the
  durable case. The pre-M5 supervisor is explicitly best-effort and says so in
  its status output rather than implying convergence it cannot guarantee.
- Authenticated multi-substrate deploy remains blocked on the
  `ControllerAgreement` creation tool
  ([M04A B7 plan](../planning/milestones/M04A-proxy-and-auth-foundation/plans/B7.md)
  §6.1), which is unchanged by this ADR but becomes load-bearing for it: the
  supervisor cannot hold a deploy capability on a substrate it does not own until
  ownership is establishable.

**Amendment (2026-08-01, after Slice A5a implementation).** Two places
where planning and building A5 found this ADR left a real decision
unmade, not merely under-specified. Recorded here rather than edited
silently into §4/§5 above, for the same reason as ADR-0020's own
amendments: the gap between what this ADR said and what the tree needed
should stay legible.

1. **§4 does not say where the generation is durable, and that decision
   decides whether a rebuilt supervisor can be fooled.** "Each app
   instance record on a substrate carries the managing supervisor's DID
   and a monotonic generation" names what the record holds, not which
   party's copy is authoritative. Three candidates exist -- the
   operator's `roymctl`, the supervisor's own store, the substrate's own
   app-instance row -- and only one of them cannot simply believe
   itself: two supervisors racing an adopt, or one supervisor rebuilt
   from nothing, must resolve their disagreement at a party neither of
   them controls. **The substrate is the durable arbiter.** Its
   `app_instance_management` row (`(app_instance_id) -> (owner_did,
   supervisor_did, generation)`, `crates/data_db/src/registry_store.rs`)
   is what `adopt` reads before minting `held + 1`
   (`orchestrator.app-instance-management-of`), what `claim-app-instance`
   writes under the same four-case rule every other write uses
   (`check_generation`,
   `crates/control_plane/src/service/orchestration.rs`), and what every
   generation-gated verb (`deploy-plan`, `write-bindings`, `restart`,
   `undeploy`) checks a presented generation against. The supervisor's
   own copy (A5b's `desired_state.generation`) and the operator's `--
   generation` flag are both caches of this row, not sources of truth --
   a supervisor that reads a **higher** generation than the one it holds
   has been superseded and stops managing the instance (failure-matrix
   row 9), rather than trusting its own cache over the substrate's.
2. **§5's "narrow 'apply this action to that substrate' trait" names one
   action; A5 needs three.** Delivery is sticky for every action a
   supervisor issues without a human present, not only the initial
   deploy: a binding push that never reaches an unreachable dependent is
   exactly as wrong-until-retried as a deploy that never lands, and A5's
   bounded restart is a third such action. The trait -- `SubstrateActor`
   in `crates/sdk/src/deploy.rs`, which A3 introduced as `PlanApplier`
   when `apply_plan` was its only method -- widens to `apply_plan`,
   `write_bindings`, `restart`. A6's durable outbox/DLQ implementation
   swaps all three at once, which is the property this section's
   "nothing above the trait changes" promise depends on: it only holds
   if every action that must become durable is *on* the trait it swaps.

Full reasoning for both is in
[slice-a5-implementation-plan.md](../planning/milestones/M05A-app-supervisor/slice-a5-implementation-plan.md)
§0.10 and §0.22; only the corrections themselves are repeated here.
