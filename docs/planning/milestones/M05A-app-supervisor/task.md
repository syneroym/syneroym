# Milestone 5A: App Supervisor (M05A-app-supervisor)

> **Provenance.** Split out on 2026-07-27 from Milestone 5's item 2 ("Active
> Controller"), and supersedes the *Interstitial: Live App-Context Registry*
> placeholder reserved on 2026-07-24
> ([meta-implementation-plan.md](../../meta-implementation-plan.md)). Both of
> that interstitial's goals are carried forward here — logical-name resolution
> backed by real deployment state, and `expected_asserter_did` publication
> (M04B Slice B3's D-B3-8 residual). The mechanism changed: see
> [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) and
> [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md),
> both accepted 2026-07-27, which are the design of record for this milestone.
>
> **What this milestone is.** Today an operator deploys an app to *one*
> substrate with `roymctl` and inspects it by hand. This milestone makes an app
> a supervised, multi-substrate thing: desired state held in one place,
> deploy-time checks and retries, health monitoring, bounded remediation,
> alerting, and dependency bindings pushed to constituent services as topology
> changes. The component is the **App Supervisor**, a substrate role.
>
> **What it is deliberately not.** Not a live registry — nothing queries the
> supervisor to make a call (ADR-0021). Not a scheduler — placement is
> operator-declared, not computed. Not a failover system — `[PLT-RED]` promotion
> stays manual.

## Goal

By the end of M05A, an operator declares an app whose services are placed across
more than one substrate, hands it to a supervisor, and walks away: the supervisor
deploys it, retries what fails, notices when a service becomes unhealthy,
restarts it within a bounded policy, raises an alert when it cannot, and keeps
every dependent's bindings current — without any service ever calling the
supervisor.

---

## Requirement IDs (Traceability)

| Requirement ID | Sub-scope in M05A | Current matrix status |
|---|---|---|
| `[LFC-MGT]` (App Supervisor) | Multi-substrate placement, substrate inventory, unattended deploy with retry, health monitoring, bounded remediation, alerting, binding propagation | New row, status **Planned** |
| `[FND-IDT]` (stable service identity) | Master DID per *member*; instance keys delegated via `DelegationCertificate` for outbound-call authentication; ingress-side scope enforcement; master-signed endpoint records | Extends the existing `[FND-IDT]` row, whose scope is already "Master Key → Temporary Key delegation certificates" — the same mechanism applied to services. **Not** `[FND-IAM]`, which is Access Control. |
| `[PLT-DAP-01]` (app-context registry) | Logical-name → member-set resolution backed by real deployment state; `expected_asserter_did` per member | Inherited from the superseded interstitial |

---

## Explicit non-goals

- **No live/pull directory interface.** ADR-0021 §6 records the trigger for
  revisiting and the fact that it is re-addable as a second `AppRegistry`
  implementation without redesign.
- **No relocation of stateful services.** Remediation is restart-in-place.
  ADR-0020 removes the *identity* blocker; the data blocker (replication) is M7.
- **No scheduler / placement solver.** Placement is declared by the operator and
  resolved against the substrate inventory. Substrate pools and constraint-based
  placement are later work.
- **No automatic rollback of a partially-applied app.** Partial failure leaves
  the instance `Degraded` and retrying; rolling back a stateful service is itself
  destructive.
- **No alerting delivery beyond event emission and MQTT.** In scope: structured
  events, queryable through the operator read surface, and published to an MQTT
  topic (the broker is already in-process). Out of scope: SMTP, webhook, and
  paging delivery, which belong in a separate consumer service.
- **No key escrow or master-key rotation.** Losing a member master key is
  unrecoverable and orphans that member's stored data
  ([ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §4).
  Backup is an operator duty this milestone documents rather than automates.
- **No supervisor HA.** Single writer, enforced by the generation stamp
  (ADR-0021 §4). A lease for redundant supervisors is post-M5.
- **No external discovery of an app's topology.** Slice A7 mints the
  app-instance master DID and nothing more; publishing it to the registry,
  the signed topology document, and the resolve RPC are slices S1-S2 of the
  [Logical Service Discovery Overlay](../../meta-implementation-plan.md#committed-work-logical-service-discovery-overlay-2026-08-02)
  ([ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)).
  Through this milestone, a caller outside an app instance still cannot
  resolve a logical service inside it.

---

## Dependency gates

| Gate | Status | Effect if unmet |
|---|---|---|
| `ControllerAgreement` creation tool | **Complete (2026-07-30)** — see P0 below | Was not a *functional* blocker: an unowned substrate granted `orchestrator/deploy` to every verified caller ([io.rs:174-182](../../../../crates/router/src/route_handler/io.rs#L174)), a deliberate bootstrap posture, before P0 removed it. Cleared. |
| M04A/M04B identity + FDAE | Complete | ADR-0020 depends on `subject_did`/`caller_did` already being the master DID. |
| M5 item 1 async primitives (Outbox, DLQ, cron leases) | Not built | Not a gate for this milestone; gates the post-M5 slice only (see below). |
| M7 replication | Not built | Gates stateful relocation only. |

---

## Slices

### P0 — `ControllerAgreement` creation tool *(pulled forward from M5 item 5)* — **Complete (2026-07-30)**
Implementation plan: [slice-p0-implementation-plan.md](slice-p0-implementation-plan.md).
Verification evidence: [status.md](status.md)'s P0 section.

A prerequisite pulled in from another milestone rather than part of the A-series
design, which is why it is numbered separately.

Nothing in the tree can create a `ControllerAgreement`: `roymctl` creates
identities and `DelegationCertificate`s but has no verb to produce one, and
`[iam].admin_ucan_root` is set in no config file and no e2e or smoke setup. So
`admin_root` resolves to `None` on every real deployment, and B7b's ownership
gate is inert.

**What that actually meant, before this slice.** An unowned substrate did
*not* fail closed. It issued `orchestrator/deploy`, `undeploy`, and `status`
to **every verified caller**
([io.rs:174-182](../../../../crates/router/src/route_handler/io.rs#L174),
before P0 removed the grant) — a deliberate bootstrap posture, since
default-deny would have bricked a substrate permanently: you could not deploy
the thing that establishes ownership. So a supervisor could already deploy to
unowned substrates, and A3 was not functionally blocked by this alone.

The problem is what this milestone does to that posture. Today it is one
operator hand-deploying to their own substrate with a human in the loop.
A3 onward makes substrates long-lived, networked, unattended deploy targets,
every one of which accepts a deploy from anyone who can complete a handshake.
Shipping multi-substrate placement while ownership remains unestablishable means
shipping a deployment system whose authorization model cannot be turned on.

### Scope: three changes that only work together

B7 pairs two items with the tool and argues all three "become coherent at once."
That is not a preference — **each one alone is either inert or a regression**,
so they ship as one slice:

1. **The `roymctl` verb to create and sign a `ControllerAgreement`.** The
   consuming side already exists (`setup_substrate_identity` reads
   `config.identity.agreement`); only the producing side is missing. *Alone:*
   ownership becomes establishable but nothing changes behaviourally, because
   neither gate below exists to benefit from it.
2. **Gate the `security` interface on `substrate/admin`** (B7 F3.1).
   `inject-kek`, `rotate-kek`, and `set-secret` were dispatched with **no
   capability check at all** before this slice — the standing
   `TODO(M04B/FDAE)` this replaced, whose named milestone had since closed
   without it being addressed, is gone; the gate now lives at
   [service.rs:268](../../../../crates/control_plane/src/service.rs#L268).
   Any verified caller could rotate the KEK that encrypts every service
   database on the node. *Alone:* gating it bricks the interface, since
   nobody can hold `substrate/admin` while no `ControllerAgreement` can be
   created — B7 rejected exactly this as "a functional regression, not a
   tightening."
3. **Reconsider F4's fail-closed default**, so an unowned substrate stops
   issuing `orchestrator/deploy` to every verified caller. *Alone:* bricks the
   substrate permanently — you could not deploy the thing that establishes
   ownership.

Chained, they work: (1) makes ownership establishable → `substrate/admin`
becomes holdable → (2)'s gate becomes meaningful → (3) can fail closed because a
bootstrap path now exists.

**Scope note.** This makes P0 larger than "a `roymctl` verb" — it is a
security-posture change to M04A's authorization surface, pulled into a
supervisor milestone because the supervisor is what makes the current posture
untenable. Taken deliberately: the alternative is shipping an intermediate state
that is broken or pointless.

**The tool is local and offline, by construction — constrains A3's
provisioning story.** Producing an agreement needs both the controller's and
the node's private keys; the node's key exists only on the node's own
filesystem, and no RPC returns a signature over caller-supplied bytes. So
`roymctl substrate claim` runs on the substrate host, and there is no remote
claim. Provisioning N substrates for A3 means visiting N hosts (or shipping
`agreement.json` out of band) — acceptable for P0, but A3's substrate-inventory
design must not assume an operator can claim a substrate they have no shell on.
Tracked as a backlog row (`deferred-backlog.md` §3), not a redesign.

**Not pulled from M5 item 5's bundle:** multiple substrate owners (F12) — a
single owner per substrate is sufficient here — and Tier 1 for the five data
native-capability interfaces (F3), unrelated to placement.

The bundle's remaining item, the registry-trust-model ADR, is **discharged** by
[ADR-0020 §6](../../../decisions/0020-stable-logical-service-identity.md) plus
Slice A1 (see A1), not scheduled here.

### A0 — Stable member identity — **Complete (2026-07-28)**
Design of record: [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1-§5
([amended](../../../decisions/0020-stable-logical-service-identity.md#amendment-2026-07-28-after-slice-a0-implementation)
2026-07-28 against what actually shipped). Implementation plan:
[slice-a0-implementation-plan.md](slice-a0-implementation-plan.md). Verification
evidence: [status.md](status.md)'s A0 section.

Master keypair per **member** of a `LogicalServiceRef` (one for a `Singleton`,
N for an N-member redundant or sharded service), minted by the deployer — the
`<dir>/identities/*.key` storage is reused, but minting needed a new verb,
`roymctl identity certify-instance`, since the existing `identity delegate`
requires `--temp-did`, which the operator does not have until the substrate
reports it; instance keys generated on the hosting substrate and certified with
a `DelegationCertificate` under a distinct service-instance scope; deploy issues
and installs the certificate. **Renewal**: this slice ships the **attended
posture only** (an operator re-runs `certify-instance` on their own cadence,
with a near-expiry warning on the heartbeat sweep and an expiry column on `svc
list`); the **online-key posture** (unattended renewal, needs a component
holding member master keys) is A5's, per ADR-0020 §3.

**Includes ingress-side `scope` enforcement**, which did not exist before this
slice — `scope` was signed but never read by `DelegationCertificate::verify` or
the ingress verifier (`HandshakeVerifier::verify_preamble`), so a `"routing"`
certificate was accepted where a service-instance one belonged. The verifier now
compares the presented scope against a caller-supplied accepted set and rejects
a mismatch.

*Independently mergeable* — identity/deploy work, valuable on its own before any
supervisor exists.

### A1 — Endpoint records under the member master DID — **Complete (2026-07-28, design revised 2026-07-29 before merge)**
Design of record: [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
§6. Implementation plan:
[slice-a1-implementation-plan.md](slice-a1-implementation-plan.md) (see its
own note on a fifth review pass that reopened the design before merge —
§3/§5's code and test listings describe an earlier, superseded shape; §0/§1's
decisions describe what shipped). Verification evidence:
[status.md](status.md)'s A1 section.

**Added after review found the mapping this design assumed does not exist.**
The registry today verifies an endpoint record's signature against the key
resolved from the `service_id` it is keyed under
([registry.rs:234](../../../../crates/community_registry/src/registry.rs#L234)),
so an instance key cannot publish under its master; and `MasterAnchorPayload` is
a revocation list with no forward index. Without this slice, a dependent holding
a master DID cannot resolve it to an address at all, and relocation silently
stops working — the exact failure A0 exists to prevent.

**The record is signed by the member master key directly, by whoever holds it
— the deployer — never by the hosting substrate.** The substrate stores the
finished, self-verifying `EndpointInfo` it receives at deploy time and
replays those exact bytes on every heartbeat; it never builds or signs one
itself. Verification is the ordinary self-signed check, applied uniformly:
the key `service_id` resolves to must be the key that signed the packet.
There is no certificate on the record and nothing to revoke there — an
earlier version of this design had the substrate sign with a delegated
instance key, carrying a certificate binding it to the master, but that
shape had no DHT home (pkarr keys a packet by its signing key, and the
signer and the record's key did not match) and needed real complexity to
check the certificate's expiry and revocation. Master-DID resolution now
works on the DHT exactly as any other record's does.

A monotonic pkarr/BEP44 timestamp, enforced identically at the DHT and the
HTTP registry (a strictly newer record always displaces an older one; an
equal, byte-identical one refreshes rather than conflicts), plus a generous
`not_after` freshness bound on the record itself, together mean a substrate a
member has relocated away from cannot keep its stale mapping alive by
replaying its last blob forever.

**One verification function, not two — corrected after planning found the
mapping this design assumed does not exist.** `verify_endpoint_signature`
([registry.rs:234](../../../../crates/community_registry/src/registry.rs#L234))
*calls* `SignedEndpointInfo::verify`; its own body only ever resolved a key for
a debug log a single caller discarded. The genuinely separate path is
`RegistryClient::lookup`'s **DHT branch**
([dht_registry.rs](../../../../crates/core/src/dht_registry.rs)), which never
called `verify` at all. Both now share the identical single-keying-shape
check.

Covers every `ServiceType`'s *key derivation* with no special case — instance
keys (still used for outbound-call authentication and FDAE, unrelated now to
the endpoint record) are HKDF-derived from the hosting node's identity
([keys.rs:240-257](../../../../crates/identity/src/keys.rs#L240)) rather than
stored, so a TCP or container service has one exactly as a WASM service does.
The *surface* is narrower: `SyneroymClient::deploy_container` and `roymctl svc
deploy` have no way to carry a member master for a container service today, so
a container-hosted member cannot yet get a master-keyed record (backlog row,
retargeted off this slice — it needs a container-deploy CLI surface first).

**Discharges the deferred registry-trust-model ADR** that M04A Slice B7 recorded
as owed (§6.2 / F9 option 2: relax `verify()` to accept a record for X signed by
someone other than X, carrying proof of authorization). B7 gated that work on
finding "a real consumer" — this slice is it. The mechanism is simpler than
B7's own sketch: the record is signed by the owner's own key, so the
signature *is* the authorization proof, with no separate credential or
owner-store lookup needed.

**Sequences before A2.** Depends on A0.

### A2 — Host-side dependency resolution — **Complete (2026-07-29)**
Design of record: [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md) §2.
Implementation plan:
[slice-a2-implementation-plan.md](slice-a2-implementation-plan.md). Verification
evidence: [status.md](status.md)'s A2 section.

The guest names a **declared dependency**, not a `LogicalServiceRef` — the host
supplies `app_instance_id` from its own `HostState` and resolves through
`LogicalResolver` before constructing the `ProxyRequest`, instead of the guest
supplying a DID
([host_capabilities.rs:1110-1111](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1110)).
Naming a `LogicalServiceRef` directly would let a guest address an arbitrary app
instance, which contradicts the least-privilege property this design claims.
Raw-DID targeting stays as a second variant for its existing non-dependency
callers: guest self-proxy, native dispatch, and external/`roymctl` callers.

Binding entries carry `{member_master_did, expected_asserter_did}` per member,
closing D-B3-8's publication gap. `StaticInventory` gains its first real
`.register()` callers.

**Also in A2, found while planning A0: make `expected_asserter_did` survive
reinstantiation.** A service signs its `RelationshipProof` with its *instance*
key — `asserter_did` comes from `derive_service_identity(owner_did, service_id)`
([synsvc_native.rs](../../../../crates/control_plane/src/synsvc_native.rs),
[relationship_proof.rs](../../../../crates/rpc/src/relationship_proof.rs)) — and
the fetching side requires **exact equality** with the policy-declared
`expected_asserter_did`. So a member reinstantiated on another node, or
redeployed by a different owner, silently stops satisfying every policy and
every binding that names it: the exact failure ADR-0020 exists to prevent, on a
credential path that ADR never mentions. Republishing on each reinstantiation
is not available — the reference scenario's step 4 requires that reinstantiating
a member propagates *nothing*. So A2 declares `expected_asserter_did` as the
**member master** DID and teaches `RelationshipProof::verify` to accept an
instance-key signature carrying a `DelegationCertificate` from that master —
the same accept-an-instance-key-plus-certificate trust chain the handshake
already walks on every inbound connection (A0), applied at a third site;
load-bearing: without it A2 publishes bindings that go stale on the first
restart.

Also here: `ProxyRouter::invoke_remote_at`'s `CallOrigin::Native` arm still
presents the *node's* key on the wire (A0 changes only the guest-origin arm),
which is the transport half of the same gap.

### A3 — Multi-substrate placement and the substrate inventory — **Complete (2026-07-30)**
Design of record: [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§1/§5, [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
§1/§3/§6. Implementation plan:
[slice-a3-implementation-plan.md](slice-a3-implementation-plan.md). Verification
evidence: [status.md](status.md)'s A3 section.

Placement selector in the manifest (v1: a global default plus per-service
override, by **alias**, never a bare DID); a substrate inventory holding
alias → DID, reachability, capabilities, and the deploy capability held on each;
resolved `substrate` recorded on `PlannedService`; per-(service, substrate)
journal action records; partial-failure semantics (no auto-rollback, mark
`Degraded`, keep retrying).

**Dated correction (2026-07-30):** the implementation plan's §0 found fifteen
places this paragraph understated or left open (multi-substrate identity
minting, the journal never having a writer, the credential-per-substrate gap,
the two-publisher relocation hazard, the split-registry-namespace precondition,
among others) — see the plan's §0/§1 for the fourteen numbered decisions taken.
"Keep retrying" means **a manual re-run** through A4 (§0.14): nothing retries on
its own until A5's loop exists. Failure-matrix row 10 (lost-response dedup) is
**not** met by A3 — recorded as A5's in `deferred-backlog.md`. §9's test 6 (a
WASM-guest two-substrate dependency call, which would also discharge two
outstanding coverage rows from A0 and A2) was **declined**, not built — sized
correctly in the plan as the largest single item in the slice, and judged out
of proportion to add in the same pass as the rest of A3's five two-substrate
e2e tests. Both backlog rows it would have discharged are updated to say A3
declined it too, with the same reasoning.

### A4 — Health, read-only — **Complete (2026-07-31)**
Design of record: [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§7/§8. Implementation plan:
[slice-a4-implementation-plan.md](slice-a4-implementation-plan.md). Verification
evidence: [status.md](status.md)'s A4 section.

Health-check declaration in `ServiceConfig` (absent today); a substrate-side
per-instance status query; the supervisor's poll loop; three signals kept
distinct because remediation differs per signal — substrate unreachable, instance
not running, author-declared readiness probe failing. Alert events emitted and
queryable. **No remediation yet**: watch the signal before acting on it.

**Dated correction (2026-07-31):** the implementation plan's §0 found thirteen
places this paragraph left a decision unmade, described a component that does
not exist yet, or understated the work (six of them scope-changing) — see the
plan's §0/§1 for the full numbered list. Two matter most. **"The supervisor's
poll loop" does not exist**: A5 is the slice that introduces the substrate
role and the `supervisor` interface, so A4's sweep is a library function
(`crates/sdk::health::poll_once`/`record_report`), driven one-shot by `roymctl
app health`/`app alerts`, the same shape A3 took for `apply_plan` (D-A4-1,
D-A4-2). And **"alert events … queryable" has no read surface in A4 either**:
`roymctl app alerts` reading a local `AlertStore` stands in for it; MQTT
publication moves to A5, since only a substrate role holds an in-process
broker (D-A4-10). Also: `task.md`'s single "instance not running" signal is
actually three distinct substrate-side truths (container, wasm, and
TCP/native-host, the last two having no deploy-time liveness signal at all)
that the substrate could not previously distinguish — fixed by an
`AppServiceType` recorded at deploy time (§0.5); and `deployed_service_id` (the
DID a sweep polls) inherits `check_no_placement_change`'s member-index-0
assumption, correct today but broken the moment A5 scales a service (D-A4-11,
backlog row).

### A5 — The supervisor loop

**Dated correction (2026-08-01), against the
[A5 implementation plan](slice-a5-implementation-plan.md) §0.1/D-A5-1.** A5's
original one-paragraph text (below, kept for the exit criteria and matrix
rows it still describes accurately) undercounted the work by an order of
magnitude — twelve workstreams, not one. It ships as **five sub-slices**,
each independently mergeable and each closing a named set of failure-matrix
rows:

- **A5a — substrate write primitives.** ✅ Complete. The binding-only write
  path (`write-bindings`, epoch-guarded on the four-case rule), `restart`,
  the `app_instance_management` table and generation gate, `generation` on
  `app-context`, deploy content-hash dedup, `claim-app-instance`/
  `release-app-instance`/`app-instance-management-of`, the `SubstrateActor`
  trait, and the `roymctl` lib split. No supervisor exists yet; every write
  is exercised directly against one substrate. Closes matrix rows 5, 6, 7, 8
  (substrate half), 10.
- **A5b — the role, the store, the interface, custody.** ✅ Complete. The
  `syneroym-app-supervisor` crate; `[roles.supervisor]`; `supervisor.db`; the
  `supervisor` WIT interface (submit/adopt/release/pause/resume/retire/
  force-reconcile/status/alerts/export-master/import-master); mint-in-place
  master custody in the supervisor's own vault (moved here from where this
  paragraph originally placed it, because an operator-minted certificate is
  rejected the moment the supervisor itself deploys — see ADR-0020's
  2026-08-01 amendment); `roymctl supervisor …`. `status` sweeps on demand
  (D-A5-21) — there is no resident loop in this slice, so "the supervisor
  loop" this section is titled for does not exist until A5c. Closes matrix
  row 9 (supervisor half); the exit criterion "an operator can read health,
  alerts, and per-dependent binding convergence" (binding convergence itself
  is not yet populated — A5b writes no bindings on its own, so there is
  nothing to compare a substrate's observed epoch against until A5c's push
  exists).
- **A5c — the loop and remediation.** ✅ Complete (2026-08-02, plan Part IV,
  phases 1-7). The resident reconcile loop (spawned, `MissedTickBehavior::
  Skip`, joined on shutdown, D-A5c-8), the per-instance async lock
  (D-A5c-7), bounded restart with backoff/max-attempts/terminal `Degraded`
  (D-A5c-20), `OrphanedService` alerting on a plan-level removal without
  undeploying it (D-A5c-3/D-A5c-21), the binding push with its epoch
  bookkeeping and convergence read (D-A5c-4/D-A5c-5/D-A5c-19, exercised by
  a fixture per D-A5c-16), MQTT alert publication (D-A5c-6), and the
  health-poll-cost budget measured. Closes matrix rows 11, 12, 13. Note on
  §19.18's own trigger gap: matrix row 12's e2e (test 48,
  `supervisor_loop_e2e.rs`) needed `submit` reordered to persist desired
  state before its own best-effort deploy attempt, and the loop's filtered-
  plan apply narrowed to only the substrates it actually connected to this
  pass (`resolve_targets` otherwise fails an entire filtered plan closed
  over one alias it has no target for) — neither was named in the `§0`
  pass, both surfaced only once the loop and a genuine two-substrate
  partial-failure e2e existed to expose them.

  **Dated correction (2026-08-01), against the A5 implementation plan's
  Part IV.** The `§0` pass found **eighteen** items this bullet and the
  plan's own §14 leave open, understate, or state wrongly; **nine** change
  what A5c has to build. Four matter most.

  1. **The supervisor has no placement-change refusal, and §14 said it
     did.** `check_no_placement_change` is private to `roymctl` and reads
     the operator's `--dir/identities/`; the supervisor never calls it.
     So a re-`submit` that moves a service is **silently applied today**,
     leaving two live copies of one member master — the milestone doing
     the thing its own non-goals forbid. An A5b defect, fixed in A5c
     (D-A5c-1); matrix row 20 does not cover it, since it settles name
     resolution only and the old copy keeps running.
  2. **A partially-deployed app reports `Active`, not `Degraded`** —
     `Signal::NotDeployed` is deliberately not a fault (D-A4-19, correct
     for a poll), and the supervisor's `status` derives its state from
     faults alone. So **matrix row 12's read-surface half is unmet
     today**. The supervisor has the plan and the poll does not; A5c adds
     that knowledge rather than changing what a fault means (D-A5c-10).
  3. **A5c has no reachable membership change of its own**, so the binding
     push in this bullet has no in-slice trigger: one member per service
     until A5e's `replicas`, `get_or_mint` returns the same master on
     re-submit, and `write-bindings` refuses an undeclared dependency by
     design. A5c builds the push, its epoch bookkeeping, and the
     convergence read; the trigger is exercised by a fixture, and the real
     one arrives in A5e (D-A5c-16).
  4. **The binding epoch has no owner**, and the supervisor's first push
     would be classified a `Conflict` against its own deploy: bindings are
     emitted at a hardcoded epoch 0 and the initial deploy writes the
     per-dependent row unguarded, so an equal-epoch push with changed
     members is a conflict by the four-case rule. The supervisor takes
     ownership of the epoch, holding one counter **per dependent service**
     that always advances before a write; `0` comes to mean "no supervisor
     has written here", which the operator's own `roymctl app deploy` keeps
     (D-A5c-4).

  Also decided rather than deferred: `ProbeFailing` is **alert only**
  (§18 q8 — `HealthCheck` has no initial delay and no failure threshold,
  so a slow-starting service would be restarted until it hit the attempt
  ceiling; and `restart` is meaningful for `container` only among the four
  service types); and a changed-placement re-submit is a **permanent
  refusal the loop never retries** (§18 q9 — the reviewer's answer
  confirmed, with item 1 above as the correction to what it costs).
- **A5d — unattended renewal.** ✅ Complete (2026-08-03, plan Part V,
  phases 1-5). Renewal on the loop as a fourth work-list inside the existing
  pass (D-A5d-5/D-A5d-11), `RotationPolicy` finally read (D-A5d-6), the
  `SynSvcNativeService` rebuild (D-A5d-3), a 30-day maximum certificate
  lifetime enforced at install (D-A5d-7), master-anchor refresh on the
  existing tick against a persisted fact (D-A5d-8), and the revocation
  surface (`roymctl supervisor revoke-instance`, D-A5d-10/D-A5d-15). Closes
  matrix rows 1/3's automation half and row 14b.

  **Correction to what this bullet used to say.** It read as though renewal
  could "reissue via `certify_instance`" and be done. Minting is half the
  job: **no verb in the tree installed a certificate without reinstalling
  the whole service**, so A5d had to add one — `renew-cert`, sized like
  `restart` (an in-place lifecycle action, not a variant of a reinstall) and
  gated identically. Routing renewal through `deploy` instead would have
  resent the whole inlined artifact on every renewal cycle, reopening the
  exact per-pass churn §19.10 fixed once already. Three of the six items
  this bullet listed — renewal itself, `RotationPolicy`, and the
  `SynSvcNativeService` refresh — were all downstream of that one missing
  verb.

  Also corrected: **row 14's "second half" was two properties wearing one
  label**, and A5a had already shipped one of them. The row is split above
  into 14a (a *superseded* supervisor, refused at the substrate — A5a) and
  14b (a *compromised* one, revoked by an operator — A5d). Neither subsumes
  the other.

  Two things the plan decided rather than deferred, both worth knowing
  operationally. **The 30-day certificate cap is a backstop, not a forcing
  function**: a tight cap would contradict ADR-0020 §3's attended posture,
  whose certificates are deliberately long-lived, so the ceiling is generous
  enough to catch an unbounded mint and nothing else. And **cutting the
  supervisor's own certificate lifetime to 4 hours also cuts the
  post-restart grace window**: the vault is locked after every restart, so
  the time to run `inject-kek` before managed members fail closed is now
  between roughly 1 and 4 hours rather than 24. That is what the online-key
  posture is for and the 4-hour default stands, but it promotes the new
  `VaultLocked` alert from honest reporting to the single control between a
  routine restart and an outage. Recorded in the developer guide in the
  operator's own terms.
- **A5e — scale-out, budgets.** ✅ Complete (2026-08-04, plan Part VI,
  phases 1-6). **Rescoped from the seven-item sketch this bullet originally
  named** — see the plan's §33.1/D-A5e-1: the cross-app `Bind` manifest
  surface and the ADR-0021 §7 probe left the milestone on 2026-08-02, into
  ADR-0022's Logical Service Discovery Overlay as slice **S4**, which needs
  S0-S2 first and is at minimum three slices past this milestone. Building
  the naming surface here would not have made matrix rows 15/18 testable
  regardless — it is one of four missing pieces, the other three (the
  compiler resolving `Bind` at all, the substrate's intra-app placement
  refusal, and S2's topology document) sitting outside A5e's reach too.
  `replicas` turned out not to be a compiler feature but **the key change of
  the slice** (§33.2): the `MemberRef { logical_ref, index }` type replaces a
  bare `LogicalServiceRef` string everywhere a stored, reported, or wire-level
  fact identifies a managed unit — four `SupervisorStore` tables, the alert
  index, the deployment journal's action rows, every member-naming field of
  the `supervisor` interface, and the binding epoch's own wire assembly.
  Ships: the manifest's `ServiceSpec.replicas` (capped at 16, refused
  alongside a declared `schema` until M7's state replication lands);
  `replicas > 1` compiling to `TopologyMode::Redundant`; the loop's
  membership-change classifier giving `push_bindings` its first production
  caller, so a scale-out reaches every dependent member with a push, not a
  redeploy; `Degraded` derived from an active `BindingConflict`, with the
  clear site it never had; the `LogicalResolver::resolve` Criterion bench
  (`crates/app_orchestration/benches/resolver.rs`, not
  `crates/router/benches/proxy.rs` as the backlog row's own text expected —
  `ProxyRouter` has no dependency target); and the convergence budget and
  ADR-0021 §6 trigger, measured and written down. Closes matrix rows 5, 6, 7
  (live scale-out evidence), 11 (now reachable, not just unit-proven); rows
  15/18 move to **S4** rather than close here (D-A5e-1) — see the exit
  criteria's explicit exception below.

**Milestone closes when A5e and A7 have both landed**, whichever lands
second flipping `[LFC-MGT]`/`[FND-IDT]` to Complete in the traceability
matrix (§41 answer 1 of the A5e plan's Part VI) — corrected from this
section's original "closes at the end of A5e," written before A7 was pulled
forward into the milestone (2026-08-02).

---

**Original text**, describing the whole of A5 rather than any one sub-slice:

Substrate role; the `supervisor` interface, carrying both the write path (submit
desired state, retire, pause, force reconcile, **adopt**) and the operator-facing
read surface (status, alerts, per-dependent binding convergence) that the exit
criteria require; desired state persisted and designed to be rebuildable from
manifests plus a substrate sweep; reconcile loop over the shared
`app_orchestration` compiler (an effectful adapter, not a second planner);
epoch-guarded binding writes on the four-case rule (lower rejects, equal+same
content is an idempotent no-op, equal+different content is a reported conflict,
higher applies); owner+generation stamp where **the generation is minted by the
operator's `adopt` action, never self-incremented** — a supervisor that finds a
higher generation stops managing that instance and alerts;
bounded restart-in-place remediation with backoff, max attempts, and a terminal
`Degraded` state that only alerts; probing of bound external dependencies
(ADR-0021 §7). Delivery is **best-effort synchronous** behind a narrow "apply
this action to that substrate" trait, and the supervisor's status output says so
rather than implying convergence it cannot guarantee.

**Plus the online-key posture, which nothing before A5 can carry.** A0 ships the
attended posture only: an operator issues instance certificates on their own
cadence, and A0's contribution to a missed cadence is visibility (a near-expiry
warning on the heartbeat sweep, an expiry column on `svc list`), not automation.
It cannot be otherwise — automatic renewal needs a component holding member
master keys that runs unattended, and putting a renewal timer on the substrate
would put the master key there, which ADR-0020 §3 forbids outright. So A5 owns
three things A0 deliberately left out:

- **Custody**: member master keys held in the supervisor's own substrate vault
  rather than on disk beside a config (ADR-0020 §4). A0 leaves them as ordinary
  `roymctl` identities under a computable name, so adoption is a read, not a
  scan. **✅ A5b**: mint-in-place, not the "adoption is a read" shape this
  paragraph describes — see the sub-slice list above and ADR-0020's
  2026-08-01 amendment for why.
- **Unattended issue and renewal**, so relocation and renewal both just work
  (ADR-0020 §3's online-key posture) — and with it the operator's choice between
  the two postures becomes real rather than nominal. **A5d**.
- **`RotationPolicy`** (`models.rs`) finally becomes load-bearing: with renewal
  automated, whether a service tolerates in-place certificate replacement or
  needs a restart is a decision something actually makes. A0 does not use it
  because operator-initiated replacement is always in-place. **A5d**.

Until A5d lands, **a missed renewal cadence is an outage** (matrix row 3), and
that is the milestone's standing operating cost through A0–A5c.

### A6 — Durable delivery *(post-M5 item 1 — do not start before it lands)*
Replace the A5 trait's implementation with an outbox/DLQ-backed one: durable push
delivery, retry against substrates that are offline at the time of the change,
terminal-failure handling, and a single-writer cron lease if redundant
supervisors are wanted. Nothing above the trait changes.

**Pickup trigger:** M5 item 1 (Outbox, DLQ, cron leases) marked Complete in the
[traceability matrix](../../traceability-matrix.md). Tracked in
[deferred-backlog.md](../../deferred-backlog.md) §8 *Node lifecycle & ops* so it
is not remembered by accident.

### A7 — App-instance master identity *(pulled forward, 2026-08-02)* — **Complete (2026-08-04)**

> **Numbered after A6 but not sequenced after it.** A7 depends only on A5b's
> vault custody and may land before, after, or alongside A5d/A5e. A6 is the
> only slice in this milestone gated on an external trigger.

Mint an **app-instance master DID** at `adopt`, hold it in the supervisor's
own vault beside the member masters, record it on the instance row, and
surface it on `status`. `export-master` / `import-master` cover it by name, so
supervisor handover moves it the same way it moves any other master.

Design of record:
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §1.
Slice **S0** of the [Logical Service Discovery Overlay](../../meta-implementation-plan.md#committed-work-logical-service-discovery-overlay-2026-08-02).

**Scope is identity and custody only.** No registry publication, no topology
document, no resolve RPC — those are S1 and S2, outside this milestone. What
lands here is the DID, where it lives, and how it moves.

**Why it is pulled forward rather than left with S1.** `adopt` is the natural
mint point and A5b already built the vault, the minting path, and the
generation counter. Doing it now is an addition to files that are already
open; doing it later is a second pass through `adopt`, the instance rows, and
vault naming, in a supervisor that has shipped. Nothing pre-release makes the
retrofit *hard* — it is simply wasted work.

**Why an app-level DID at all** (ADR-0022 §1, condensed): the supervisor has
no identity of its own — `status` reports the node's DID — so addressing an
app by its supervisor would change the app's address on every handover. The
app DID stays stable while the supervisor underneath changes, and it is the
natural subject for access grants, where a grant against each member DID
changes on every membership change.

`app_instance_id` is unchanged and stays the human name, in vault names, alert
topics, and `LogicalServiceRef`'s display form — the same alias/DID split the
substrate inventory already uses.

Implementation plan:
[slice-a7-implementation-plan.md](slice-a7-implementation-plan.md). Verification
evidence: [status.md](status.md)'s A7 section.

**Delivered as scoped.** `adopt` mints or resolves the app-instance master
(`app-<app-instance-id>`, `MasterVault::get_or_mint` now taking a
`MasterKind` so its mint warning names the right noun) before claiming any
generation, and records the DID on the instance row after the claim
succeeds — resolve-and-record on *every* call, not mint-once, so an
`import-master`/`adopt` handover always leaves the row agreeing with the
vault (D-A7-5). `instance-status` reports `app-master-did: option<string>`
from the stored row alone, readable through a locked vault; `adopt`'s
result is now a record (`generation`, `app-master-did`, `vault-name`) so an
operator learns the backup command the moment the key exists, matching
`submit`'s own `minted-master` rows. `export-master`/`import-master` carry
it under exactly that name, with no signature change. Every exit-criteria
bullet this slice owns is met — see §5 of the implementation plan and
`status.md`'s A7 evidence section for the full test list and gate
history.

---

## Migration impact

Pre-release, so changes are made in place with no compatibility shims
(the project's standing position, most recently applied in
[ADR-0019](../../../decisions/0019-deploy-time-artifact-delivery.md) §3).

- Manifests gain placement and health-check fields. Existing manifests keep
  working: absent placement means "the substrate you deployed to," absent health
  check means liveness only.
- The proxy target gains a dependency-name variant (A2). This is a WIT-visible
  change to the proxy interface and updates every consumer in the same change;
  raw-DID targeting is retained for its non-dependency callers.
- `verify_endpoint_signature` gains a second acceptance path (A1). Existing
  self-signed records are unaffected.
- **A pre-A0 service redeployed later *with* a member master gets a new
  authorization identity, and is orphaned from the rows it already created.**
  Services deployed before A0 keep the existing "service is its own master"
  fallback
  ([handshake.rs:88-90](../../../../crates/router/src/handshake.rs#L88)) and
  continue working untouched — but adopting a master is not a transparent
  upgrade for a service that already holds data. This is the same failure
  described in [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
  Context problem (2), and pre-release we take it rather than build a
  re-attribution path. Existing deployments that hold data should be recreated
  under a master rather than migrated.
- **A service deployed with an instance certificate now asserts `RelationshipProof`s
  under its member master, not its derived instance DID (A2).** A policy
  authored between A0 and A2 whose `expected_asserter_did` names the
  instance DID (the only shape available in that window) stops verifying
  the moment this slice merges; dependents must name the member master
  instead. A service deployed *without* an instance certificate is
  unaffected (self-asserted, unchanged). No compatibility shim: accepting
  either DID would defeat D-B3-8's single-trust-anchor guarantee.

---

## Reference scenario

A two-substrate app: `frontend` placed on substrate A, `backend` on substrate B,
`frontend` depending on `backend`. `backend` starts as a single member.

1. Operator submits the manifest to the supervisor; it deploys both, mints a
   master DID per **member**, and pushes `frontend`'s binding for `backend`.
2. `frontend` calls `backend` by its declared dependency name; the host resolves
   the name to `backend`'s member master DID, then that DID to an endpoint.
3. Substrate B is stopped. The supervisor's poll notices, distinguishes
   "substrate unreachable" from "service unhealthy", and alerts.
4. Substrate B returns; the supervisor restarts `backend` **as the same member**.
   Its instance key is new; **its member master DID is not**, so it republishes
   its endpoint record under the unchanged master, `frontend`'s binding is still
   valid, and `backend`'s own data and FDAE policy still recognize it.
   **No push, no membership change.**
5. Operator scales `backend` to two members. A *second* member master is minted,
   so the member set changes and **a push does happen**; `frontend` resolves
   across both from the next call, with no restart.
6. A stale retry carrying the pre-scale epoch arrives late and is rejected; the
   same write re-sent at the current epoch with identical content succeeds as a
   no-op.

Steps 4 and 5 are the milestone's real claim, and the difference between them is
the design: reinstantiating a member propagates nothing, while changing the
member set propagates correctly. Step 4 also exercises A1 — without a
master-signed endpoint record on file, the restarted member would have
nothing for the heartbeat sweep to replay, and step 2's second lookup would
fail.

---

## Failure / security matrix

| # | Case | Expected |
|---|---|---|
| 1 | Instance certificate expired | Handshake fails closed. **✅ A0**: already true of the general mechanism, and pinned specifically for a service-instance certificate by `an_expired_instance_certificate_fails_the_handshake_closed` (`crates/router/src/handshake.rs`). **✅ A5d** closes the automation half: the supervisor's resident loop reissues any managed member inside the last 25% of its certificate's lifetime and installs it with `renew-cert`, so under the online-key posture nothing has to expire (`a_pass_renews_a_member_within_the_near_expiry_window`, `crates/app_supervisor/src/service.rs`; the install itself proven live over the wire, refusal included, by `renew_cert_installs_over_the_real_wire_and_refuses_a_certificate_for_the_wrong_derived_key`, `crates/substrate/tests/cert_renewal_e2e.rs` -- a live handshake presenting the renewed certificate is not additionally exercised, see `status.md`'s own scoping note). Through A0-A4 renewal was an operator-run cadence (see row 3) |
| 2 | **Certificate presented with the wrong `scope`** | Rejected. **✅ A0, at two granularities**: the ingress (`handshake.rs`) admits either transport scope and rejects anything outside that set (`a_certificate_scoped_outside_transport_is_rejected_at_the_handshake`); the narrow single-value check — "this record/install may only be admitted by a `service-instance` certificate" — lives at the deploy-time install verification (`a_deploy_is_rejected_when_the_certificate_carries_the_routing_scope`, `crates/control_plane/src/service/orchestration.rs`) and, for A1, at endpoint-record admission. Proven live over two real substrates in `crates/substrate/tests/instance_identity_e2e.rs` (a `routing`-scoped certificate rejected at deploy) |
| 3 | **Attended posture, renewal cadence missed** | Instance fails closed; this is an outage, not a degradation, and the docs say so (ADR-0020 §3). **✅ A0** proves the handshake half: not a distinct code behavior there — the failure mode *is* row 1 — so the evidence is row 1's test plus the near-expiry heartbeat warning (`a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep`, `crates/substrate/src/runtime.rs`) and the `svc list` expiry column. **The instance certificate's own expiry is now unrelated to name resolution** (the endpoint record carries no certificate at all), so resolution has its own, independent freshness bound instead: `EndpointInfo.not_after`, checked uniformly whether admitting or reading a record — deliberately generous (30 days) so it is a backstop for a signer that stops renewing entirely, not a routine failure mode. **✅ A1**: `an_expired_record_is_rejected` (`crates/core/src/dht_registry.rs`) and `an_expired_stored_record_is_not_replayed` (`crates/core/src/endpoint_publisher.rs`). **✅ A5d** removes the cadence from the operator entirely under the online-key posture (see row 1), leaving one honest gap it does *not* close: the supervisor's vault is locked after a restart, so "unattended" means unattended between KEK injections. `AlertKind::VaultLocked` names that gap, per affected member, the moment a renewal is due and the vault is shut |
| 4 | **Endpoint record keyed by a master DID, signed by an unrelated key** | Rejected; a record must be self-signed by the key its own `service_id` resolves to. **✅ A1**: `a_record_is_rejected_when_signed_by_a_key_other_than_its_own_service_id` (`crates/core/src/dht_registry.rs`), proven live over two real substrates by the negative half of `a_member_master_did_resolves_to_an_address_and_follows_the_member_across_nodes` (`crates/substrate/tests/master_endpoint_record_e2e.rs`) — a hand-built record posted to the registry is rejected with `401` |
| 5 | Out-of-order binding write (stale epoch) | Rejected by the substrate; mapping does not regress. **✅ A5a**: `classify_binding_write`'s stale case (`crates/app_orchestration/src/resolver.rs`) |
| 6 | **Binding write re-sent at the current epoch, identical content** | Idempotent no-op, reported as success — distinct from the stale rejection above. **✅ A5a**: `classify_binding_write`'s no-op case |
| 7 | **Binding write at the current epoch, *different* content** | Rejected as a conflict, reported distinctly; this is the two-writer signal. **✅ A5a**: `classify_binding_write`'s conflict case |
| 8 | **A second supervisor that has not adopted the instance issues a write** | Its presented generation is lower than the one an adopting supervisor holds; rejected, no flapping. (Corrected wording, §0.10: the original "second supervisor adopts" describes a supervisor that *did* adopt, which correctly wins with a *higher* generation — this row is about one that never did.) **✅ A5a substrate mechanism** (`check_generation`'s `Ordering::Less` case, `crates/control_plane/src/service/orchestration.rs`); **✅ A5b live proof**: `a_second_supervisor_that_has_not_adopted_loses_every_write` (`crates/substrate/tests/supervisor_interface_e2e.rs`) |
| 9 | **Supervisor finds a *higher* generation than its own** | Stops managing that instance and alerts; never self-increments (ADR-0021 §4). **✅ A5b substrate mechanism**: `SupervisorService::handle_status` reads the max held generation across every substrate the *plan* places a service on (not only ones this supervisor's own journal already shows landed) on every sweep and raises `AlertKind::SupervisorSuperseded` when it exceeds the supervisor's own stored generation (`crates/app_supervisor/src/service.rs`); **✅ A5b live proof**: `a_supervisor_that_reads_a_higher_generation_marks_the_instance_superseded_and_alerts` (`crates/substrate/tests/supervisor_interface_e2e.rs`) |
| 10 | Deploy retried after a lost response | Idempotent no-op for identical (instance, service, content hash). **✅ A5a**: `deploy_with_context` hashes `(manifest, app_context-minus-generation)` with blake3 and short-circuits to `Ok(())` on an identical redeploy, distinct from the epoch guard and the generation gate (ADR-0021 §3). **Post-review correction (2026-08-02, finding E-1):** the row was credited complete against a test that leaves `instance_certificate`/`registry_certificate` `None` on every call. Both are minted fresh (a new signature, a `SystemTime::now()`-derived expiry) on every real apply through *either* deploy path (`roymctl app deploy` or the supervisor), so hashing them raw made the hash differ every time regardless of content -- the no-op branch was unreachable outside a test that never populates them. Fixed: the hash now covers each certificate's stable identity fields (`master_did`/`temporary_did`/`scope` for the instance certificate; `service_id`/`substrate_id`/`endpoint_type`/`mechanisms`/`nickname`/`is_private`/`ttl` for the registry certificate), not the freshness-bearing ones, with a regression test (`an_identical_redeploy_with_freshly_minted_certificates_is_still_a_no_op`) reproducing two independently-issued, byte-different certificates for the same member. |
| 11 | Dependent unreachable during a push | Retried; a `BindingConflict` alert raised and visible on the operator read surface, and the instance reports `Degraded` while it is active. **✅ A5c** built the mechanism at unit scale (test 47, a fake `SubstrateActor` whose `write_bindings` fails on one pass and succeeds on the next); **✅ A5e makes it live** (D-A5e-7/D-A5e-8, this row **rewritten rather than annotated** since A5e falsifies both halves the previous wording relied on). The loop's own membership-change classifier is `push_bindings`'s first production caller — a scale-out is the trigger, exercised end to end by `reference_scenario_e2e.rs`. `InstanceStatus.state` now does turn `Degraded` from the active `BindingConflict` set (derived, no new store column), and clears the moment a retried push lands cleanly — the raise/clear pairing every other `AlertKind` already had, closed here (§33.19/§33.21) along with a correction to what the raise sites wrote into the alert's `substrate_did` column (a `SubstrateAlias` before this slice, the real DID after). Unit tests 64-67, 75-78, 80 cover the classifier, the clear/`Degraded` pairing, and the corrected column; the e2e proves the sequence live. |
| 12 | Partial app deploy (3 of 5 services) | No rollback; `Degraded`; failed services retried. **✅ A5c**, tests 10, 11 (unit) and **48 (e2e)** — two real nodes, one stopped at submit time; `svc-a` lands, `svc-b` stays `Degraded`/missing, and the loop retries only `svc-b` once its node returns, with `svc-a` never rolled back. `overall_state` derives `Degraded` from "planned but never landed" (D-A5c-10) rather than widening what a fault means. Building the e2e also surfaced two gaps not named by the `§0` pass: `handle_submit` reordered to persist desired state before its own best-effort deploy attempt, and the loop's filtered plan narrowed to only the substrates it actually connected to this pass (see the A5c bullet above) — both needed for a plan spanning two substrates, one of them down, to land what it can rather than nothing. |
| 13 | Remediation exceeds max attempts | Terminal `Degraded`; alerting only; no restart loop. **✅ A5c**, tests **37, 38** (unit). Reached only from `InstanceNotRunning`: `ProbeFailing` is alert-only and never consumes an attempt (D-A5c-17), and `SubstrateUnreachable` never did (D-A4-13). The terminal flag is escapable — `force-reconcile` and `adopt` clear it (D-A5c-20), since a service nothing will restart cannot clear it by becoming healthy |
| 14a | Supervisor holds master keys and is **superseded** | Its lifecycle actions are refused at the substrate, so a stale supervisor cannot act on instances it no longer manages. **✅ A5a**: `restart`/`undeploy`/`write-bindings`/`deploy` all carry the generation gate (§0.23), proven by `undeploy_is_rejected_at_a_lower_generation` (`crates/control_plane/src/service/orchestration.rs`) — the most destructive lifecycle action there is. **✅ A5d** extends the same gate to `renew-cert` (`renew_cert_respects_the_same_generation_gate_as_restart`) |
| 14b | Supervisor holds master keys and is **compromised** | Blast radius bounded to the members it manages; instance certificates short-lived and revocable (ADR-0020 §3). **✅ A0** proved the property at unit scale against an in-memory mock: an instance certificate is revocable without touching the member master, and a fresh instance key from the same master still verifies (`a_revoked_instance_key_is_rejected_while_the_member_master_still_certifies_a_new_one`, `crates/router/src/handshake.rs`). **✅ A5d** supplies what that mechanism never had — a production writer and an operator surface: `roymctl supervisor revoke-instance` (`RegistryClient::revoke_instance_key`, D-A5d-10/D-A5d-15), proven end to end against a real registry and the real ingress check by `a_revoked_instance_key_handshake_fails_while_a_fresh_one_verifies` (`crates/substrate/tests/cert_renewal_e2e.rs`). "Short-lived" is now real too: the supervisor mints every certificate at `renewed_cert_expires_hours` (4h) and renews unattended. **✅ A5e** re-keys `revoke-instance`'s own argument from a bare `logical-ref` to a `MemberRef` string (`<app_instance_id>/<service_name>#<index>`, D-A5e-2) — with `replicas`, the argument is now scoped to **one member**, not automatically the whole logical service, and revoking one member's key leaves its siblings renewable and recertified by the next `submit` (test 62). **Note these are two distinct properties, not one:** a generation gate does nothing about a supervisor that is still the current manager and has been compromised, and revocation does nothing about a stale supervisor that was never compromised |
| 15 | Bound cross-app dependency replaced, **online-key posture** | Active probe fails on a call the supervisor makes *as the depending member*; A's owner alerted (ADR-0021 §7). **Moved to slice S4** of the Logical Service Discovery Overlay (ADR-0022, D-A5e-1), not A5e: reading the code (A5e §33.1) found the cross-app manifest surface is only one of **four** missing pieces — the compiler does not resolve `Bind` at all, the substrate refuses any intra-app-scoped binding write across app instances, A's supervisor has no directory to learn B's member set through, and the probe's posture split has nothing to report without one. All four sit behind S0-S2, which are themselves post-milestone, so A5e does not depend on S4 and S4 does not depend on A5e. **Named exception to this milestone's own exit criterion** "every row of the failure/security matrix has a test": rows 15 and 18 are the two that do not get one here |
| 16 | **`security` call (`inject-kek`/`rotate-kek`/`set-secret`) without `substrate/admin`** | Rejected. **✅ P0**: `security_is_denied_without_substrate_admin` (`crates/control_plane/src/service.rs`) |
| 17 | **Deploy to an unowned substrate once F4 flips** | Rejected; the bootstrap path becomes establishing ownership with the P0 tool, not an open deploy grant. **✅ P0**: `an_unowned_substrate_grants_no_node_wide_capability` (`crates/control_plane/src/service.rs`) |
| 18 | Bound cross-app dependency replaced, **attended posture** | No master key, so no active probe: detection is passive, on the first real call that fails. Weaker by design, and the operator chose it. **Moved to S4 along with row 15** — same four prerequisites, none of them A5e's to build |
| 19 | **Member reinstantiated under a policy declaring `expected_asserter_did`** | Its cross-service `RelationshipProof` still verifies. **✅ A2**: `RelationshipProof` carries an optional `delegation`; `sign`/`verify` assert and check under the certificate's master when one is installed (`a_proof_signed_by_an_instance_key_with_a_certificate_verifies_against_the_master`, `crates/rpc/src/relationship_proof.rs`). The transport half (`ProxyRouter::invoke_remote_at`'s `(None, Native)` arm) proven in `crates/router/src/proxy.rs` |
| 20 | **A substrate a member has relocated away from keeps replaying its old endpoint record on its heartbeat** | Rejected, at both the DHT and the HTTP registry: a record's pkarr/BEP44 timestamp must be strictly newer than what is stored to be admitted, or equal and byte-identical (the routine heartbeat replay, admitted as a refresh, not a conflict). Since a substrate that cannot re-sign a member's record can only ever replay the same frozen bytes at the same timestamp, once the master has signed one newer record for the new placement, the old substrate's replay is permanently stale and rejected everywhere. **✅ A1**: `verify_returns_the_packets_own_timestamp` (`crates/core/src/dht_registry.rs`) and the registry-side compare-and-swap proven live in `publish_all_services_survives_a_record_rejected_by_admission` (`crates/community_registry/src/registry.rs`) |

---

## Performance budgets

- **Binding convergence — measured at A5e sign-off (2026-08-03).** The
  provisional target was: all reachable dependents converged within 5 s, and
  any dependent that was unreachable converged within one poll interval of
  becoming reachable. **Measured against `replicas > 1`**, the first slice
  where more than one dependent member exists to converge: a `submit`-driven
  membership change's clock (RPC received → every reachable dependent
  member's `write-bindings` returning `Applied`/`NoOp`) lands in
  microseconds against a fake substrate answering immediately — the write
  itself, not a poll, and inside budget by three orders of magnitude. A
  loop-discovered change adds up to one `poll_interval_secs` (default 30s)
  before the first push is attempted, which is the second clause's own
  bound and not a miss of the first. The second clause has **two** causes,
  not one — `poll_interval_secs`, and the absence of durable delivery (A6,
  "after M5") to hold a push queued while the dependent was unreachable —
  and a pull-side directory would address neither, so **ADR-0021 §6's
  trigger has not fired**; see that ADR's own 2026-08-03 amendment and
  ADR-0022 §11, which reaches the same conclusion independently for
  callers *outside* the app instance. `roymctl supervisor status`'s
  `bindings` array is a third, slower number — the operator-facing
  confirmation, lagging a landed push by up to `poll_interval_secs` — kept
  distinct from the write's own latency above precisely so a slow read
  surface is never mistaken for a slow push (D-A5e-9). Full reasoning and
  the harness: [slice-a5-implementation-plan.md](slice-a5-implementation-plan.md)
  Part VI §33.9-§33.10; operator-facing numbers:
  [developer-guide.md](../../../developer-guide.md)'s "Scaling a service"
  section.
- **Health poll cost:** the supervisor's steady-state poll must not be a
  meaningful load on a target substrate at the intended inventory size.
  **Made a number, a priori, at the A5c `§0` pass (D-A5c-12)**, for the same
  reason the convergence budget above is a guess on purpose — a budget
  derived from the first measurement can never fail. One pass over a
  **20-service instance completes in under 2 s** and issues **at most 2 RPCs
  to that substrate** (one batched `status`, one
  `app-instance-management-of`); serving it costs the target **under 5% of
  one core averaged over the poll interval**. The number at risk is wasm:
  `probe_cached`'s 5 s minimum against a 30 s default poll interval means
  every sweep misses the cache and pays one component instantiation per
  `rpc`-probed wasm service. Measured **before** A5c's loop is written, so
  `poll_interval_secs`' default is chosen from the result.
- **Resolution adds no network hop.** A2 moves dependency resolution host-side;
  the name → master-DID step must stay an in-process cache lookup. The
  master-DID → endpoint step is the registry lookup that already happens today
  (A1 keeps it at exactly one, which is why ADR-0020 §6 chose a directly
  master-signed record over a two-hop anchor index). **✅ A5e** gives this
  budget its Criterion case (D-A5e-13): `crates/app_orchestration/benches/
  resolver.rs`, not `crates/router/benches/proxy.rs` as the backlog row
  originally named — `ProxyRouter` has no dependency target at all, since
  `CallTarget::Dependency` resolves in the WASM host capability before a
  `ProxyRequest` exists. Three cases: a cache hit, a cache miss through the
  registry, and a two-member `Redundant` round-robin, the last of which A5e
  is the first slice to make real. The existing unit-level invoke-count
  assertion in `host_capabilities.rs` keeps guarding against a regression to
  a second *network* hop; this bench quantifies the resolution step itself.
  **Measured (2026-08-04):** cache hit ~60 ns, cache miss through the
  registry ~306 ns, two-member `Redundant` round-robin ~58 ns — all three
  orders of magnitude below a network hop, confirming the budget rather
  than merely asserting it.

---

## Exit criteria

Standard gates: `cargo +nightly fmt --all`, `cargo clippy --workspace
--all-targets --all-features`, `cargo test --workspace`, `mise run test:e2e`,
`wasm32-wasip2` compilation, plus:

- **✅** The reference scenario runs end to end across two genuinely
  independent `syneroym-substrate` instances (the
  [federated_fdae_e2e.rs](../../../../crates/substrate/tests/federated_fdae_e2e.rs)
  harness is the precedent for a real two-node test) —
  [reference_scenario_e2e.rs](../../../../crates/substrate/tests/reference_scenario_e2e.rs),
  steps 1-6 (D-A5e-15).
- **Every row of the failure/security matrix has a test, with one named
  exception:** rows 15 and 18 do not, and will not inside this milestone —
  see D-A5e-1 and the two rows' own entries above. They move to slice **S4**
  of the Logical Service Discovery Overlay, which needs three slices of its
  own (S0-S2) this milestone does not build. The remaining eighteen rows are
  each ✅ against a named test.
- **✅** Binding convergence measured; ADR-0021 §6's trigger evaluated
  explicitly, with the answer (not fired) written down — see *Performance
  budgets* above.
- An operator can read health, alerts, and per-dependent binding convergence
  through the `supervisor` interface — the read surface is a deliverable, not an
  implementation detail. **✅**, including the member dimension A5e adds to
  every field of it (D-A5e-2).
- **✅** `[LFC-MGT]` (App Supervisor) and `[FND-IDT]` (stable service identity)
  rows flip to Complete with evidence **once A7 has also landed** (§41 answer
  1) — both A5e and A7 are now Complete, so the milestone closes; see
  [traceability-matrix.md](../../traceability-matrix.md).
- Slice A6 recorded as outstanding in `deferred-backlog.md` §8 *Node lifecycle &
  ops* with its pickup trigger — this milestone closes without it, deliberately.
- **✅** An app instance carries a master DID minted at `adopt`, readable
  through `status`, and movable through `export-master` / `import-master`
  (slice A7).
