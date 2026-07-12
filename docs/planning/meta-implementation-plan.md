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
**Goal:** Bridge isolated services securely by introducing the Universal Proxy and layering the FDAE (Access Control) on top of the established Data Layer.

**Feature Grouping:**
- Universal Proxy / wRPC
- `[PLT-DAP-05]` Data Pipeline Streams (`syneroym:data/stream`)
- `[LFC-VER]` Protocol Negotiation
- `[FND-IAM]` Access Control (FDAE, UCAN context, RLS/CLS)

**Implementation Approach:**
1. **Universal Proxy & Streams:** Implement wRPC over Iroh QUIC for typed calls, and the `syneroym:data/stream` WIT interface for backpressured data pipelines.
2. **UCAN Context:** Extract and normalize UCAN scopes/claims upon request ingress.
3. **Local FDAE:** Implement the SQL Pushdown Sieve, compiling declarative policies into the SQLite engine (handling data-centric RLS/CLS).
4. **Federated FDAE:** Expand the pipeline to support cross-service parameter fetching via the Universal Proxy.
5. **`AggregationPipeline`** (gate item deferred from M3A, [ADR-0007](../decisions/0007-data-layer-wit-interface.md)): add `$group`/`$having`/projections to the data-layer `query` WIT surface, translating MongoDB aggregation-pipeline stages onto SQLite constructs (`GROUP BY`, `HAVING`, views) rather than inventing parallel syntax.
6. **Privileged raw-SQL escape hatch** (gate item deferred from M3A, [ADR-0011](../decisions/0011-privileged-raw-sql-query.md)): add the `query-raw` host function for trusted contexts needing SQL expressivity beyond the JSON filter DSL, gated by the Admin UCAN capability introduced in this milestone (replacing the M3 `is_init_context` scaffold).
7. **Close the M3B/M3C native-dispatch authentication gap (gate item, not optional hardening):** `RouteHandler::handle_stream` (`crates/router/src/route_handler/io.rs`) only runs `HandshakeVerifier::verify_preamble` — the sole point that checks caller identity against `preamble.service_id` — when `preamble.delegation` is present, and the native-capability interfaces added across M3A/M3B/M3C (`data-layer`, `vault`, `app-config`, `blob-store`, `messaging`, `http-native`) never require one. Concretely: any peer that can open an Iroh connection to a node (the QUIC listener binds `0.0.0.0` by default — this is *not* bounded by `client_gateway`'s `127.0.0.1`-only convenience proxy) and knows a target service's DID can act "as" that service on every native-capability interface and the M3C HTTP bridge, with no cryptographic proof of being it — `data-layer` writes, `messaging::publish`, SSE eavesdrop, blob access. Recorded as an explicit, tracked interim posture at M3C close (see `docs/planning/milestones/M03B-messaging/status.md`, "Interim HTTP-write security posture," and [ADR-0010](../decisions/0010-mqtt-broker-rumqttd.md) Finding B9) rather than silently absorbed — M4's UCAN/FDAE work must wire delegation/capability verification into this exact dispatch path (native-dispatch **and** the HTTP-bridge routes that share it) before M4 can be considered closed, not just into new call sites this milestone adds.

---

## Milestone 5: Async Lifecycle and Developer Experience
**Goal:** Handle background jobs, offline semantics, and continuous reconciliation, while finalizing developer toolchains.

**Feature Grouping:**
- `[PLT-ASY]` Asynchronous Operations
- `[LFC-MGT]` Active Control-Plane Mode
- `[PLT-DAP]` Federated Query Orchestrator
- `[LFC-VER]` Versioning Support (State snapshot/rollback)
- `[ADV-DEV]` SynApp Developer Tooling

**Implementation Approach:**
1. **Async Primitives:** Implement the Outbox queue, cron lease mechanisms, Dead Letter Queue (DLQ), long-running task restart rules, and compensating transactions (sagas).
2. **Active Controller & Query Orchestrator:** Deploy the controller `SynApp` that continuously reconciles desired state. Introduce foundational DataFusion logical planning and Substrait serialization for federated queries. This includes:
   - Defining the DataFusion `TableProvider` interface for Syneroym Data Services.
   - Defining the plan-fragment serialization contract (Substrait schema version pinning).
   - Defining the network protocol for distributing plan fragments to edge nodes.
   - Defining what "done" looks like (e.g., a working end-to-end query across 2 nodes in a test).
   - *(Design TBD to resolve before M5: How the Orchestrator discovers which node holds which shard, and how data routing tables are maintained for `[PLT-DAP-01]`)*
3. **Versioning:** Implement pre-upgrade SQLite snapshotting and automatic rollback mechanisms.
4. **Developer Tools:** Release the mock SDK, project templates, the zero-drift `roymctl dev` local environment, and remote package retrieval over HTTP/OCI for the `ManifestCatalog`.

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
**Goal:** Deliver the first cohesive product experience using the completed foundations, proving the value of the reference application.

**Feature Grouping:**
- Thin Syneroym Hub (Desktop/Web surface only)
- Professional Services Guild Application
- Chat SynApp (`[PLT-DAT]`/`[PLT-ASY]`/`[FND-FDA]` per the requirements-spec Substrate Feature Coverage Matrix)

**Implementation Approach:**
1. **Headless Native Shell:** Build the thin desktop/web UI shell that renders JSON Action Cards.
2. **Product Polish:** Finalize the `SynSvcs` necessary for the Professional Guild to operate end-to-end.
3. **Chat SynApp:** Implement the default Layer 4 chat wrapper described in [ADR-0013](../decisions/0013-p2p-messaging-architecture.md) — Actor/Infrastructure identity delegation, Primary Substrate multi-device sync, 1-to-1 delivery via X3DH + Double Ratchet (`libsignal-protocol-rust`), group chat via the Gossip DAG + MLS (`openmls`), and relative-clock deterministic ordering. Builds on the M3B pub/sub broker, M4 FDAE access control, and M5 outbox/DLQ primitives, all of which are already in place by this milestone.
4. **Exclusions:** Native ledger/mutual credit and integrated escrow are explicitly excluded at this stage. AI participants in group chats (`[APP-AGI]`) are also excluded here — they depend on the Milestone 9A inference/tooling foundations and are sequenced as a Milestone 9 follow-on once Chat itself exists.

---

## Milestone 7: Resilience and Operability
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
