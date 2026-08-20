# Syneroym Meta-Implementation Plan

This document provides a high-level strategic roadmap for implementing the Post-DD864A1 architecture. It takes the pending items described across Phase 0 to Phase 7 and reorganizes them into logical, sequentially buildable **Milestones (Epics)**. 

There is no single "MVP" boundary or distinct "Pilot" launch. Instead, we build features phase by phase, treating every phase as its own incremental pilot that delivers immediate end-to-end value. This meta-plan avoids dependency cycles by slicing deep architectural features (like Security and Data) across multiple milestones. It emphasizes the "walking skeleton" principle by growing a reference SynApp (Professional Services Guild) through every milestone to prove out the infrastructure.

## Guiding Implementation Strategy
1. **Inside-Out Development:** We build the core local primitives first (routing, security, data) before expanding to multi-node federation and high-level applications.
2. **Continuous Walking Skeleton:** Do not defer product validation. Every milestone must expand a reference SynApp (focusing on the **Professional Services Guild**) to ensure we don't build isolated testing facades.
3. **Strict Boundaries:** No communication crossing a `SynSvc` trust boundary may bypass identity and authorization enforcement. Statically composed components are treated as one `SynSvc` boundary.
4. **Shared Orchestration:** Planning logic is not built independently inside `roymctl` and the Substrate. Instead, `crates/app_orchestration` acts as a pure manifest compiler producing an immutable `DeploymentPlan`. `roymctl` and the active controller act merely as effectful adapters around this shared planner.
5. **Continuous Observability & Tooling:** Observability instrumentation and developer tooling begin early and mature throughout the milestones.

## Standard Milestone Documentation Format
When we begin work on any milestone below, we will generate a dedicated `task.md` that strictly includes:
- **Requirement IDs** (e.g., `[TOP-PRM]`)
- **Explicit non-goals**
- **Dependency gates**
- **Migration impact**
- **A runnable reference scenario** and failure/security tests
- **Performance budgets**
- **Measurable exit criteria**, which must strictly enforce passing:
  - `cargo +nightly fmt --all`
  - `cargo clippy --workspace --all-targets --all-features`
  - `cargo test --workspace`
  - `mise run test:e2e`
  - Relevant `wasm32-wasip2` compilation
  - End-to-end reference-scenario and failure-recovery tests

---

## Milestone 0: Contract and Decision Gate
**Goal:** Formalize the traceability matrix and create a baseline API migration plan for the existing codebase before major structural changes begin.

**Implementation Approach:**
1. **Traceability Matrix:** Map every requirement and sub-requirement to its current implementation status, target milestone, and acceptance evidence.
2. **Decision Register:** Resolve only milestone-blocking ADRs (e.g., encryption implementation, DLN scope, manifest versioning boundaries). Maintain a decision register for non-blocking open questions relevant to Milestones 9–10.
3. **DLN Scope Resolution:** Explicitly decide the target milestone for the Dynamic Ledger Network (DLN) and whether signed-interaction-receipts (and therefore robust `[P2P-REP]` reputation) are scheduled early or assigned to later milestones.
4. **SQLite Encryption ADR:** Build an ADR/feasibility prototype for the exact encrypted-SQLite mechanism to be used in M3.
5. **Baseline Migration Plan:** Draft the plan to migrate current `roymctl` contracts.

---

## Milestone 1: Local App Model and Lifecycle
**Goal:** Establish the fundamental execution boundary (`SynSvc`), the application grouping overlay (`SynApp`), and a shared orchestration planner.

**Feature Grouping:**
- `[TOP-PRM]` Core Primitives & Overlay
- `[TOP-ADR]` Service Addressing
- `[TOP-REG]` Registries (App & Endpoint)
- `[TOP-DSC]` Discovery Mechanisms
- `[LFC-MGT]` Standalone `roymctl` Deployment & Manifest parsing
- `[LFC-VER]` Manifest versioning

**Implementation Approach:**
1. **Baseline Migration:** Migrate the current CLI and dispatcher contracts to align with the new `SynApp` vs `SynSvc` terminology.
2. **Shared Orchestration:** Build the pure `DeploymentPlan` compiler in `crates/app_orchestration`.
3. **Topology Work:** 
   - Implement strongly typed IDs and logical references.
   - Build dependency graph compilation with cycle detection, explicitly differentiating `Spawn` vs `Bind`.
   - Create the Logical resolver that sits above the physical router.
   - Implement Static and Native registry modes with TTL/topology-epoch cache invalidation.
   - Implement the Phase 0 Master Anchor resolution contract.
4. **Deployment Journal:** Implement a crash-consistent local deployment journal for standalone deployments.

---

## Milestone 2: Reliable, Operable Node
**Goal:** Establish basic network transport robustness, node identity, and foundational deployment mechanics before introducing stateful persistence.

**Feature Grouping:**
- `[TOP-ROB]` Network & Connection Robustness
- `[FND-IDT]` Cryptographic Identity Primitives (Handshake authorization slice)
- `[FND-DEP]` Deployment/Operations
- `[FND-SEC]` Substrate Security (Runtime quotas, memory protection)

**Implementation Approach:**
1. **Robust Transport:** Implement automatic connection retries and idle timeouts. Ensure reactive eviction relies natively on Iroh’s connection pooling rather than a custom application-level cache.
2. **Identity Handshake:** Implement Master Key → Temporary Key delegation, handshake authorization, revocation, and the Master Anchor signed-record contract.
3. **Operational Baselines:** Integrate runtime quotas and memory protection bounds. Provide explicit support for native TLS and certificate lifecycle, official Docker images, cross-platform release pipelines, and deployed smoke tests.

---

## Milestone 3: Secure Stateful Services

To prevent dependency cycles and scope creep, the data layer and storage mechanisms are split into sequential sub-milestones.

> **Planning-doc split (2026-07-09):** M3A and the blob half of M3B are
> tracked in `docs/planning/milestones/M03-sss/task.md` (complete). The
> messaging half of M3B and all of M3C are tracked in their own document,
> `docs/planning/milestones/M03B-messaging/task.md`, split out before
> implementation began because pre-implementation planning for that
> remaining work had grown large enough to make the original single file
> unwieldy. The milestone numbering/labels below (M3A/M3B/M3C) are
> unchanged — only which file carries the detailed task checklist differs.

### Milestone 3A: Structured State and Security
**Goal:** Introduce the baseline SQLite data layer intimately paired with storage encryption and the secret vault.

**Feature Grouping:**
- `[PLT-DAT]` Data Layer (Structured SQLite DBs per service, `syneroym-oltp`/`syneroym-olap` profiles)
- `[PLT-DAP]` Distributed Data Topology (Logical Data Service foundations)
- `[FND-SEC]` Substrate Security (Storage encryption, Vault)
- `[FND-CFG]` Service Configuration Delivery

**Implementation Approach:**
1. **Encrypted Isolation:** Provision isolated, encrypted SQLite databases for each deployed `SynSvc` (based on the M0 prototype).
2. **Data Interface:** Implement schema initialization, CRUD/batch operations, structured MongoDB-style JSON filters, pagination, concurrency architecture, and nested WIT serialization (JSON payloads at the WIT boundary per [ADR-0007](../decisions/0007-data-layer-wit-interface.md)). The `AggregationPipeline` is deferred to Milestone 4 (gate item below). Include Cargo feature gates for `syneroym-olap` and `syneroym-oltp` profiles (both currently backed by standard SQLite).
3. **Vault Integration:** Build the secret vault into the encrypted DB and implement `syneroym:vault/reveal`.
4. **Configuration Delivery:** Finalize the delivery mechanics (WASM host functions vs. Podman environment mapping).

### Milestone 3B: Objects and Events

> Blob storage (below) is tracked in `docs/planning/milestones/M03-sss/task.md`
> (Slice 5, complete). The event-broker/messaging item is tracked in
> `docs/planning/milestones/M03B-messaging/task.md` (Slice 6A).

**Goal:** Provide the remaining fundamental asynchronous data primitives.

**Feature Grouping:**
- `[PLT-DAT]` Blob S3 Integration
- `[PLT-DAP-04]` Decentralized Pub/Sub (MQTT API)

**Implementation Approach:**
1. **Blob Storage:** Implement the `object_store`-backed S3-compatible backend interface with signed (HMAC presigned) HTTP object access ([ADR-0009](../decisions/0009-blob-storage-object-store.md)); public unsigned serving is deferred. Blob content is DEK-encrypted at rest per `[FND-SEC]`.
2. **Event Broker:** Embed the pub/sub half of `syneroym:messaging` as an in-process `rumqttd` Tokio task with host-enforced topic namespacing ([ADR-0010](../decisions/0010-mqtt-broker-rumqttd.md)). The package was formerly named `syneroym:pubsub`; renamed to `syneroym:messaging` to share a boundary with the bidirectional-streaming half added in Milestone 3C. Adapting the broker to decentralized P2P log replication over Iroh QUIC (avoiding classical TCP brokers, per `[PLT-DAP-04]`) is deferred to Milestone 7 (moved there from an earlier Milestone 5 placement — see the Milestone 5 note below). **Course correction (post-implementation):** the original plan to declare the `stream-types`/`handle-stream-request` portion of the WIT package in M3B "for interface stability" but leave it unimplemented until M3C was dropped before implementation began — `syneroym:messaging@0.1.0` shipped in M3B (Slice 6A) with *only* `host-api::publish`/`subscribe`/`unsubscribe` and `guest-api::handle-message`; no streaming surface, no placeholder machinery. Since this WIT package is never released outside this repository, breaking additions between M3B and M3C cost nothing, so the streaming surface was added fresh in M3C (Slice 6B) instead — see `docs/planning/milestones/M03B-messaging/task.md`'s "WIT Boundary Versioning" section (Finding A3) for the full reasoning.

### Milestone 3C: Unified Messaging Streams and HTTP Bridge

> Tracked in `docs/planning/milestones/M03B-messaging/task.md` (Slice 6B:
> streaming; Slice 7: HTTP bridge).

**Goal:** Extend `syneroym:messaging` with generic bidirectional streaming, then bridge HTTP conventions onto the native-dispatch surface established across M3A/M3B/3C (data-layer, vault, app-config, blob-store, messaging).

**Feature Grouping:**
- `[PLT-DAP-06]` Generic Bidirectional Streaming
- HTTP Passthrough (GET/POST/streaming-upload/SSE-style translation onto native dispatch)

**Implementation Approach:**
1. **Streaming Out (guest as source):** Wire `host-api::register-stream-protocol`, `guest-api::handle-stream-request`, and the `stream-cursor` resource end to end, including host-side QUIC stream acceptance/routing (new infrastructure, not present in M3B — expected to need its own short design note/ADR before implementation, the way D-03-01 through D-03-05 preceded M3 slice work).
2. **Streaming In (guest as sink):** Wire `guest-api::accept-stream-upload` and the `stream-sink` resource (`push-chunk`/`finalize`) end to end — the host runs the async QUIC-read loop and pushes chunks into the guest, the reverse of the pull loop above. Covered by the same QUIC routing infrastructure and design note as item 1.
3. **HTTP Passthrough:** Convert an HTTP GET/POST/streaming request's path, method, and body into a native or WASM call against data-layer, blob-store, or messaging, and stream the response back — enabling signed-URL blob serving, static content, JSON-RPC-style DB access, SSE/long-poll pub/sub subscription, and chunked upload (via `accept-stream-upload`/`stream-sink`) over the same substrate HTTP surface.

This was **Milestone 3B Slice 6 / "Deferred: HTTP Passthrough"** in earlier planning; split out because both items are new, undecided infrastructure (no prior ADR covers QUIC stream routing or HTTP-to-WIT translation) rather than execution of an already-resolved M3 decision, and neither should block M3B's close.

---

> **Interstitial maintenance (2026-07-09):** between M03B's close and M4
> start, the workspace crate layout was normalized (`data-layer` →
> `data_db`, `blob-store` → `data_blob`, `key-store` → `data_keystore`,
> `bindings` → `wit_interfaces`, `app_sandbox` → `sandbox_wasm`,
> `podman_sandbox` → `sandbox_podman`) and workspace-wide import cleanup was
> applied per `AGENTS.md`. No behavior change. See
> [ADR-0012](../decisions/0012-crate-rename-refactor.md) and
> [crate-rename-refactor.md](crate-rename-refactor.md) for the decision and
> execution plan. File paths referencing the old crate names in *closed*
> milestone docs (M0–M3B) below and above are left as-is by design.

## Milestone 4: Typed Communication and Authorization

> **Planning-doc split (2026-07-13):** M4 is planned and implemented as two
> sub-milestones, mirroring the M3A/M3B/M3C precedent —
> **M4A** (`docs/planning/milestones/M04A-proxy-and-auth-foundation/task.md`) and
> **M4B** (`docs/planning/milestones/M04B-fdae-policy/task.md`) — split along the
> **capability-plumbing vs. data-aware-policy-engine** boundary. During planning,
> three simplifications were adopted (full reasoning in each task.md's Decision
> Register): **(a) wRPC deferred** to a later milestone — the Universal Proxy
> ships over **JSON-RPC**, since the QUIC transport wRPC would ride on already
> exists (M3C) and both wires need the same host-side `Val`⇄JSON conversion, so
> wRPC is a wire-efficiency optimization, not a prerequisite; **(b) protocol
> negotiation deferred** — fail-fast unsupported-protocol errors instead, since
> there is only one protocol; **(c) credit-based backpressure dropped** in favour
> of QUIC-native flow control. `[PLT-DAP-05]` data-pipeline streams ship as a
> spike / M5 candidate only.

**Goal:** Bridge isolated services securely by introducing the typed Universal
Proxy (over JSON-RPC) and layering the FDAE (Access Control) on top of the
established Data Layer.

### Milestone 4A: Universal Proxy & Auth Foundation
**Goal:** Typed cross-node calls plus the authentication / capability-admission
foundation. Independently closes the tracked M3 native-dispatch security gap
(gate item below).

**Feature Grouping:**
- Universal Proxy over **JSON-RPC** (wRPC deferred)
- Full WIT⇄JSON value conversion (typed dispatch)
- `[FND-IAM]` foundation — UCAN context, verified caller-identity threading, Admin UCAN capability
- `[FND-SEC]` — per-SynApp-Instance KEK narrowing
- `[PLT-DAP-05]` Data Pipeline Streams — spike-first / M5 candidate
- *(deferred: `[LFC-VER]` protocol negotiation; wRPC binary wire)*

**Implementation Approach:**
1. **Full WIT⇄JSON conversion** (startable immediately, no ADR): replace the `conversions.rs` stub with a full component-model ↔ JSON converter — the enabler for genuinely typed calls over a JSON wire.
2. **Native-dispatch auth-gap closure** (highest priority — closes the gate item below): make `verify_preamble` mandatory, thread a verified caller identity through `NativeInvocation`/`dispatch` and the HTTP bridge, and replace `is_init_context` with the Admin UCAN capability.
3. **Universal Proxy** over JSON-RPC / Iroh QUIC, kept transport-agnostic behind the `AdaptationStage` seam so a later wRPC wire slots in additively.
4. **UCAN Context:** verify and normalize UCAN scopes/claims into a SessionContext at request ingress.
5. **`AggregationPipeline`** (gate item deferred from M3A, [ADR-0007](../decisions/0007-data-layer-wit-interface.md)): `$group`/`$having`/projections onto SQLite `GROUP BY`/`HAVING`/views.
6. **Privileged `query-raw`** (gate item deferred from M3A, [ADR-0011](../decisions/0011-privileged-raw-sql-query.md)), gated by the Admin UCAN capability.
7. **Per-SynApp-Instance KEK narrowing** (D-03-01 follow-on).

### Milestone 4B: FDAE Data-Aware Authorization
**Goal:** The FDAE policy engine, layered on M4A's identity/capability
foundation. Carries no M3 debt — it is purely the new engine.

**Feature Grouping:**
- `[FND-IAM]` Access Control — FDAE, data-centric RLS/CLS, the 4-stage hybrid pipeline

**Implementation Approach:**
1. **Local FDAE:** the SQL Pushdown Sieve — compile declarative ReBAC policies into SQLite `WHERE EXISTS`/`WITH RECURSIVE`, with Mode A (point-in-time evaluation) and Mode B (relational data filtering), RLS + CLS.
2. **Federated FDAE:** cross-service parameter fetch (pipeline stage 2) via the M4A Universal Proxy.
3. **Stage-4 ABAC:** an optional pure-predicate WASM after-step on candidate rows (restrict-only by default).

**Native-dispatch authentication gap (gate item — closed by M4A, item 2 above).** `RouteHandler::handle_stream` (`crates/router/src/route_handler/io.rs`) only runs `HandshakeVerifier::verify_preamble` — the sole point that checks caller identity against `preamble.service_id` — when `preamble.delegation` is present, and the native-capability interfaces added across M3A/M3B/M3C (`data-layer`, `vault`, `app-config`, `blob-store`, `messaging`; plus the HTTP-bridge call path) never require one. Concretely: any peer that can open an Iroh connection to a node (the QUIC listener binds `0.0.0.0` by default — *not* bounded by `client_gateway`'s `127.0.0.1`-only convenience proxy) and knows a target service's DID can act "as" that service on every native-capability interface and the M3C HTTP bridge, with no cryptographic proof of being it — `data-layer` writes, `messaging::publish`, SSE eavesdrop, blob access. Recorded as an explicit, tracked interim posture at M3C close (see `docs/planning/milestones/M03B-messaging/status.md`, "Interim HTTP-write security posture," and [ADR-0010](../decisions/0010-mqtt-broker-rumqttd.md) Finding B9). M4A's B0 slice wires delegation/capability verification into this exact dispatch path (native-dispatch **and** the shared HTTP-bridge routes) before M4A can be considered closed.

### Interstitial: `[FND-CFG]` Deploy-Time Artifact Delivery (2026-07-22)

> Executed **between M04B Slice B2 and Slice B3**, tracked in
> [fnd-cfg-artifact-delivery-plan.md](fnd-cfg-artifact-delivery-plan.md) and
> decided in [ADR-0019](../decisions/0019-deploy-time-artifact-delivery.md).
> This follows the *Interstitial maintenance (2026-07-09)* precedent above —
> work owned by no single milestone, given its own ADR and plan doc — with one
> honest difference: the crate rename was "No behavior change," this is a
> behavior change and a breaking WIT change.

Deploy-time artifacts split into two groups by accident of build order, not by
design. WASM bytes (`artifact-source::binary`) and `custom-config` travel
**inside** the deploy call; the JSON Schema (`schema-path`), the FDAE policy
document (`fdae-policy-path`), and Podman volume contents are **assumed to
already exist on the substrate host's filesystem**, with no upload path
anywhere in the WIT, the SDK mapper, or the deploy handling. A deploy needing
any of the three only succeeds if someone with direct filesystem access staged
the file out of band — which defeats the point of a deploy API. For Podman
volumes the gap is total: `create_dir_all` produces an *empty* bind-mounted
directory, so an off-the-shelf image that reads its config from a mounted file
rather than `-e KEY=VALUE` env vars cannot be deployed through the API at all.

The fix adds a shared `document-source { path, inline }` variant used at all
three sites, plus per-file content on container volumes mounted read-only.

**Why it is not an M04B slice.** Two of the three gaps are `[FND-CFG]` debt
from M3A Slice 4, whose scope included "Podman env-var **and file-mount**
fallback" — only the env-var half shipped. M04B's charter is explicitly "purely
the new engine" and "carries no M3 debt" (M04A is the milestone designated to
carry M3 debt). Only `fdae-policy-path` is FDAE's, and even that is a delivery
mechanism rather than engine work.

**Why here rather than after M04B closes.** It is independent of B3/B4/B5 in
both directions, so sequencing it after them would couple it to B5 — which is
itself gated on the unresolved **D-04-02-f** create-authorization sub-decision.
It also breaks the `service-config` WIT record across ~30 construction sites,
and a breaking schema change is cheapest at a quiescent point with no slice in
flight. Secondary: B2 shipped `fdae_policy_path`, which is API-undeployable
until this lands.

---

## Interstitial: Live App-Context Registry (between Milestone 4 and Milestone 5)

> **Superseded 2026-07-27 by [M05A-app-supervisor](./milestones/M05A-app-supervisor/task.md).**
> Both goals below are carried forward; the *mechanism* changed. Designing this
> out produced [ADR-0020](../decisions/0020-stable-logical-service-identity.md)
> and [ADR-0021](../decisions/0021-binding-propagation-and-app-supervisor.md),
> which conclude that a **live** registry queried at runtime is the wrong shape:
> a service's master DID is stable across relocation, so the logical-name
> mapping changes only on genuine membership change, and pushing it into each
> dependent's configuration removes a hot-path dependency on the control plane
> for a cold event.
>
> **How each goal is now met** — the two are still one build, as argued below:
> 1. *Logical-name resolution backed by real deployment state* — the App
>    Supervisor holds that state and pushes the resolved member set into each
>    dependent. `StaticInventory` remains the `AppRegistry` implementation; what
>    changes is that something finally calls `.register()` on it. The
>    resolution itself moves **host-side** (ADR-0021 §2), so a guest names a
>    *declared dependency* — not a `LogicalServiceRef`, which would let it
>    address an arbitrary app instance — and can never hold a stale DID.
> 2. *`expected_asserter_did` publication (B3's D-B3-8 residual)* — carried
>    forward unchanged in intent: the pushed binding carries
>    `{member_master_did, expected_asserter_did}` per member rather than a bare
>    `service_id`, so an
>    FDAE policy author's lookup still resolves both "who backs this logical
>    name" and "what will it sign with" from one place. Push versus pull does
>    not affect this; it is a question of what the entry contains.
>
> The rest of this section is retained as the original statement of the problem
> and of the verified state that motivated it. Where it says "live registry,"
> read "the supervisor's state, pushed."

**Goal:** Give `[PLT-DAP-01]`'s "app-context registry" — named in
`system-architecture.md`'s "Logical Names and Public Aliases" section and in
[ADR-0017](../decisions/0017-fdae-policy-schema-and-compilation.md) §1 as a
mechanism that "already exists" — a real, **live** implementation, so a
logical service name (e.g. a policy's `Relation.service`, or a manifest
dependency) resolves to a DID backed by actual current deployment state, not a
hand-populated test double. **Second goal, folded in here rather than a
separate initiative (added 2026-07-25, M04B Slice B3 Phase 5):** the same
registry entry should also carry each service's `expected_asserter_did` —
the residual gap from B3's D-B3-8. `resolve_fetches` verifies a remote
`RelationshipProof` against a policy-declared `expected_asserter_did`
(`Identity::derive_service_identity(owner_did, service_id)`, deliberately
node-private), but a policy author on a different node has no automated way
to learn it today — only the out-of-band channel Phase 5's own e2e test
uses (reading it directly off the node the test itself constructed, the
same access a real deploying operator would have). This is the same class
of lookup problem as the logical-name resolution above, and plausibly the
same data store: if the live `AppRegistry` records `{service_id,
expected_asserter_did}` per entry instead of just `{service_id}`, one build
closes both gaps instead of two initiatives solving overlapping problems.
Previously tracked as its own `deferred-backlog.md` §3 row ("Cross-node
`expected_asserter_did` discovery/publication"); moved here as its actual
home, for the same reason D-B3-10 lives here and not there — this is
committed platform work, not droppable debt.

**Verified current state (2026-07-24).** It does not exist. `crates
/app_orchestration/src/resolver.rs`'s `AppRegistry` trait +
`LogicalResolver`/`TopologyCache` is the right shape (`LogicalServiceRef
{app_instance_id, service_name} -> ServiceId`, topology-mode-aware selection
for `Singleton`/`Redundant`/`Sharded`), but its only implementation,
`StaticInventory`, is an in-memory map nothing calls `.register()` on — no
deploy or reconcile code path populates it, and it has zero production
callers today (only re-exported from `app_orchestration::lib`). The
`Reconciler`/`DeploymentJournal` (`reconcile.rs`) compute desired-vs-active
deployment diffs but do not track per-service health, and nothing currently
publishes a live topology entry when a service actually comes up.

**Scope:** superseded — see [M05A slices A0-A2](./milestones/M05A-app-supervisor/task.md).
The original scope paragraph here prescribed a live registry backend and has
been removed rather than left to be read as current design; what replaced it
is push-based propagation over a stable per-member identity, with the
`expected_asserter_did` goal above carried into the pushed binding entry.

**Why it's not a Slice B3 deliverable.** It is an `app_orchestration`-crate,
cross-milestone concern (also serves `[PLT-DAP-01]`'s physical-sharding
transparency goal, not just FDAE), not FDAE engine work — the same reasoning
that kept `[FND-CFG]` out of M04B above. Building it as a Slice-B3-Phase-4
side quest would both under-scope it (B3 only needs single-DID resolution, not
topology-mode/sharding selection) and block a slice on work that belongs to a
different owner.

**What Slice B3 Phase 4 does in the interim, and why that's acceptable.**
`RemoteFetch.service` (a policy-declared name) is resolved the same way
`ProxyRequest.target_service` already is today — directly, through the
existing `EndpointRegistry`/community-registry DID lookup `ProxyRouter`
already performs for every other proxied call — with no logical-name
indirection layer. This is a real, working mechanism for the deployment
shapes M04B supports (a policy names a service identifier the proxy can
already resolve); it is not a stub. The gap this interstitial closes is
specifically the *logical name* indirection (`org-service` -> whichever DID
currently backs it), which nothing in the platform provides yet for *any*
consumer, FDAE included. Recorded in
[slice-b3-implementation-plan.md](milestones/M04B-fdae-policy/slice-b3-implementation-plan.md)
§1 and task.md's Decision Register, not `deferred-backlog.md` — this is
committed platform work, not an item that can be safely dropped.

---

## Build-Order Amendment: M5–M7 Resequencing (2026-07-16)

> **Optimization target:** the end-state at M7 close, not the wall-clock
> timing of any individual milestone. The milestone **numbers and labels
> below are unchanged** — this is a build-order note, exactly like the
> M3B/M3C course corrections above, not a renumbering. It re-slices *when*
> the work inside M5–M7 is done, driven by two observations: (1) the two
> highest-uncertainty pieces in the whole plan — the DataFusion/Substrait
> federated-query orchestrator (M5, item 2) and SQLite WAL replication
> (M7, item 2) — both carry explicit "Design TBD" flags and are the things
> most likely to surprise us; (2) M6 (the product/chat milestone, our best
> signal the foundation actually works) depends only on M5's **async
> primitives**, not on the orchestrator, versioning, or dev tooling.

**Resequenced build order:**

1. **M4A → M4B first, unchanged.** Identity/capability foundation → FDAE is
   a genuine, security-critical dependency chain; no reason to reshuffle it.
2. **Split M5; front-load only the half M6 needs.** Do the async primitives
   (Outbox, DLQ, cron leases, long-running-task restart, sagas — M5 item 1)
   immediately after M4B. **Defer** the federated-query orchestrator (M5
   item 2), state versioning/rollback (item 3), and developer tooling
   (item 4) — nothing downstream blocks on them.
3. **Pull M6 (product/chat) forward**, right after the async-primitives
   slice, so integration bugs surface early against real identity + FDAE +
   async infra rather than after two more heavy subsystems are stacked on
   top.
4. **Start the two high-uncertainty spikes early and in parallel**, decoupled
   from the rest: the **M7 SQLite-WAL-replication feasibility prototype**
   (M7 item 1 — already scoped as a bounded prototype with correctness /
   crash-recovery / performance exit criteria) and the **orchestrator's
   shard-discovery / data-routing design question** (the M5 item 2 "Design
   TBD"). Neither depends on FDAE, the async primitives, or the chat app —
   both are pure systems-design problems, so running them early de-risks the
   plan without blocking the product path.
5. **Finish the deferred M5 half + all of M7 as the final phase**, informed by
   the spikes. The orchestrator (query planning) and M7 storage/transport
   replication (WAL, pub/sub log, blob) are largely independent of each
   other and can overlap.

**Tradeoff, stated plainly:** this runs two work streams concurrently
(coordination overhead) and gives up the clean "one milestone, one
checkpoint" story. Accepted deliberately: it moves the two riskiest,
least-defined pieces to cheap early spikes instead of leaving them as
mid-plan blocking dependencies, and lands the product-validation milestone
earlier. Per-milestone `task.md` documents are still authored per the
Standard Milestone Documentation Format above; this amendment only governs
the order in which their slices are picked up.

---

## Build-Order Amendment: App Supervisor Split (2026-07-27)

> **What changed:** M5 item 2's "Active Controller" was scoped as a *live
> registry* — a Server SynApp that services query at runtime to resolve
> logical names. Designing it produced two decisions that shrink it
> substantially and change where it sits in the order:
> [ADR-0020](../decisions/0020-stable-logical-service-identity.md) (each
> *member* of a logical service has a stable master DID, so relocation and
> restart stop changing who that member *is*) and
> [ADR-0021](../decisions/0021-binding-propagation-and-app-supervisor.md)
> (bindings are pushed into service config; there is no live directory). The
> component is renamed the **App Supervisor** and runs as a substrate role,
> not a WASM `SynApp`.

**Consequence for build order:** most of this work no longer depends on M5's
async primitives and moves **before** them; only the durable-delivery half
waits.

1. **Lands ahead of M5's async primitives** — tracked in
   [M05A-app-supervisor](./milestones/M05A-app-supervisor/task.md):
   stable master DID per member of a logical service; endpoint records
   published under that master; host-side dependency resolution in the proxy
   target; multi-substrate placement plus the substrate inventory; health
   definition and read-only monitoring; and the supervisor loop itself with
   best-effort synchronous delivery.
2. **Waits for M5 item 1** (Outbox/DLQ/cron leases): durable push delivery and
   retry against offline substrates, terminal-failure handling, and the
   single-writer lease if redundant supervisors are ever wanted. These sit
   behind one narrow "apply this action to that substrate" trait so the
   pre-M5 implementation is replaced rather than unwound — deliberately, so
   the project does not grow a second retry mechanism.
3. **Waits for M7** (replication): remediation by *relocating* a stateful
   service. Until a service's data can follow it, remediation is
   restart-in-place only. ADR-0020 removes the identity blocker; replication
   remains.
4. **Hard dependency, unchanged:** the `ControllerAgreement` creation tool
   (item 5 below) gates authenticated deploy to any substrate the operator
   does not already own, so it becomes load-bearing for a multi-substrate
   supervisor rather than merely outstanding.

The federated-query orchestrator (M5 item 2's *other* half, DataFusion /
Substrait) is untouched by this amendment and stays deferred to the final
phase per the 2026-07-16 resequencing.

---

## Committed Work: Logical Service Discovery Overlay (2026-08-02)

**Design of record:** [ADR-0022](../decisions/0022-two-tier-logical-service-discovery.md).
**Promoted to a milestone directory on 2026-08-04:
[M05C-logical-discovery-overlay](./milestones/M05C-logical-discovery-overlay/task.md)**,
which carries the `task.md`, the slice plans, and the `§0` findings passes. This
section keeps the build-order reasoning below — that is meta-plan work — and the
milestone doc carries everything else. Two findings from that planning pass
correct text on this page and are noted in place: the record cannot be
`EndpointInfo` "unchanged" *and* carry a generation, and "generation-fenced" in
the slice table below overstates what the generation does.

This is the build-out of the 2026-07-16 resequencing's item 4 spike — "the
orchestrator's shard-discovery / data-routing design question" — whose design
half is now discharged. Recorded here, not in
[deferred-backlog.md](./deferred-backlog.md), for the reason the *Live
App-Context Registry* interstitial above gives for itself: this is committed
platform work, not droppable debt. Every slice below carries an explicit
pickup trigger so none of it is remembered by accident.

**Two rows moved here out of `deferred-backlog.md` §5, where they were
tracked in isolation** — same move, same reasoning, as the
`expected_asserter_did` row the Live App-Context Registry interstitial
absorbed:

- *Cross-app `Bind` dependency naming has no manifest surface* (was targeted
  "A5 / first real cross-app dependency") — now **S4** below. ADR-0022 §5
  settles how it is authorized, which was the missing half.
- *`TopologyMode::Sharded` has no expressible sharding strategy in a manifest*
  (was `TBD`) — now **S1** below. ADR-0022's consequences note that
  `Sharded` needs four things at once and is unusable with any one missing;
  the manifest surface is the first.

**Whose milestone this is.** S0-S4 are **Milestone 5 item 2** work, by the
same lineage as [M05A](./milestones/M05A-app-supervisor/task.md) itself: item 2
carried the "Design TBD" flag this discharges, and M05A was split out of it on
2026-07-27. Item 2 therefore has three halves — the supervisor (M05A), this
discovery overlay (S0-S4), and the federated-query orchestrator
(DataFusion/Substrait), which this does not touch.

**They are item 2 work that is deliberately *not* deferred with the rest of
item 2.** The 2026-07-16 resequencing defers items 2-4 to the final phase
alongside M7. S1-S4 must not inherit that position: M7's `[PLT-RED]` depends
on them, so filing them next to M7 puts them behind their own consumer. They
belong to item 2 by subject matter and to the resequencing's item-4 spike
stream by schedule.

**Position in the build order.** Only **S5** is M7 work. M7's `[PLT-RED]` is
*state* replication — SQLite WAL, pub/sub log, blob — and sits **downstream**
of this overlay, not upstream: replicating a service across three members is
pointless while callers cannot discover the current member set.

**S1–S4 sit between M05A and M7, in the 2026-07-16 resequencing's item-4
stream** — the second, decoupled work stream that amendment already accepts.
They are **not** queued behind M5 item 1 (async primitives) or M6 (the
product/chat milestone) and may run concurrently with either. Their only real
gate is M05A slice A7; nothing in S1–S4 needs FDAE work, the async primitives,
or the chat app.

Read the per-slice triggers below as a *dependency chain within that stream*,
not as a position in the global order — S1 through S3 are strictly sequential
because each consumes what the previous one publishes, while S4 additionally
waits on a real consumer appearing.

**Against M5's own remaining halves, which sit in two different places:** M5
item 1 (async primitives) is front-loaded ahead of M6 and gates M05A slice A6;
M5 items 2-4 are deferred to the final phase with M7. S1-S4 have no dependency
in either direction with any of them. If only one stream is available, do M5
item 1 first — it unblocks A6 and M6, where the overlay unblocks nothing yet.

**One cross-milestone coupling to know in advance: S3 changes the client
gateway hostname format, and M6 builds a web/desktop shell against it.** Not a
reason to reorder either one. The format is centralised in
`core::util` (build) and `core::protocol_utils` (parse), so as long as M6's
client goes through those helpers rather than formatting host strings itself,
S3 changes one place. Recorded here because the cost of discovering it late is
paid in M6's code, not in this slice's.

So the full picture, start to finish:

```
M05A (slice A7 = S0)  →  S1 → S2 → S3   ─┐
                              └→ S4      ─┼→  M7 (+ S5)
   M5 item 1 → M6 (product path)         ─┘
```

**Slices and pickup triggers:**

| # | Scope | Pickup trigger |
|---|---|---|
| **S0** | App-instance master DID: minted at `adopt`, held in the supervisor vault, surfaced on `status`, exportable/importable for handover. Identity and custody only — no registry publication. | **Landed as M05A slice A7, Complete 2026-08-04.** `docs/planning/milestones/M05A-app-supervisor/status.md`'s A7 section carries the evidence. |
| **S1** | Tier 1: the app-DID registry record, published and refreshed by the supervisor, carrying a `generation` a **reader** compares. Manifest surface for `ShardingStrategy` (absorbed backlog row). *(Corrected 2026-08-04: this row said "generation-fenced". ADR-0022 §2 is careful and this was not — admission stays last-writer-wins; the generation makes the split-brain **visible**, not impossible. Prevention would need registry compare-and-set, which [ADR-0023](../decisions/0023-durable-async-primitives.md) §6 declines to build. Also: §2 says the record reuses `EndpointInfo` "unchanged" and that it carries a generation; `EndpointInfo` has no such field, so S1 adds one — see [M05C plan](./milestones/M05C-logical-discovery-overlay/implementation-plan.md) §0.1.)* | M05A slice A7 (S0) Complete — the design gate is **cleared**. **Sequenced after M05B (decided 2026-08-04):** the two streams are design-independent but edit the same four files, so M05B runs to completion first — see [M05C plan](./milestones/M05C-logical-discovery-overlay/implementation-plan.md) §2. S1 inherits an ordering constraint from A7, not just a key: `import-master` must run before `adopt` on a handover, or `adopt` mints a *second* app identity under the same name that the generation fence cannot catch (two DIDs, not one record two writers contend over). A7 documents the order and makes `adopt` self-correcting; S1 is where publishing under the wrong identity first has an external consequence, so whoever picks this slice up should read A7's plan §0.5 before writing the publisher. |
| **S2** | Tier 2: the signed topology document, the supervisor `resolve` RPC, and the client-side verify/cache path feeding `LogicalResolver::register`. Ships the `epoch` field unenforced. | S1 Complete. |
| **S3** | Gateway hostname scheme (`-a…-s…-i…`) plus the routing-key request header; coordinator relay of the document in the WebRTC bootstrap page. | S2 Complete. |
| **S4** | Cross-app `Bind`: manifest surface, UCAN-scoped per-service exposure declared in the submitted plan, and replacing `prepare_binding`'s intra-app refusal with an authorization check (absorbed backlog row). | S2 Complete **and** a first real cross-app dependency exists. |
| **S5** | Shard rebalancing, and enforcing the epoch fence on the data path. | **M7** `[PLT-RED]` — nothing to rebalance until redundancy/sharding is actually deployed. S2 must already have shipped the field. |

**Why S2's epoch field ships before S5 enforces it:** adding a field to a wire
format is free before anything depends on that format and expensive
afterwards. Stated here because the two slices are separated by a milestone
and the reason would otherwise be lost.

---

## Milestone 5: Async Lifecycle and Developer Experience

> **Build order:** see the *M5–M7 Resequencing* amendment above — item 1
> (async primitives) is front-loaded ahead of M6; items 2–4 (orchestrator,
> versioning, dev tooling) are deferred to the final phase, with the
> orchestrator's shard-discovery/routing design pulled forward as an early
> parallel spike.

**Goal:** Handle background jobs, offline semantics, and continuous reconciliation, while finalizing developer toolchains.

**Feature Grouping:**
- `[PLT-ASY]` Asynchronous Operations
- `[LFC-MGT]` Active Control-Plane Mode
- `[PLT-DAP]` Federated Query Orchestrator
- `[LFC-VER]` Versioning Support (State snapshot/rollback)
- `[ADV-DEV]` SynApp Developer Tooling
- `[FND-IAM]` `ControllerAgreement` creation tool (closes the M04A B7b gate)

**Implementation Approach:**
1. **Async Primitives:** Implement the Outbox queue, cron lease mechanisms, Dead Letter Queue (DLQ), long-running task restart rules, and compensating transactions (sagas). **Split out 2026-08-04 into its own milestone directory, [M05B-async-primitives](./milestones/M05B-async-primitives/task.md)** — the same treatment item 2 got, and for the same reason: this is five distinct mechanisms, not one, and it now has a design of record ([ADR-0023](../decisions/0023-durable-async-primitives.md)). **Four of the five are built there (slices B1–B4); long-running-task restart rules are deferred** to this milestone's final phase alongside items 2–4 — the only mechanism with no consumer and no near-term consumer, and the one needing genuinely new machinery, since `dispatch_epoch_timeout_secs` bounds every guest entry point at 5 seconds. Tracked in [deferred-backlog.md](./deferred-backlog.md) §8. Milestone numbering and labels are unchanged; M5 itself still has no directory.

   **Build order within M5's front-loaded halves (decided 2026-08-04):** M05B first, then [M05C](./milestones/M05C-logical-discovery-overlay/task.md) (the discovery overlay). The two are design-independent — the overlay section below says so and is right — but they edit the same four files, so sequencing removes the collision instead of managing it. It also matches the priority that section already states on merit.
   > **Landing this fires a pickup trigger.** The App Supervisor ships its
   > binding propagation with best-effort synchronous delivery behind a narrow
   > "apply this action to that substrate" trait, deliberately, because durable
   > delivery needs the Outbox/DLQ built here
   > ([ADR-0021](../decisions/0021-binding-propagation-and-app-supervisor.md) §5).
   > When this item completes, swap that trait's implementation — tracked in
   > [deferred-backlog.md](./deferred-backlog.md) §8 *Node lifecycle & ops*.
   > Recorded here rather than only in the supervisor's own milestone doc, so
   > the trigger is visible from the side that fires it.
   >
   > **Amended 2026-08-04 — "and add the single-writer cron lease" is
   > withdrawn.** [ADR-0023](../decisions/0023-durable-async-primitives.md) §6
   > finds there is nothing for such a lease to arbitrate in this topology:
   > registry writes are partitioned by key ownership, so distinct principals
   > never contend, and inside an app instance the supervisor is already the
   > single writer at an operator-minted generation
   > ([ADR-0021](../decisions/0021-binding-propagation-and-app-supervisor.md) §4).
   > What the specification calls a lease reduces to target selection plus a
   > local overlap guard. Supervisor HA is unaffected by this and stays with M7,
   > where replicated supervisor state lives.
2. **App Supervisor & Query Orchestrator:** Continuous reconciliation of desired state across substrates — **split out and mostly resequenced ahead of item 1**; see the *App Supervisor Split* amendment above and [M05A-app-supervisor](./milestones/M05A-app-supervisor/task.md). What remains here is the half that genuinely needs item 1's primitives (durable push delivery, retry against offline substrates, DLQ, single-writer lease). Separately, introduce foundational DataFusion logical planning and Substrait serialization for federated queries. This includes:
   - Defining the DataFusion `TableProvider` interface for Syneroym Data Services.
   - Defining the plan-fragment serialization contract (Substrait schema version pinning).
   - Defining the network protocol for distributing plan fragments to edge nodes.
   - Defining what "done" looks like (e.g., a working end-to-end query across 2 nodes in a test).
   - ~~*(Design TBD to resolve before M5: How the Orchestrator discovers which node holds which shard, and how data routing tables are maintained for `[PLT-DAP-01]`)*~~ **Discharged 2026-08-02** by [ADR-0022](../decisions/0022-two-tier-logical-service-discovery.md), the same way item 5's registry-trust-model ADR is discharged by ADR-0020 §6. The build-out is the [Logical Service Discovery Overlay](#committed-work-logical-service-discovery-overlay-2026-08-02) item below — **item 2 work that does not defer to the final phase with the rest of item 2**, because M7 depends on it. So item 2 now has three halves: the supervisor (M05A, split out 2026-07-27), the discovery overlay (not deferred), and the federated-query orchestrator below (deferred, unchanged).
3. **Versioning:** Implement pre-upgrade SQLite snapshotting and automatic rollback mechanisms.
4. **Developer Tools:** Release the mock SDK, project templates, the zero-drift `roymctl dev` local environment, and remote package retrieval over HTTP/OCI for the `ManifestCatalog`.
5. **`ControllerAgreement` Creation Tool:** Build the `roymctl` tool to create/sign a `ControllerAgreement`, spun out of M04A Slice B7 (`docs/planning/milestones/M04A-proxy-and-auth-foundation/plans/B7.md` §6; task.md's post-B7b item list). Until this exists, B7b's ownership/deploy capability gate is inert — every substrate remains unowned, so this closes that gap. **Amended 2026-07-27:** the tool itself is **pulled forward into [M05A](./milestones/M05A-app-supervisor/task.md) as Slice P0**, because item 5 has no position in the 2026-07-16 resequencing (which front-loads item 1 and defers items 2-4) and M05A's multi-substrate placement cannot ship responsibly while ownership is unestablishable. Of the three items bundled with it: the **registry-trust-model ADR is discharged** by [ADR-0020](../decisions/0020-stable-logical-service-identity.md) §6 plus M05A slice A1 — the same change to `verify()`'s contract, with B7's "needs a real consumer" gate met; **multiple-substrate-owners representation (F12)** and **Tier 1 for the five data native-capability interfaces (F3)** stay here, neither being needed for single-owner multi-substrate placement.

> **Note:** Decentralized Pub/Sub completion (`[PLT-DAP-04]`, adapting the
> M3B in-process `rumqttd` broker to synchronise its topic log with peer
> nodes over Iroh QUIC) was previously planned here. It moved to **Milestone
> 7**, alongside SQLite WAL replication and blob replication, because all
> three are the same underlying problem — pull-based log/state
> synchronisation to peer nodes over QUIC, purely for redundancy/failover of
> the broker's own state if its hosting node is lost. This is exactly
> parallel to WAL replication and is **not** a prerequisite for cross-node
> pub/sub to function: a `publish`/`subscribe` call from a different
> physical node is routed to whichever node hosts the target service via
> the same RPC/native-dispatch path used for any cross-node host-function
> call (e.g. `data-layer`), available well before M7. See
> [ADR-0010 Amendment 2](../decisions/0010-mqtt-broker-rumqttd.md).

---

## Milestone 6: Initial Integrated Experience

> **Build order:** see the *M5–M7 Resequencing* amendment above — M6 is
> pulled forward to directly after M5's async-primitives slice (it depends
> only on those, plus M4 FDAE and the M3B broker), so it validates the
> foundation early rather than landing last.

> **Planning-doc split (2026-08-13):** M6 is planned and implemented as three
> sub-milestones, mirroring the M3A/M3B/M3C and M05A/M05B/M05C precedent. The
> split came out of writing
> [roym-integrated-experience-spec.md](../roym-integrated-experience-spec.md)
> and reading it against the tree: M6 was first split two ways (foundation /
> product), and scoping the foundation half found two unrelated clusters inside
> it.
>
> - **M06A** — [App Platform Surface](./milestones/M06A-app-platform-surface/task.md).
>   What any SynApp needs to be a web app at all: static assets served from
>   blobs without instantiating the sandbox, and a route target that puts guest
>   logic in an inbound HTTP request. Proven by building a WASM equivalent of
>   `miniapp-demo1-web` and running the existing browser suite against it.
>   Slices **A1–A5**, all Complete (2026-08-17). Nothing in it is
>   Roym-specific.
> - **M06B** — [Roym Substrate Foundations](./milestones/M06B-roym-substrate-foundations/task.md).
>   The dual-build shim, person identity at the client gateway, the durable
>   messaging host interface and Layer 3 delivery, outbox delivery state, and
>   service-record visibility. These are gaps **G1–G4** in the experience spec.
>   Slices **B1–B5**, `task.md` written 2026-08-18. **B1–B3 Complete
>   (2026-08-20)**; B4–B5 not started.
> - **M06C** — the Roym product itself, following the spec's four releases (a
>   usable local guild, the transaction vertical, cross-installation trust,
>   private group chat). Directory not yet created.
>
> **Why this order.** M06A removes the only non-WASM piece of Roym — the spec's
> Web entrypoint service, which exists purely because a component cannot serve
> HTTP and is the sole part of Roym exempt from its one-source/two-builds rule.
> Closing that gap against a throwaway fixture is cheaper than discovering it
> under product code, and it gives a foundation milestone a runnable exit gate
> rather than "the interfaces exist".
>
> **External prerequisite.** M06C additionally needs
> [M05C](./milestones/M05C-logical-discovery-overlay/task.md) **S4's visibility
> half** (ADR-0022 §5's per-logical-service "open to all" declaration).
> Without it a client resolving an app on an unaffiliated node is refused
> unless an operator pre-installed a token, which blocks both directory search
> and every cross-installation flow. S4's *cross-app `Bind`* half stays parked
> on its own gate — Roym does not supply the consumer it waits for. See
> [deferred-backlog.md](./deferred-backlog.md) §3.

**Goal:** Deliver the first cohesive product experience using the completed foundations, proving the value of the reference application.

**Feature Grouping:**
- Thin Syneroym Hub (Desktop/Web surface only)
- Professional Services Guild Application
- Chat SynApp (`[PLT-DAT]`/`[PLT-ASY]`/`[FND-IAM]` per the requirements-spec Substrate Feature Coverage Matrix)
- Guild Directory service — category/area/filter search over listings members published to it. **Added 2026-08-13**: the grouping listed no discovery at all, though the milestone's product story cannot work without it. This is deliberately *not* `[P2P-DSC]`, which stays in M8: one directory service answering queries, with no shard placement, no signed Publications, and no rendezvous hashing. Cross-installation search is still demonstrated, because the consumer, the directory, and the provider can each be on a different installation. See [roym-integrated-experience-spec.md](../roym-integrated-experience-spec.md) D1.

**Implementation Approach:**
1. **Headless Native Shell:** Build the thin desktop/web UI shell that renders JSON Action Cards.
2. **Product Polish:** Finalize the `SynSvcs` necessary for the Professional Guild to operate end-to-end.
3. **Chat SynApp:** Implement the default Layer 4 chat wrapper described in [ADR-0013](../decisions/0013-p2p-messaging-architecture.md) — Actor/Infrastructure identity delegation, 1-to-1 delivery via X3DH + Double Ratchet (`libsignal-protocol-rust`), group chat over the Gossip DAG, and deterministic ordering by **raw sender timestamps**, sorted `(sender_timestamp, sender_did)`. Builds on the M3B pub/sub broker, M4 FDAE access control, and M5 outbox/DLQ primitives, all of which are already in place by this milestone.

   > **Corrected 2026-08-13.** This item previously said "relative-clock deterministic ordering"; [ADR-0013](../decisions/0013-p2p-messaging-architecture.md) §5 explicitly rejects relative-clock negotiation and chooses raw sender timestamps, so the old wording named a design the ADR had already ruled out. Two further changes, from [roym-integrated-experience-spec.md](../roym-integrated-experience-spec.md): **Primary Substrate multi-device sync moves out of the core set** (a separate system that no other core goal depends on), and **group encryption is no longer MLS** — [ADR-0013 Amendment 1](../decisions/0013-p2p-messaging-architecture.md) replaces it with an owner-distributed per-epoch key, since MLS assumes a Delivery Service supplying a consistent Commit order that a gossip DAG does not provide. The Gossip DAG, ordering, and history sync are unchanged by that swap.
4. **Exclusions:** Native ledger/mutual credit and integrated escrow are explicitly excluded at this stage. AI participants in group chats (`[APP-AGI]`) are also excluded here — they depend on the Milestone 9A inference/tooling foundations and are sequenced as a Milestone 9 follow-on once Chat itself exists. **Added 2026-08-13:** attachments, multi-device sync, and chat polish (threads, polls, reminders, read receipts, typing indicators, broadcasts) are also out — each is its own system, and none is a prerequisite for the milestone's product story.

> **Prerequisite substrate gaps.** M6 cannot start on the Chat SynApp until three
> things exist that do not today: a durable messaging host interface (`syneroym:messaging`
> is pub/sub only, and [ADR-0013](../decisions/0013-p2p-messaging-architecture.md) §6
> forbids building durable chat on it), a guest-reachable outbox (`crates/async_queue`
> has no WIT surface), and person-level identity at the client gateway (it authenticates
> no client and presents the node's DID). All three are rows in
> [deferred-backlog.md](./deferred-backlog.md); see
> [roym-integrated-experience-spec.md](../roym-integrated-experience-spec.md) G1-G3.
>
> **Scheduled 2026-08-18.** All three are now
> [M06B](./milestones/M06B-roym-substrate-foundations/task.md) slices — the
> messaging interface and 1:1 delivery as **B4**, the group DAG as **B5**, the
> outbox folded into B4's interface rather than given its own, and gateway person
> identity as **B1**. M06B adds two more the note above did not name: the D2/D3
> dual-build shim (**B3**) and declared service visibility (**B2**).

---

## Milestone 7: Resilience and Operability

> **Build order:** see the *M5–M7 Resequencing* amendment above — item 1
> (the SQLite-WAL-replication feasibility prototype) is pulled forward as an
> early parallel spike; the remaining items run in the final phase alongside
> the deferred M5 orchestrator/versioning work.

**Goal:** Harden the system for production by adding high-availability replication (database, pub/sub, and blob), redundancy, and deep observability.

**Feature Grouping:**
- `[PLT-RED]` Service Redundancy (Declarative Replication Topology) — database, pub/sub log, and blob replication
- `[PLT-DAP-04]` Decentralized Pub/Sub over Iroh QUIC (completes the M3B in-process broker)
- `[FND-SEC]` Encrypted Backups, Attestation & Supply-chain signing
- `[ADV-OBS]` Advanced Observability

**Implementation Approach:**
1. **SQLite Replication Feasibility:** Validate the SQLite-safe replication mechanism through a bounded prototype with correctness, crash-recovery, and performance exit criteria.
2. **Declarative Replication:** Implement live, reliable SQLite WAL replication across Substrate nodes based on the validated prototype, controlled by the `DeploymentPlan` (Primary, Read-Replica, Cold Backup).
   - *(Design TBD to resolve before M7: Define the distributed replication consistency model and failover behavior).*
3. **Decentralized Pub/Sub Completion** (deferred from M3B, [ADR-0010](../decisions/0010-mqtt-broker-rumqttd.md)): Adapt the in-process `rumqttd` broker to synchronise its topic log with peer nodes via pull-based log replication over Iroh QUIC streams (rather than raw TCP), fulfilling the `[PLT-DAP-04]` overlay requirement without changing the `syneroym:messaging` WIT surface shipped in M3B. Shares its replication primitive (ordered, checksummed frame streaming over an Iroh multiplexed stream) with item 2 above — the payload differs (SQLite WAL frames vs. MQTT topic-log entries) but the transport and pull/ack model do not.
4. **Blob Replication:** For deployments without an S3-compatible backend, implement peer-to-peer blob replication across Substrate nodes (the "peer backup substrate" case), reusing the same declarative `DeploymentPlan` topology and Iroh QUIC transport as items 2 and 3. Content-addressing (SHA-256) makes this simpler than WAL/log replication — no ordering or frame-sequence invariants to preserve, just "does a valid copy of hash `H` exist on N nodes." Deployments that do configure an S3-compatible backend continue to rely on the provider's own redundancy (unchanged from the original `[PLT-RED]` decision).
5. **HA Upgrade:** Upgrade the M5 active controller's database to rely on replicated HA storage.
6. **Topology Control:** Implement Registry topology epochs, manual promotion workflows, and strict bidirectional quarantine fencing.
7. **Security Hardening:** Add Attestation API and verification flows, binary signature verification, and support for scheduled, encrypted remote backups (with tested restore paths).
8. **Metrics Pipeline:** Finalize data rollups and expose metrics via secured RPCs.

---

## Milestone 8: Federation and Trust
**Goal:** Expand the single-node capability into a robust, community-driven mesh network.

**Feature Grouping:**
- `[P2P-DSC]` Distributed Matching Fabric (discovery)
- `[P2P-REP]` Peer Reputation & Trust

**Implementation Approach:**
1. **Matching Fabric, flat slice:** Implement the signed Publication data model, one or two protocol routing dimensions (spatial + category), and deterministic placement via rendezvous hashing onto a small set of leaf index shards (the existing aggregator/super-peer nodes). Client-side verification — signature, timestamp, expiry — ships from day one. The hierarchical synopsis tree, composite routing descriptors, and cross-shard ranking are additive follow-on work once leaf-shard count makes flat fan-out expensive; none of them require reworking the Publication or placement contract shipped here.
2. **Reputation CRDT:** Implement `[P2P-REP]` only if M0 selects and schedules a mutually signed interaction-receipt mechanism (DLN); otherwise sequence `[P2P-REP]` for a later milestone without weakening its cryptographic prerequisite.

---

## Milestone 9: Expansion Track - AI Concierge

Due to its complexity, the AI expansion track is subdivided into foundational and advanced stages.

### Milestone 9A: Agentic Foundations
**Goal:** Connect the node to local AI capabilities and manage tools.
**Implementation Approach:**
1. **Hardware Gating:** Implement hardware detection and strict model allow-lists.
2. **Inference Execution:** Integrate local LLM inference wrappers and implement remote inference fallback logic.
3. **Tool Directory:** Implement `sqlite-vec` indexing for the Ecosystem Directory and basic tool retrieval loops.

### Milestone 9B: Advanced Orchestration
**Goal:** Execute complex, autonomous workflows safely.
**Implementation Approach:**
1. **Concierge App:** Deploy the core Concierge `SynSvc`, integrating strict HITL (Human-in-the-Loop) consent, pause, and resume workflows for agentic execution.
2. **Loopcraft:** Deploy specialized sub-agents and verification loops via WASM to orchestrate complex reasoning.
3. **Observability:** Implement streaming progress telemetry back to the Hub UI.
4. **External Integration:** Implement the MCP headless gateway and support for agent-to-agent delegation.

---

## Milestone 10: Expansion Track - Mobile Edge
**Goal:** Bring full native substrate execution to mobile devices.

**Feature Grouping:**
- `[EDG-MOB]` Mobile Operation

**Implementation Approach:**
1. **Toolchain & Targets:** Establish cross-compilation for WASM runtime and substrate against iOS/Android targets.
2. **Mobile Lifecycle:** Integrate robustly with OS-level background task windows and push notification wakes to defer network responses.
3. **Secure Hardware:** Implement Android StrongBox and iOS Secure Enclave bindings for the WIT security interfaces.

---

## Later-Phase Additions
> The full, cross-cutting deferred backlog (these items plus ADR deferrals, per-slice scope-outs, and in-code TODOs) is consolidated in [deferred-backlog.md](./deferred-backlog.md). The list below is the implementation-sequencing subset.

To keep Milestones 1–10 achievable, the following features are sequenced after them, not shelved. Each is designed to compose with what ships in Milestones 1–10 without reworking it:
- **`[FND-IDT]` Extensions:** Master Key export/recovery, Tier-1 Fallback processing, and Method B Zero-Knowledge (ZK) plugin verification.
- **Phase 6 Product Expansion:** 
  - The Producer-Distributor Mesh application vertical.
  - Complete, rich Syneroym Hub UI surfaces.
  - Dedicated marketplace, aggregator, and facilitator `SynSvcs`.
- **Financial & Escrow Services:** Native settlement, mutual credit ledger operations, and integrated transaction escrow, layered onto the pluggable Payment Abstraction Layer once the Dynamic Ledger Network is scoped.
- **MQTT Shared Subscriptions:** `$share/<group>/<filter>` competing-consumer delivery for fleets of external `SyneroymClient` workers. `rumqttd 0.20` already supports this; the work is a small fix to `namespace_topic` in `crates/mqtt_broker`. Deferred post-M3B as it doesn't block any subsequent milestone.

---

## Next Steps
Once you are ready to begin execution, we will trigger **Milestone 0: Contract and Decision Gate** to construct the traceability matrix and resolve the blocking ADRs.
