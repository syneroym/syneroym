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
| `[FND-IDT]` (stable service identity) | Master DID per *member*; instance keys delegated via `DelegationCertificate`; ingress-side scope enforcement; delegation-signed endpoint records | Extends the existing `[FND-IDT]` row, whose scope is already "Master Key → Temporary Key delegation certificates" — the same mechanism applied to services. **Not** `[FND-IAM]`, which is Access Control. |
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

---

## Dependency gates

| Gate | Status | Effect if unmet |
|---|---|---|
| `ControllerAgreement` creation tool | **Pulled into this milestone as Slice P0** (was M5 item 5) | Not a *functional* blocker: an unowned substrate grants `orchestrator/deploy` to every verified caller today ([io.rs:185-200](../../../../crates/router/src/route_handler/io.rs#L185)), a deliberate bootstrap posture. It is a *security* blocker — see P0. **Gates Slice A3 onward**; A0–A2 are unaffected. |
| M04A/M04B identity + FDAE | Complete | ADR-0020 depends on `subject_did`/`caller_did` already being the master DID. |
| M5 item 1 async primitives (Outbox, DLQ, cron leases) | Not built | Not a gate for this milestone; gates the post-M5 slice only (see below). |
| M7 replication | Not built | Gates stateful relocation only. |

---

## Slices

### P0 — `ControllerAgreement` creation tool *(pulled forward from M5 item 5)*

A prerequisite pulled in from another milestone rather than part of the A-series
design, which is why it is numbered separately.

Nothing in the tree can create a `ControllerAgreement`: `roymctl` creates
identities and `DelegationCertificate`s but has no verb to produce one, and
`[iam].admin_ucan_root` is set in no config file and no e2e or smoke setup. So
`admin_root` resolves to `None` on every real deployment, and B7b's ownership
gate is inert.

**What that actually means — and it is not what an earlier draft of this
milestone said.** An unowned substrate does *not* fail closed. It issues
`orchestrator/deploy`, `undeploy`, and `status` to **every verified caller**
([io.rs:185-200](../../../../crates/router/src/route_handler/io.rs#L185)) — a
deliberate bootstrap posture, since default-deny would brick a substrate
permanently: you could not deploy the thing that establishes ownership. So a
supervisor can already deploy to unowned substrates, and A3 is not functionally
blocked.

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
   `inject-kek`, `rotate-kek`, and `set-secret` are dispatched with **no
   capability check at all** today — see the standing `TODO(M04B/FDAE)` at
   [service.rs:256-260](../../../../crates/control_plane/src/service.rs#L256),
   whose named milestone has since closed without it being addressed. Any
   verified caller can rotate the KEK that encrypts every service database on
   the node. *Alone:* gating it bricks the interface, since nobody can hold
   `substrate/admin` while no `ControllerAgreement` can be created — B7 rejected
   exactly this as "a functional regression, not a tightening."
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

### A1 — Endpoint records under the member master DID — **Complete (2026-07-28)**
Design of record: [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §6
([amended](../../../decisions/0020-stable-logical-service-identity.md#amendment-2026-07-28-after-slice-a1-implementation)
2026-07-28 against what actually shipped). Implementation plan:
[slice-a1-implementation-plan.md](slice-a1-implementation-plan.md). Verification
evidence: [status.md](status.md)'s A1 section.

**Added after review found the mapping this design assumed does not exist.**
The registry today verifies an endpoint record's signature against the key
resolved from the `service_id` it is keyed under
([registry.rs:234](../../../../crates/community_registry/src/registry.rs#L234)),
so an instance key cannot publish under its master; and `MasterAnchorPayload` is
a revocation list with no forward index. Without this slice, a dependent holding
a master DID cannot resolve it to an address at all, and relocation silently
stops working — the exact failure A0 exists to prevent.

`SignedEndpointInfo::verify` gains a second acceptance path: a record keyed by
a master DID, signed by an instance key that presents a valid
`DelegationCertificate` from that master. A substrate-side `EndpointPublisher`
now builds and signs that record at deploy and on the heartbeat — there was no
publish path for it before this slice, only a replay of an operator-signed
file. Master-DID resolution requires a configured HTTP registry: BEP0044/pkarr
keys a packet by its signing key, so a delegation-signed record has no DHT-only
home under its master DID.

**One verification function, not two — corrected after planning found the
mapping this design assumed does not exist.** `verify_endpoint_signature`
([registry.rs:234](../../../../crates/community_registry/src/registry.rs#L234))
*calls* `SignedEndpointInfo::verify`; its own body only ever resolved a key for
a debug log a single caller discarded. The genuinely separate path is
`RegistryClient::lookup`'s **DHT branch**
([dht_registry.rs](../../../../crates/core/src/dht_registry.rs)), which never
called `verify` at all and, by construction, can never carry a delegation-signed
record for a master DID (pkarr keys a packet by its signing key). A1 rewrote
the one verification function to accept §6's keying, added a `RecordTrust`
parameter distinguishing admission (the certificate's expiry is checked) from
reading a record another party already admitted (it is not, so a lapsed
renewal degrades on the registry's TTL clock rather than breaking resolution
instantly), and left the DHT branch untouched.

Covers every `ServiceType`'s *key derivation* with no special case — instance
keys are HKDF-derived from the hosting node's identity
([keys.rs:240-257](../../../../crates/identity/src/keys.rs#L240)) rather than
stored, so a TCP or container service has one exactly as a WASM service does.
The *surface* is narrower: `SyneroymClient::deploy_container` and `roymctl svc
deploy` have no way to carry a member master for a container service today, so
a container-hosted member cannot yet get a master-keyed record (backlog row 79,
retargeted off this slice — it needs a container-deploy CLI surface first).

**Discharges the deferred registry-trust-model ADR** that M04A Slice B7 recorded
as owed (§6.2 / F9 option 2: relax `verify()` to accept a record for X signed by
someone other than X, carrying proof of authorization). B7 gated that work on
finding "a real consumer" — this slice is it. The mechanism differs from B7's
sketch deliberately: the delegation certificate the ingress path already
verifies, rather than a proof drawn from B7a's `owner_of` store.

**Sequences before A2.** Depends on A0.

### A2 — Host-side dependency resolution
Design of record: [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md) §2.

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
A1's trust-chain move at a third site, cheap once A1 lands, and load-bearing:
without it A2 publishes bindings that go stale on the first restart.

Also here: `ProxyRouter::invoke_remote_at`'s `CallOrigin::Native` arm still
presents the *node's* key on the wire (A0 changes only the guest-origin arm),
which is the transport half of the same gap.

### A3 — Multi-substrate placement and the substrate inventory
Placement selector in the manifest (v1: a global default plus per-service
override, by **alias**, never a bare DID); a substrate inventory holding
alias → DID, reachability, capabilities, and the deploy capability held on each;
resolved `substrate` recorded on `PlannedService`; per-(service, substrate)
journal action records; partial-failure semantics (no auto-rollback, mark
`Degraded`, keep retrying).

### A4 — Health, read-only
Health-check declaration in `ServiceConfig` (absent today); a substrate-side
per-instance status query; the supervisor's poll loop; three signals kept
distinct because remediation differs per signal — substrate unreachable, instance
not running, author-declared readiness probe failing. Alert events emitted and
queryable. **No remediation yet**: watch the signal before acting on it.

### A5 — The supervisor loop
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
  scan.
- **Unattended issue and renewal**, so relocation and renewal both just work
  (ADR-0020 §3's online-key posture) — and with it the operator's choice between
  the two postures becomes real rather than nominal.
- **`RotationPolicy`** (`models.rs`) finally becomes load-bearing: with renewal
  automated, whether a service tolerates in-place certificate replacement or
  needs a restart is a decision something actually makes. A0 does not use it
  because operator-initiated replacement is always in-place.

Until A5 lands, **a missed renewal cadence is an outage** (matrix row 3), and
that is the milestone's standing operating cost through A0–A4.

### A6 — Durable delivery *(post-M5 item 1 — do not start before it lands)*
Replace the A5 trait's implementation with an outbox/DLQ-backed one: durable push
delivery, retry against substrates that are offline at the time of the change,
terminal-failure handling, and a single-writer cron lease if redundant
supervisors are wanted. Nothing above the trait changes.

**Pickup trigger:** M5 item 1 (Outbox, DLQ, cron leases) marked Complete in the
[traceability matrix](../../traceability-matrix.md). Tracked in
[deferred-backlog.md](../../deferred-backlog.md) §8 *Node lifecycle & ops* so it
is not remembered by accident.

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
member set propagates correctly. Step 4 also exercises A1 — without
delegation-signed endpoint records the restarted member could not republish
under its master and step 2's second lookup would fail.

---

## Failure / security matrix

| # | Case | Expected |
|---|---|---|
| 1 | Instance certificate expired | Handshake fails closed. **✅ A0**: already true of the general mechanism, and pinned specifically for a service-instance certificate by `an_expired_instance_certificate_fails_the_handshake_closed` (`crates/router/src/handshake.rs`). Unattended renewal under the online-key posture is A5's; through A0-A4 renewal is an operator-run cadence (see row 3) |
| 2 | **Certificate presented with the wrong `scope`** | Rejected. **✅ A0, at two granularities**: the ingress (`handshake.rs`) admits either transport scope and rejects anything outside that set (`a_certificate_scoped_outside_transport_is_rejected_at_the_handshake`); the narrow single-value check — "this record/install may only be admitted by a `service-instance` certificate" — lives at the deploy-time install verification (`a_deploy_is_rejected_when_the_certificate_carries_the_routing_scope`, `crates/control_plane/src/service/orchestration.rs`) and, for A1, at endpoint-record admission. Proven live over two real substrates in `crates/substrate/tests/instance_identity_e2e.rs` (a `routing`-scoped certificate rejected at deploy) |
| 3 | **Attended posture, renewal cadence missed** | Instance fails closed; this is an outage, not a degradation, and the docs say so (ADR-0020 §3). **✅ A0** proves the handshake half: not a distinct code behavior there — the failure mode *is* row 1 — so the evidence is row 1's test plus the near-expiry heartbeat warning (`a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep`, `crates/substrate/src/runtime.rs`) and the `svc list` expiry column. **A1 adds a second, distinct failure mode** (D-A1-10): a lapsed certificate also stops `EndpointPublisher` refreshing the member's record, so name *resolution* fails too — on the registry's TTL clock (`DEFAULT_REGISTRY_TTL_SECS`, two hours) rather than the handshake's instant cliff. **✅ A1**: `an_expired_certificate_blocks_publishing_but_not_reading` (`crates/core/src/dht_registry.rs`) and `an_expired_certificate_publishes_nothing` (`crates/core/src/endpoint_publisher.rs`) |
| 4 | **Endpoint record keyed by a master, signed by a non-delegated key** | Rejected; only a valid `DelegationCertificate` from that master admits the record. **✅ A1**: `a_record_keyed_by_a_master_is_rejected_when_signed_by_an_uncertified_key` (`crates/core/src/dht_registry.rs`), proven live over two real substrates by the negative half of `a_member_master_did_resolves_to_an_address_and_follows_the_member_across_nodes` (`crates/substrate/tests/master_endpoint_record_e2e.rs`) — a hand-built record posted to the registry is rejected with `401` |
| 5 | Out-of-order binding write (stale epoch) | Rejected by the substrate; mapping does not regress |
| 6 | **Binding write re-sent at the current epoch, identical content** | Idempotent no-op, reported as success — distinct from the stale rejection above |
| 7 | **Binding write at the current epoch, *different* content** | Rejected as a conflict, reported distinctly; this is the two-writer signal |
| 8 | Second supervisor adopts a managed instance | Lower generation rejected; no flapping |
| 9 | **Supervisor finds a *higher* generation than its own** | Stops managing that instance and alerts; never self-increments (ADR-0021 §4) |
| 10 | Deploy retried after a lost response | Idempotent no-op for identical (instance, service, content hash) |
| 11 | Dependent unreachable during a push | Instance marked `Degraded`; retried; state visible on the operator read surface |
| 12 | Partial app deploy (3 of 5 services) | No rollback; `Degraded`; failed services retried |
| 13 | Remediation exceeds max attempts | Terminal `Degraded`; alerting only; no restart loop |
| 14 | Supervisor holds master keys and is compromised | Blast radius bounded to the members it manages; instance certificates short-lived and revocable (ADR-0020 §3). **Split.** **✅ A0** proves the testable bound: an instance certificate is revocable without touching the member master, and a fresh instance key from the same master still verifies (`a_revoked_instance_key_is_rejected_while_the_member_master_still_certifies_a_new_one`, `crates/router/src/handshake.rs`). The other half — blast radius actually bounded to what a supervisor manages — needs a supervisor and is A5's |
| 15 | Bound cross-app dependency replaced, **online-key posture** | Active probe fails on a call the supervisor makes *as the depending member*; A's owner alerted (ADR-0021 §7) |
| 16 | **`security` call (`inject-kek`/`rotate-kek`/`set-secret`) without `substrate/admin`** | Rejected. Ungated entirely today (P0 item 2); the gate only becomes holdable once P0 item 1 ships |
| 17 | **Deploy to an unowned substrate once F4 flips** | Rejected; the bootstrap path becomes establishing ownership with the P0 tool, not an open deploy grant |
| 18 | Bound cross-app dependency replaced, **attended posture** | No master key, so no active probe: detection is passive, on the first real call that fails. Weaker by design, and the operator chose it |
| 19 | **Member reinstantiated under a policy declaring `expected_asserter_did`** | Its cross-service `RelationshipProof` still verifies. Today it would not: `asserter_did` is the *instance* key and the check is exact equality, so a restart on another node silently breaks every policy naming that service (A2) |

---

## Performance budgets

- **Binding convergence — provisional target, set a priori so it can actually
  fail:** from a membership change, **all reachable dependents converged within
  5 s**, and **any dependent that was unreachable converged within one poll
  interval of becoming reachable**. A budget derived from the first measurement
  could never be missed, which would make ADR-0021 §6's falsification test
  vacuous — so this number is a guess on purpose, and the milestone owner revises
  it at A5 sign-off with the measurement and the reasoning recorded. Missing it
  is the trigger to build the pull path.
- **Health poll cost:** the supervisor's steady-state poll must not be a
  meaningful load on a target substrate at the intended inventory size.
- **Resolution adds no network hop.** A2 moves dependency resolution host-side;
  the name → master-DID step must stay an in-process cache lookup. The
  master-DID → endpoint step is the registry lookup that already happens today
  (A1 keeps it at exactly one, which is why ADR-0020 §6 chose delegation-signed
  records over a two-hop anchor index).

---

## Exit criteria

Standard gates: `cargo +nightly fmt --all`, `cargo clippy --workspace
--all-targets --all-features`, `cargo test --workspace`, `mise run test:e2e`,
`wasm32-wasip2` compilation, plus:

- The reference scenario runs end to end across two genuinely independent
  `syneroym-substrate` instances (the
  [federated_fdae_e2e.rs](../../../../crates/substrate/tests/federated_fdae_e2e.rs)
  harness is the precedent for a real two-node test).
- Every row of the failure/security matrix has a test.
- Binding convergence measured against the provisional budget above; ADR-0021
  §6's trigger evaluated explicitly, with the answer written down either way.
- An operator can read health, alerts, and per-dependent binding convergence
  through the `supervisor` interface — the read surface is a deliverable, not an
  implementation detail.
- `[LFC-MGT]` (App Supervisor) and `[FND-IDT]` (stable service identity) rows
  flipped to Complete with evidence.
- Slice A6 recorded as outstanding in `deferred-backlog.md` §8 *Node lifecycle &
  ops* with its pickup trigger — this milestone closes without it, deliberately.
